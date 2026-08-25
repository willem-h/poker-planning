//! Invite tickets: everything a browser needs to find the room.

use iroh::EndpointAddr;
use iroh_gossip::proto::TopicId;
use serde::{Deserialize, Serialize};

/// How many bootstrap peers an invite carries.
///
/// More than one so that an invite keeps working after the peer who created
/// the room closes their tab.
const MAX_BOOTSTRAP: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub topic: TopicId,
    pub bootstrap: Vec<EndpointAddr>,
}

#[derive(Debug)]
pub struct InvalidTicket(String);

impl std::fmt::Display for InvalidTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid invite: {}", self.0)
    }
}

impl std::error::Error for InvalidTicket {}

impl Ticket {
    pub fn new(topic: TopicId, bootstrap: impl IntoIterator<Item = EndpointAddr>) -> Self {
        let mut seen = Vec::new();
        for addr in bootstrap {
            if seen.len() == MAX_BOOTSTRAP {
                break;
            }
            if !seen.iter().any(|a: &EndpointAddr| a.id == addr.id) {
                seen.push(addr);
            }
        }
        Self {
            topic,
            bootstrap: seen,
        }
    }

    pub fn encode(&self) -> String {
        let bytes = postcard::to_stdvec(self).expect("ticket is always serializable");
        data_encoding::BASE32_NOPAD
            .encode(&bytes)
            .to_ascii_lowercase()
    }

    pub fn decode(text: &str) -> Result<Self, InvalidTicket> {
        // Accept a bare ticket, a full invite URL, or anything with the ticket
        // in the fragment, so pasting from a chat window just works.
        let text = text.trim();
        let text = text
            .rsplit_once("#room=")
            .map(|(_, ticket)| ticket)
            .unwrap_or(text);
        let text = text.split(['&', '?', ' ']).next().unwrap_or(text);

        let bytes = data_encoding::BASE32_NOPAD
            .decode(text.to_ascii_uppercase().as_bytes())
            .map_err(|e| InvalidTicket(e.to_string()))?;
        postcard::from_bytes(&bytes).map_err(|e| InvalidTicket(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(seed: u8) -> EndpointAddr {
        EndpointAddr::from(iroh::SecretKey::from_bytes(&[seed; 32]).public())
    }

    fn ticket() -> Ticket {
        Ticket::new(TopicId::from_bytes([7; 32]), [addr(1), addr(2)])
    }

    #[test]
    fn round_trips() {
        let decoded = Ticket::decode(&ticket().encode()).expect("decodes");
        assert_eq!(decoded.topic, ticket().topic);
        assert_eq!(decoded.bootstrap.len(), 2);
    }

    #[test]
    fn decodes_out_of_an_invite_url() {
        let encoded = ticket().encode();
        let url = format!("https://example.com/poker/#room={encoded}");
        assert_eq!(Ticket::decode(&url).expect("decodes").topic, ticket().topic);
    }

    #[test]
    fn rejects_junk() {
        assert!(Ticket::decode("not-a-ticket!!").is_err());
    }

    #[test]
    fn drops_duplicate_and_excess_bootstrap_peers() {
        let t = Ticket::new(
            TopicId::from_bytes([7; 32]),
            [addr(1), addr(1), addr(2), addr(3), addr(4), addr(5), addr(6)],
        );
        assert_eq!(t.bootstrap.len(), MAX_BOOTSTRAP);
        assert_eq!(t.bootstrap[0].id, addr(1).id);
        assert_eq!(t.bootstrap[1].id, addr(2).id);
    }
}
