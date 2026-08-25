//! Planning poker over [iroh]: a browser peer joins a gossip topic, and the
//! room is whoever else is on that topic.
//!
//! Browsers cannot send UDP, so every iroh connection made here is carried over
//! a relay via WebSocket. The relay only ever sees ciphertext — the QUIC
//! connections between peers are end-to-end encrypted — so the app needs no
//! backend of its own and can be served as static files.
//!
//! [iroh]: https://www.iroh.computer

// The room and its wire format are the reusable half of this crate, and are
// public so they can be exercised on any target.
pub mod protocol;
// Invites name an iroh topic and the peers to dial. The WebRTC build addresses
// rooms by plain id instead, and has no use for them.
#[cfg(feature = "iroh-transport")]
pub mod ticket;

// One module per transport, each browser-only; on other targets the crate still
// builds so the room logic can be unit tested. `protocol` knows about neither:
// it is the same room semantics either way.
#[cfg(all(target_family = "wasm", feature = "iroh-transport"))]
mod session;
#[cfg(all(target_family = "wasm", feature = "webrtc-transport"))]
mod room_api;
