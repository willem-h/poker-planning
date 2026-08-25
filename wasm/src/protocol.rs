//! Wire protocol and room state.
//!
//! There is no host: every peer keeps a full copy of the room and every peer
//! broadcasts its own slice of it. Merging is last-writer-wins per peer, which
//! makes the room converge no matter what order messages arrive in, and makes
//! a peer leaving a non-event for everyone else.

use std::collections::HashMap;

use iroh_base::{EndpointId, PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};

/// How long a peer may stay silent before we consider it gone.
///
/// Generous on purpose: browsers throttle timers in hidden tabs, so a
/// participant who switched away is quiet for much longer than their heartbeat
/// interval suggests, and dropping them off the table would be wrong.
pub const PEER_TIMEOUT_MS: f64 = 45_000.0;
/// How often each peer re-announces itself.
pub const HEARTBEAT_MS: u64 = 5_000;
/// How long a peer gets after gossip reports it unreachable.
///
/// Losing a direct connection usually means the peer is gone, but it can also
/// be the swarm reshuffling around a peer that is still here. Rather than
/// dropping them outright we shorten their timeout: if they are still around
/// their next heartbeat lands well inside this window.
pub const UNREACHABLE_GRACE_MS: f64 = 8_000.0;

pub const DECK: &[&str] = &["0", "1", "2", "3", "5", "8", "13", "21", "?", "coffee"];

/// Where a round is in its lifecycle.
///
/// Rounds form a lattice: a higher number always wins, and within one number
/// `revealed` wins over hidden. Any peer may advance it, and all peers land on
/// the same value without needing to agree first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub number: u64,
    pub revealed: bool,
}

impl Round {
    pub fn initial() -> Self {
        Self {
            number: 1,
            revealed: false,
        }
    }

    /// Merges `other` into `self`, returning true if that changed anything.
    pub fn merge(&mut self, other: Round) -> bool {
        let newer = (other.number, other.revealed) > (self.number, self.revealed);
        if newer {
            *self = other;
        }
        newer
    }

    pub fn next(self) -> Self {
        Self {
            number: self.number + 1,
            revealed: false,
        }
    }
}

/// One peer's announcement of itself, rebroadcast on every change and heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announce {
    pub name: String,
    pub round: Round,
    /// A hash of `value:nonce`, published as soon as a vote is cast.
    ///
    /// Publishing the commitment rather than the vote is what keeps votes
    /// secret before the reveal: with no host to withhold them, a peer that
    /// shipped its raw vote early would be readable by anyone patching their
    /// client. The commitment proves a vote exists and pins it down without
    /// disclosing it.
    pub commitment: Option<String>,
    /// The vote itself, published only once the round is revealed.
    pub value: Option<String>,
    pub nonce: Option<String>,
    /// Monotonic per-peer counter, used to drop out-of-order announcements.
    pub seq: u64,
}

/// Everything we know about one peer, including ourselves.
#[derive(Debug, Clone)]
pub struct Peer {
    pub name: String,
    pub commitment: Option<String>,
    pub value: Option<String>,
    pub nonce: Option<String>,
    pub seq: u64,
    pub last_seen_ms: f64,
}

/// An [`Announce`] with its author attached, as it goes over the wire.
///
/// Gossip messages travel by flooding, so the peer that hands us a message is
/// usually not the peer that wrote it — `delivered_from` names the previous
/// hop. The author has to be carried in the message itself, and signed, or a
/// peer relaying for others could rewrite their votes on the way through.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Signed {
    from: PublicKey,
    payload: Vec<u8>,
    signature: Vec<u8>,
}

/// Why a message was thrown away.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RejectedMessage {
    /// Not a message from this protocol at all.
    Malformed,
    /// The signature does not match the claimed author.
    BadSignature,
}

/// Serialises an announcement and signs it as us.
pub fn seal(secret: &SecretKey, announce: &Announce) -> Vec<u8> {
    let payload = postcard::to_allocvec(announce).expect("announce is always serializable");
    let signature = secret.sign(&payload);
    postcard::to_allocvec(&Signed {
        from: secret.public(),
        payload,
        signature: signature.to_bytes().to_vec(),
    })
    .expect("signed message is always serializable")
}

