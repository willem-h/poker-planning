//! The browser-facing session: iroh endpoint, gossip subscription, and
//! the `wasm_bindgen` surface the page drives.

use std::{cell::RefCell, rc::Rc};

use iroh::{
    address_lookup::memory::MemoryLookup, endpoint::presets, Endpoint, EndpointId, RelayMode,
    RelayUrl,
};
use iroh_gossip::{
    api::{Event, GossipSender},
    net::{Gossip, GOSSIP_ALPN},
    proto::TopicId,
};
use n0_future::{task, time::Duration, StreamExt};
use wasm_bindgen::prelude::*;

use crate::{
    protocol::{commit, open, seal, Room, HEARTBEAT_MS},
    ticket::Ticket,
};

/// Installs a panic hook that reports to the browser console.
///
/// Without this a panic inside the wasm module surfaces as an opaque
/// "unreachable executed" trap.
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("poker-wasm panic: {info}").into());
    }));
}

/// The deck, handed to JS so the card values live in one place.
#[wasm_bindgen(js_name = deck)]
pub fn deck() -> Vec<String> {
    crate::protocol::DECK.iter().map(|v| v.to_string()).collect()
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn random_nonce() -> String {
    let bytes: [u8; 8] = rand::random();
    data_encoding::HEXLOWER.encode(&bytes)
}

fn js_err(context: &str, err: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&format!("{context}: {err}")).into()
}

/// A live seat in a room. Dropping it tears the connection down.
#[wasm_bindgen]
pub struct Session {
    endpoint: Endpoint,
    // The router owns the accept loop; keeping it alive keeps us reachable.
    router: iroh::protocol::Router,
    gossip_tx: GossipSender,
    secret: iroh::SecretKey,
    room: Rc<RefCell<Room>>,
    topic: TopicId,
    bootstrap: Vec<iroh::EndpointAddr>,
    on_state: js_sys::Function,
}

#[wasm_bindgen]
impl Session {
    /// Opens a room, or joins the one an invite points at.
    ///
    /// `on_state` is called with the merged room view whenever it changes, and
    /// `on_status` with a connectivity string for the badge.
    ///
    /// `relay` overrides the relay servers to use. Leave it unset for the ones
    /// n0 runs; set it to self-host, or to keep a room inside a network that
    /// does not let the public relays through.
    pub async fn join(
        invite: Option<String>,
        name: String,
        relay: Option<String>,
        on_state: js_sys::Function,
        on_status: js_sys::Function,
    ) -> Result<Session, JsValue> {
        let status = Status(on_status);
        status.set("connecting", "Starting endpoint");

        let (topic, bootstrap) = match invite.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(invite) => {
                let ticket = Ticket::decode(invite).map_err(|e| js_err("invite", e))?;
                (ticket.topic, ticket.bootstrap)
            }
            None => (TopicId::from_bytes(rand::random()), Vec::new()),
        };

        // The invite carries full addresses, so peers can be dialed without
        // waiting on a lookup service to resolve them.
        let lookup = MemoryLookup::new();
        for addr in &bootstrap {
            lookup.add_endpoint_info(addr.clone());
        }

