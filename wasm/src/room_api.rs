//! The room, exposed to JavaScript without a transport attached.
//!
//! The iroh build owns its connections; this one does not. JS hands us bytes it
//! received and asks for bytes to send, which is all a room needs to work — the
//! merging, commitments and signatures are the same code either way.

use wasm_bindgen::prelude::*;

use crate::protocol::{self, open, seal, Room, HEARTBEAT_MS, PEER_TIMEOUT_MS};

/// Installs a panic hook that reports to the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("poker-wasm panic: {info}").into());
    }));
}

/// The deck, handed to JS so the card values live in one place.
#[wasm_bindgen(js_name = deck)]
pub fn deck() -> Vec<String> {
    protocol::DECK.iter().map(|v| v.to_string()).collect()
}

/// How often the page should rebroadcast, in milliseconds.
#[wasm_bindgen(js_name = heartbeatMs)]
pub fn heartbeat_ms() -> u32 {
    HEARTBEAT_MS as u32
}

/// How long a silent peer is kept, in milliseconds.
#[wasm_bindgen(js_name = peerTimeoutMs)]
pub fn peer_timeout_ms() -> f64 {
    PEER_TIMEOUT_MS
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// What arriving bytes did to the room, so JS knows whether to re-render and
/// whether to answer.
#[wasm_bindgen]
pub struct Applied {
    changed: bool,
    round_changed: bool,
    author: Option<String>,
}

#[wasm_bindgen]
impl Applied {
    /// The room changed and should be re-rendered.
    #[wasm_bindgen(getter)]
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The round moved, so our own state should go back out immediately rather
    /// than waiting for the next heartbeat.
    #[wasm_bindgen(getter, js_name = roundChanged)]
    pub fn round_changed(&self) -> bool {
        self.round_changed
    }

    /// Who signed the message, or nothing if it was rejected.
    ///
    /// The transport's own peer ids are its business; this is the identity the
    /// room is keyed by, and lets the page map one onto the other.
    #[wasm_bindgen(getter)]
    pub fn author(&self) -> Option<String> {
        self.author.clone()
    }
}

/// A room with no wire of its own.
#[wasm_bindgen]
pub struct RoomHandle {
    room: Room,
    secret: iroh_base::SecretKey,
}

#[wasm_bindgen]
impl RoomHandle {
    /// Opens a room under a fresh identity.
    #[wasm_bindgen(constructor)]
    pub fn new(name: String) -> RoomHandle {
        let secret = iroh_base::SecretKey::generate();
        RoomHandle {
            room: Room::new(secret.public(), name, now_ms()),
            secret,
        }
    }

    /// The id other peers know us by. Note that this is *our* identity, not the
    /// transport's: a peer is the key that signed its announcements, whatever
    /// connection carried them.
    #[wasm_bindgen(getter, js_name = endpointId)]
    pub fn endpoint_id(&self) -> String {
        self.room.me.to_string()
    }

    /// Our own state, signed and ready to send to everyone.
    pub fn announce(&self) -> Vec<u8> {
        seal(&self.secret, &self.room.announce())
    }

    /// Folds in bytes that arrived from a peer.
    ///
    /// The sender the transport reports is ignored on purpose. WebRTC gives us
    /// a trustworthy channel to the peer at the other end, but a peer can still
    /// pass on messages for others, so the author is the key that signed it.
    pub fn receive(&mut self, bytes: &[u8]) -> Applied {
        let Ok((author, announce)) = open(bytes) else {
            web_sys::console::warn_1(&"dropped an unreadable message".into());
            return Applied {
                changed: false,
                round_changed: false,
                author: None,
            };
        };
        let applied = self.room.apply(author, announce, now_ms());
        Applied {
            changed: applied.changed,
            round_changed: applied.round_changed,
            author: Some(author.to_string()),
        }
    }

    /// Casts (or changes) our vote. Returns false once the cards are up.
    pub fn vote(&mut self, value: String) -> bool {
        self.room.cast_vote(value)
    }

    /// Turns every card face up. Returns false if they already were.
    pub fn reveal(&mut self) -> bool {
        self.room.reveal()
    }

    /// Clears the table and starts the next round.
    pub fn reset(&mut self) {
        self.room.start_next_round();
    }

    /// Changes the name we appear under. Returns false if it was already that.
    #[wasm_bindgen(js_name = setName)]
    pub fn set_name(&mut self, name: String) -> bool {
        self.room.rename(name)
    }

    /// Notes that the transport has lost its connection to a peer, starting a
    /// short countdown to dropping it.
    #[wasm_bindgen(js_name = markUnreachable)]
    pub fn mark_unreachable(&mut self, id: &str) {
        if let Ok(id) = id.parse() {
            self.room.mark_unreachable(id, now_ms());
        }
    }

    /// Drops peers we have not heard from in a while. Returns true if any went.
    pub fn expire(&mut self) -> bool {
        self.room.expire(now_ms())
    }

    /// The room as the UI renders it, as JSON.
    pub fn view(&self) -> String {
        serde_json::to_string(&self.room.view()).expect("view is always serializable")
    }
}