/// Recovers an announcement and the peer that actually wrote it.
pub fn open(bytes: &[u8]) -> Result<(EndpointId, Announce), RejectedMessage> {
    let signed: Signed = postcard::from_bytes(bytes).map_err(|_| RejectedMessage::Malformed)?;
    let signature: [u8; Signature::LENGTH] = signed
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| RejectedMessage::BadSignature)?;
    signed
        .from
        .verify(&signed.payload, &Signature::from_bytes(&signature))
        .map_err(|_| RejectedMessage::BadSignature)?;
    let announce = postcard::from_bytes(&signed.payload).map_err(|_| RejectedMessage::Malformed)?;
    Ok((signed.from, announce))
}

/// What folding in a remote announcement did to our view.
#[derive(Debug, Default, Clone, Copy)]
pub struct Applied {
    /// The room changed and should be re-rendered.
    pub changed: bool,
    /// The round we hold moved, so our own state needs re-announcing: on a
    /// reveal that means publishing our value, and on a new round it means
    /// telling everyone our table is clear. Waiting for the next heartbeat
    /// would leave the room looking stuck for seconds.
    pub round_changed: bool,
}

/// What a revealed vote turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteState {
    /// No vote cast yet.
    None,
    /// A commitment is in, but the round is still hidden.
    Hidden,
    /// Revealed and the value matches the commitment.
    Revealed,
    /// Revealed but the value does not match the commitment: someone changed
    /// their vote after committing to it.
    Tampered,
}

fn new_nonce() -> String {
    let bytes: [u8; 8] = rand::random();
    data_encoding::HEXLOWER.encode(&bytes)
}

pub fn commit(value: &str, nonce: &str) -> String {
    let hash = blake3::hash(format!("{value}:{nonce}").as_bytes());
    data_encoding::HEXLOWER.encode(&hash.as_bytes()[..16])
}

/// The merged view of the room, rendered by the UI.
#[derive(Debug, Serialize)]
pub struct RoomView {
    pub round: u64,
    pub revealed: bool,
    pub me: String,
    pub participants: Vec<ParticipantView>,
    pub stats: Option<Stats>,
}

#[derive(Debug, Serialize)]
pub struct ParticipantView {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub is_me: bool,
    /// One of "none" | "hidden" | "revealed" | "tampered".
    pub state: &'static str,
    pub value: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Stats {
    pub average: Option<f64>,
    pub median: Option<f64>,
    pub consensus: bool,
    /// Vote value -> how many peers picked it, most popular first.
    pub tally: Vec<(String, usize)>,
}

pub struct Room {
    pub me: EndpointId,
    pub round: Round,
    pub peers: HashMap<EndpointId, Peer>,
}

impl Room {
    pub fn new(me: EndpointId, my_name: String, now_ms: f64) -> Self {
        let mut peers = HashMap::new();
        peers.insert(
            me,
            Peer {
                name: my_name,
                commitment: None,
                value: None,
                nonce: None,
                seq: 0,
                last_seen_ms: now_ms,
            },
        );
        Self {
            me,
            round: Round::initial(),
            peers,
        }
    }

    pub fn my_peer(&mut self) -> &mut Peer {
        self.peers.get_mut(&self.me).expect("self is always present")
    }

    /// Records our own estimate for this round.
    ///
    /// Returns false once the cards are face up: a vote changed then would only
    /// contradict the commitment everyone has already checked.
    pub fn cast_vote(&mut self, value: String) -> bool {
        if self.round.revealed {
            return false;
        }
        let nonce = new_nonce();
        let peer = self.my_peer();
        peer.commitment = Some(commit(&value, &nonce));
        peer.value = Some(value);
        peer.nonce = Some(nonce);
        peer.seq += 1;
        true
    }

    /// Turns every card face up. Returns false if they already were.
    pub fn reveal(&mut self) -> bool {
        if self.round.revealed {
            return false;
        }
        self.round.revealed = true;
        self.my_peer().seq += 1;
        true
    }

    /// Changes the name we appear under. Returns false if it was already that.
    pub fn rename(&mut self, name: String) -> bool {
        let peer = self.my_peer();
        if peer.name == name {
            return false;
        }
        peer.name = name;
        peer.seq += 1;
        true
    }