        let builder = match relay.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            // A room on its own relay is usually on a network that cannot reach
            // n0's services at all, so skip their address lookup too: invites
            // carry full addresses, which is all we need to dial.
            Some(relay) => {
                let url: RelayUrl = relay.parse().map_err(|e| js_err("relay url", e))?;
                Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Custom(url.into()))
            }
            None => Endpoint::builder(presets::N0),
        };
        let endpoint = builder
            .address_lookup(lookup)
            .bind()
            .await
            .map_err(|e| js_err("could not start iroh endpoint", e))?;

        let secret = endpoint.secret_key().clone();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(GOSSIP_ALPN, gossip.clone())
            .spawn();

        // In a browser all traffic goes through a relay, so there is nothing to
        // share until we have picked one.
        status.set("connecting", "Finding a relay");
        endpoint.online().await;

        status.set("connecting", "Joining room");
        let peer_ids = bootstrap.iter().map(|addr| addr.id).collect();
        let (gossip_tx, mut gossip_rx) = gossip
            .subscribe(topic, peer_ids)
            .await
            .map_err(|e| js_err("could not subscribe to the room", e))?
            .split();

        let room = Rc::new(RefCell::new(Room::new(endpoint.id(), name, now_ms())));

        let session = Session {
            endpoint,
            router,
            gossip_tx: gossip_tx.clone(),
            secret: secret.clone(),
            room: room.clone(),
            topic,
            bootstrap,
            on_state: on_state.clone(),
        };
        session.emit_state();

        // Fold incoming announcements into our copy of the room.
        {
            let room = room.clone();
            let on_state = on_state.clone();
            let gossip_tx = gossip_tx.clone();
            let secret = secret.clone();
            let status = status.clone();
            task::spawn(async move {
                let mut joined = false;
                while let Some(event) = gossip_rx.next().await {
                    let event = match event {
                        Ok(event) => event,
                        Err(err) => {
                            status.set("error", &format!("Room closed: {err}"));
                            break;
                        }
                    };
                    match event {
                        Event::Received(msg) => {
                            // `msg.delivered_from` is whoever relayed this to
                            // us, so the author comes from the signature.
                            let Ok((author, announce)) = open(&msg.content) else {
                                // Forged, or from a peer speaking a protocol we
                                // don't know: drop it, don't end the room.
                                tracing::warn!("dropped an unreadable message");
                                continue;
                            };
                            let applied =
                                room.borrow_mut().apply(author, announce, now_ms());
                            if applied.changed {
                                emit(&on_state, &room);
                            }
                            if applied.round_changed {
                                // Answer the round change straight away: our
                                // value on a reveal, our cleared table on a new
                                // round.
                                broadcast(&gossip_tx, &secret, &room).await;
                            }
                        }
                        Event::NeighborUp(id) => {
                            if !joined {
                                joined = true;
                                status.set("connected", "Connected");
                            }
                            tracing::debug!(peer = %id, "neighbor up");
                            // Introduce ourselves so the new peer sees us
                            // without waiting for the next heartbeat.
                            broadcast(&gossip_tx, &secret, &room).await;
                        }
                        Event::NeighborDown(id) => {
                            tracing::debug!(peer = %id, "neighbor down");
                            room.borrow_mut().mark_unreachable(id, now_ms());
                        }
                        Event::Lagged => {
                            // We fell behind the gossip stream. Every peer
                            // re-announces its whole state, so the next
                            // heartbeat repairs the gap.
                            tracing::warn!("lagged behind the gossip stream");
                        }
                    }
                }
            });
        }

        // Re-announce on a timer: this both keeps us visible to peers that
        // joined late and expires peers that closed their tab.
        {
            let room = room.clone();
            let on_state = on_state.clone();
            let secret = secret.clone();
            task::spawn(async move {
                loop {
                    n0_future::time::sleep(Duration::from_millis(HEARTBEAT_MS)).await;
                    broadcast(&gossip_tx, &secret, &room).await;
                    // Bound separately: the `borrow_mut()` temporary would
                    // otherwise live to the end of the `if`, and `emit` borrows
                    // the room again.
                    let anyone_left = room.borrow_mut().expire(now_ms());
                    if anyone_left {
                        emit(&on_state, &room);
                    }
                }
            });
        }

        Ok(session)
    }

    /// The id other peers know us by.
    #[wasm_bindgen(getter, js_name = endpointId)]
    pub fn endpoint_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// An invite anyone in the room can share.
    ///
    /// It lists us first, then the peers we bootstrapped from, so an invite
    /// keeps working once the peer who opened the room has left.
    #[wasm_bindgen(getter)]
    pub fn invite(&self) -> String {
        let addrs = std::iter::once(self.endpoint.addr()).chain(self.bootstrap.iter().cloned());
        Ticket::new(self.topic, addrs).encode()
    }

    /// Casts (or changes) our vote for this round.
    pub fn vote(&self, value: String) {
        {
            let mut room = self.room.borrow_mut();
            if room.round.revealed {
                // Votes are locked once they are on the table.
                return;
            }
            let nonce = random_nonce();
            let peer = room.my_peer();
            peer.commitment = Some(commit(&value, &nonce));
            peer.value = Some(value);
            peer.nonce = Some(nonce);
            peer.seq += 1;
        }
        self.publish();
    }

    /// Turns every card face up.
    pub fn reveal(&self) {
        {
            let mut room = self.room.borrow_mut();
            if room.round.revealed {
                return;
            }
            room.round.revealed = true;
            room.my_peer().seq += 1;
        }
        self.publish();
    }

    /// Clears the table and starts the next round.
    pub fn reset(&self) {
        self.room.borrow_mut().start_next_round();
        self.publish();
    }

    /// Changes the name we show up under.
    #[wasm_bindgen(js_name = setName)]
    pub fn set_name(&self, name: String) {
        {
            let mut room = self.room.borrow_mut();
            let peer = room.my_peer();
            if peer.name == name {
                return;
            }
            peer.name = name;
            peer.seq += 1;
        }
        self.publish();
    }

    /// Leaves the room and closes the endpoint.
    pub async fn leave(self) {
        self.router.shutdown().await.ok();
        self.endpoint.close().await;
    }

    /// Applies our own change locally, then puts it on the wire.
    fn publish(&self) {
        self.emit_state();
        let gossip_tx = self.gossip_tx.clone();
        let secret = self.secret.clone();
        let room = self.room.clone();
        task::spawn(async move { broadcast(&gossip_tx, &secret, &room).await });
    }

    fn emit_state(&self) {
        emit(&self.on_state, &self.room);
    }
}

/// Serialises our own slice of the room and gossips it.
///
/// The borrow is released before the await so a re-entrant callback from JS
/// cannot hit a double-borrow panic.
async fn broadcast(gossip_tx: &GossipSender, secret: &iroh::SecretKey, room: &Rc<RefCell<Room>>) {
    let bytes = seal(secret, &room.borrow().announce());
    if let Err(err) = gossip_tx.broadcast(bytes.into()).await {
        tracing::warn!(?err, "failed to broadcast");
    }
}

fn emit(on_state: &js_sys::Function, room: &Rc<RefCell<Room>>) {
    let view = room.borrow().view();
    let json = serde_json::to_string(&view).expect("view is always serializable");
    if let Err(err) = on_state.call1(&JsValue::NULL, &JsValue::from_str(&json)) {
        web_sys::console::error_1(&err);
    }
}

/// The connectivity callback, cloned into the background tasks.
#[derive(Clone)]
struct Status(js_sys::Function);

impl Status {
    fn set(&self, kind: &str, detail: &str) {
        let _ = self.0.call2(
            &JsValue::NULL,
            &JsValue::from_str(kind),
            &JsValue::from_str(detail),
        );
    }
}

/// Re-exported so JS can show a peer id the same way Rust does.
#[wasm_bindgen(js_name = shortId)]
pub fn short_id(id: &str) -> String {
    id.parse::<EndpointId>()
        .map(|id| id.to_string().chars().take(7).collect())
        .unwrap_or_else(|_| id.chars().take(7).collect())
}