    /// Starts the next round, clearing the table.
    pub fn start_next_round(&mut self) {
        self.round = self.round.next();
        self.clear_votes();
    }

    /// Drops every vote, ours included. Called whenever the round number moves.
    fn clear_votes(&mut self) {
        for peer in self.peers.values_mut() {
            peer.commitment = None;
            peer.value = None;
            peer.nonce = None;
        }
        // Our cleared state is a new announcement, not a repeat of the last one.
        self.my_peer().seq += 1;
    }

    /// Folds a remote announcement in.
    pub fn apply(&mut self, from: EndpointId, announce: Announce, now_ms: f64) -> Applied {
        if from == self.me {
            return Applied::default();
        }
        let round_before = self.round.number;
        let round_changed = self.round.merge(announce.round);
        let mut changed = round_changed;
        // Whoever moved the round on, everyone's old votes belong to the round
        // that just ended.
        if self.round.number > round_before {
            self.clear_votes();
        }

        match self.peers.get_mut(&from) {
            // A stale announcement still proves the peer is alive, but must not
            // roll its vote back.
            Some(existing) if announce.seq < existing.seq => {
                existing.last_seen_ms = now_ms;
            }
            Some(existing) => {
                changed |= existing.name != announce.name
                    || existing.commitment != announce.commitment
                    || existing.value != announce.value;
                existing.name = announce.name;
                existing.commitment = announce.commitment;
                existing.value = announce.value;
                existing.nonce = announce.nonce;
                existing.seq = announce.seq;
                existing.last_seen_ms = now_ms;
            }
            None => {
                self.peers.insert(
                    from,
                    Peer {
                        name: announce.name,
                        commitment: announce.commitment,
                        value: announce.value,
                        nonce: announce.nonce,
                        seq: announce.seq,
                        last_seen_ms: now_ms,
                    },
                );
                changed = true;
            }
        }
        Applied {
            changed,
            round_changed,
        }
    }

    /// Notes that gossip can no longer reach a peer directly, starting a short
    /// countdown to dropping it.
    pub fn mark_unreachable(&mut self, id: EndpointId, now_ms: f64) {
        if id == self.me {
            return;
        }
        if let Some(peer) = self.peers.get_mut(&id) {
            let deadline = now_ms - (PEER_TIMEOUT_MS - UNREACHABLE_GRACE_MS);
            peer.last_seen_ms = peer.last_seen_ms.min(deadline);
        }
    }

    /// Drops peers we have not heard from in a while. Returns true if any went.
    pub fn expire(&mut self, now_ms: f64) -> bool {
        let me = self.me;
        let before = self.peers.len();
        self.peers
            .retain(|id, p| *id == me || now_ms - p.last_seen_ms < PEER_TIMEOUT_MS);
        self.peers.len() != before
    }

    /// Builds the announcement describing our own state right now.
    pub fn announce(&self) -> Announce {
        let me = self.peers.get(&self.me).expect("self is always present");
        Announce {
            name: me.name.clone(),
            round: self.round,
            commitment: me.commitment.clone(),
            // Hold the raw vote back until the round is revealed.
            value: if self.round.revealed {
                me.value.clone()
            } else {
                None
            },
            nonce: if self.round.revealed {
                me.nonce.clone()
            } else {
                None
            },
            seq: me.seq,
        }
    }

    fn vote_state(&self, peer: &Peer) -> VoteState {
        match (&peer.commitment, self.round.revealed) {
            (None, _) => VoteState::None,
            (Some(_), false) => VoteState::Hidden,
            (Some(commitment), true) => match (&peer.value, &peer.nonce) {
                // Revealed for everyone else, but this peer's value has not
                // reached us yet.
                (None, _) | (_, None) => VoteState::Hidden,
                (Some(value), Some(nonce)) => {
                    if &commit(value, nonce) == commitment {
                        VoteState::Revealed
                    } else {
                        VoteState::Tampered
                    }
                }
            },
        }
    }

    pub fn view(&self) -> RoomView {
        let mut participants: Vec<ParticipantView> = self
            .peers
            .iter()
            .map(|(id, peer)| {
                let state = self.vote_state(peer);
                ParticipantView {
                    id: id.to_string(),
                    short_id: id.to_string().chars().take(7).collect(),
                    name: peer.name.clone(),
                    is_me: *id == self.me,
                    state: match state {
                        VoteState::None => "none",
                        VoteState::Hidden => "hidden",
                        VoteState::Revealed => "revealed",
                        VoteState::Tampered => "tampered",
                    },
                    value: match state {
                        VoteState::Revealed | VoteState::Tampered => peer.value.clone(),
                        _ => None,
                    },
                }
            })
            .collect();
        // Stable order so cards don't shuffle on every re-render.
        participants.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));

        let stats = self.round.revealed.then(|| self.stats());

        RoomView {
            round: self.round.number,
            revealed: self.round.revealed,
            me: self.me.to_string(),
            participants,
            stats,
        }
    }

    fn stats(&self) -> Stats {
        let revealed: Vec<&str> = self
            .peers
            .values()
            .filter(|p| self.vote_state(p) == VoteState::Revealed)
            .filter_map(|p| p.value.as_deref())
            .collect();

        let mut tally: HashMap<&str, usize> = HashMap::new();
        for value in &revealed {
            *tally.entry(value).or_default() += 1;
        }
        let mut tally: Vec<(String, usize)> = tally
            .into_iter()
            .map(|(value, count)| (value.to_string(), count))
            .collect();
        tally.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        // "?" and "coffee" are opinions about the story, not estimates.
        let mut numbers: Vec<f64> = revealed
            .iter()
            .filter_map(|v| v.parse::<f64>().ok())
            .collect();
        numbers.sort_by(|a, b| a.partial_cmp(b).expect("no NaN from parse"));

        let average = (!numbers.is_empty())
            .then(|| numbers.iter().sum::<f64>() / numbers.len() as f64)
            .map(|avg| (avg * 100.0).round() / 100.0);

        let median = match numbers.len() {
            0 => None,
            n if n % 2 == 1 => Some(numbers[n / 2]),
            n => Some((numbers[n / 2 - 1] + numbers[n / 2]) / 2.0),
        };

        Stats {
            average,
            median,
            consensus: revealed.len() > 1 && tally.len() == 1,
            tally,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer_id(seed: u8) -> EndpointId {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    fn room() -> Room {
        Room::new(peer_id(1), "me".into(), 0.0)
    }

    fn announce_vote(round: Round, value: &str, nonce: &str, seq: u64) -> Announce {
        Announce {
            name: "them".into(),
            round,
            commitment: Some(commit(value, nonce)),
            value: round.revealed.then(|| value.to_string()),
            nonce: round.revealed.then(|| nonce.to_string()),
            seq,
        }
    }

    fn secret(seed: u8) -> SecretKey {
        SecretKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn a_sealed_message_names_its_author() {
        let secret = secret(9);
        let announce = announce_vote(Round::initial(), "5", "n", 1);
        let (author, decoded) = open(&seal(&secret, &announce)).expect("opens");
        assert_eq!(author, secret.public());
        assert_eq!(decoded.commitment, announce.commitment);
    }

    #[test]
    fn a_relayed_message_cannot_be_rewritten() {
        // The author is signed over, so a peer forwarding for someone else
        // cannot substitute its own payload and keep their name on it.
        let mut bytes = seal(&secret(9), &announce_vote(Round::initial(), "5", "n", 1));
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert_eq!(open(&bytes).unwrap_err(), RejectedMessage::BadSignature);
    }

    #[test]
    fn junk_is_rejected_not_panicked_on() {
        assert!(open(b"hello").is_err());
        assert!(open(&[]).is_err());
    }

    #[test]
    fn round_lattice_orders_reveal_after_hidden() {
        let mut round = Round::initial();
        assert!(round.merge(Round {
            number: 1,
            revealed: true
        }));
        assert!(round.revealed);
        // A hidden round 1 must not undo the reveal.
        assert!(!round.merge(Round {
            number: 1,
            revealed: false
        }));
        assert!(round.revealed);
        // A new round does.
        assert!(round.merge(Round {
            number: 2,
            revealed: false
        }));
        assert_eq!(round.number, 2);
        assert!(!round.revealed);
    }

    #[test]
    fn reveal_by_a_peer_advances_everyone() {
        let mut room = room();
        let them = peer_id(2);
        let revealed = Round {
            number: 1,
            revealed: true,
        };
        assert!(room.apply(them, announce_vote(revealed, "5", "n", 1), 0.0).changed);
        assert!(room.round.revealed);
    }

    #[test]
    fn votes_stay_hidden_until_revealed() {
        let mut room = room();
        let them = peer_id(2);
        room.apply(
            them,
            announce_vote(Round::initial(), "5", "n", 1),
            0.0,
        );
        let view = room.view();
        let theirs = view.participants.iter().find(|p| !p.is_me).expect("peer");
        assert_eq!(theirs.state, "hidden");
        assert_eq!(theirs.value, None);
    }

    #[test]
    fn a_vote_changed_after_committing_is_flagged() {
        let mut room = room();
        let them = peer_id(2);
        let round = Round {
            number: 1,
            revealed: true,
        };
        let mut announce = announce_vote(round, "5", "n", 1);
        announce.value = Some("13".into()); // commitment still says "5"
        room.apply(them, announce, 0.0);

        let view = room.view();
        let theirs = view.participants.iter().find(|p| !p.is_me).expect("peer");
        assert_eq!(theirs.state, "tampered");
        // A tampered vote is shown but never counted.
        assert_eq!(view.stats.expect("revealed").tally, vec![]);
    }

    #[test]
    fn stale_announcements_do_not_roll_a_vote_back() {
        let mut room = room();
        let them = peer_id(2);
        let round = Round {
            number: 1,
            revealed: true,
        };
        room.apply(them, announce_vote(round, "8", "n2", 2), 0.0);
        room.apply(them, announce_vote(round, "3", "n1", 1), 1.0);

        let view = room.view();
        let theirs = view.participants.iter().find(|p| !p.is_me).expect("peer");
        assert_eq!(theirs.value.as_deref(), Some("8"));
    }

    #[test]
    fn stats_ignore_non_numeric_votes() {
        let mut room = room();
        let round = Round {
            number: 1,
            revealed: true,
        };
        room.round = round;
        room.my_peer().commitment = Some(commit("3", "a"));
        room.my_peer().value = Some("3".into());
        room.my_peer().nonce = Some("a".into());
        room.apply(peer_id(2), announce_vote(round, "5", "b", 1), 0.0);
        room.apply(peer_id(3), announce_vote(round, "coffee", "c", 1), 0.0);

        let stats = room.view().stats.expect("revealed");
        assert_eq!(stats.average, Some(4.0));
        assert_eq!(stats.median, Some(4.0));
        assert!(!stats.consensus);
        assert_eq!(stats.tally.len(), 3);
    }

    #[test]
    fn consensus_needs_more_than_one_voter() {
        let mut room = room();
        let round = Round {
            number: 1,
            revealed: true,
        };
        room.round = round;
        room.apply(peer_id(2), announce_vote(round, "5", "b", 1), 0.0);
        assert!(!room.view().stats.expect("revealed").consensus);

        room.apply(peer_id(3), announce_vote(round, "5", "c", 1), 0.0);
        assert!(room.view().stats.expect("revealed").consensus);
    }

    #[test]
    fn a_new_round_clears_every_cached_vote() {
        let mut room = room();
        let them = peer_id(2);
        let revealed = Round {
            number: 1,
            revealed: true,
        };
        room.round = revealed;
        room.my_peer().commitment = Some(commit("3", "a"));
        room.my_peer().value = Some("3".into());
        room.my_peer().nonce = Some("a".into());
        room.apply(them, announce_vote(revealed, "5", "b", 1), 0.0);

        // A peer starts round 2 and its announcement carries no vote.
        let next = revealed.next();
        let applied = room.apply(
            them,
            Announce {
                name: "them".into(),
                round: next,
                commitment: None,
                value: None,
                nonce: None,
                seq: 2,
            },
            1.0,
        );
        assert!(applied.round_changed);

        let view = room.view();
        assert_eq!(view.round, 2);
        assert!(!view.revealed);
        assert!(view.participants.iter().all(|p| p.state == "none"));
        // Our own cleared state must go back out, so peers stop showing our
        // last round's vote.
        assert!(room.announce().commitment.is_none());
    }

    #[test]
    fn casting_a_vote_commits_to_it() {
        let mut room = room();
        assert!(room.cast_vote("5".into()));

        let me = room.peers.get(&room.me).expect("self").clone();
        let commitment = me.commitment.expect("committed");
        let nonce = me.nonce.expect("nonced");
        assert_eq!(commit("5", &nonce), commitment);
        // Two votes for the same value get different nonces, so a peer cannot
        // recognise an estimate by its commitment alone.
        room.round.revealed = false;
        let first = commitment;
        room.cast_vote("5".into());
        assert_ne!(
            room.peers.get(&room.me).expect("self").commitment.as_deref(),
            Some(first.as_str())
        );
    }

    #[test]
    fn a_vote_is_refused_once_the_cards_are_up() {
        let mut room = room();
        room.cast_vote("5".into());
        assert!(room.reveal());

        assert!(!room.cast_vote("13".into()));
        assert!(!room.reveal(), "revealing twice is a no-op");
        assert_eq!(
            room.peers.get(&room.me).expect("self").value.as_deref(),
            Some("5")
        );
    }

    #[test]
    fn renaming_is_a_no_op_when_the_name_is_unchanged() {
        let mut room = room();
        assert!(!room.rename("me".into()));
        assert!(room.rename("Ada".into()));
        assert_eq!(room.announce().name, "Ada");
    }

    #[test]
    fn starting_a_round_locally_clears_the_table() {
        let mut room = room();
        room.my_peer().commitment = Some(commit("3", "a"));
        room.apply(peer_id(2), announce_vote(Round::initial(), "5", "b", 1), 0.0);

        room.start_next_round();

        assert_eq!(room.round.number, 2);
        assert!(room.view().participants.iter().all(|p| p.state == "none"));
    }

    #[test]
    fn a_reveal_asks_us_to_republish_our_own_vote() {
        let mut room = room();
        room.my_peer().commitment = Some(commit("5", "n"));
        room.my_peer().value = Some("5".into());
        room.my_peer().nonce = Some("n".into());

        let revealed = Round {
            number: 1,
            revealed: true,
        };
        let applied = room.apply(peer_id(2), announce_vote(revealed, "8", "m", 1), 0.0);

        assert!(applied.round_changed, "a reveal has to be answered");
        // Our value is only on the wire once we know the round is revealed.
        assert_eq!(room.announce().value.as_deref(), Some("5"));
    }

    #[test]
    fn an_unreachable_peer_is_given_a_grace_period_not_dropped() {
        let mut room = room();
        let them = peer_id(2);
        room.apply(them, announce_vote(Round::initial(), "5", "n", 1), 0.0);

        room.mark_unreachable(them, 0.0);
        assert!(!room.expire(0.0), "dropped immediately");
        assert!(!room.expire(UNREACHABLE_GRACE_MS - 1.0), "dropped inside the grace period");
        assert!(room.expire(UNREACHABLE_GRACE_MS + 1.0), "outlived the grace period");
    }

    #[test]
    fn a_peer_that_speaks_again_survives_being_marked_unreachable() {
        let mut room = room();
        let them = peer_id(2);
        room.apply(them, announce_vote(Round::initial(), "5", "n", 1), 0.0);
        room.mark_unreachable(them, 0.0);

        // Its next heartbeat arrives inside the grace period.
        room.apply(them, announce_vote(Round::initial(), "5", "n", 2), 3_000.0);

        assert!(!room.expire(UNREACHABLE_GRACE_MS + 1.0));
        assert_eq!(room.peers.len(), 2);
    }

    #[test]
    fn silent_peers_expire_but_we_never_do() {
        let mut room = room();
        room.apply(peer_id(2), announce_vote(Round::initial(), "5", "n", 1), 0.0);
        assert_eq!(room.peers.len(), 2);

        assert!(room.expire(PEER_TIMEOUT_MS + 1.0));
        assert_eq!(room.peers.len(), 1);
        assert!(room.peers.contains_key(&room.me));
    }

    #[test]
    fn our_own_announcement_withholds_the_vote_until_reveal() {
        let mut room = room();
        room.my_peer().commitment = Some(commit("5", "n"));
        room.my_peer().value = Some("5".into());
        room.my_peer().nonce = Some("n".into());

        assert_eq!(room.announce().value, None);
        room.round.revealed = true;
        assert_eq!(room.announce().value.as_deref(), Some("5"));
    }
}
