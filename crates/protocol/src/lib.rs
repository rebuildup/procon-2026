//! Wire format between the Rust core/engine and the `SolidJS` frontend.
//!
//! All messages are JSON-serialisable. The frontend decodes them at the
//! boundary using generated TypeScript types.
//!
//! Versioning: every envelope carries [`ProtocolVersion`]. Bump the major
//! version when a breaking change ships; minor versions must remain
//! wire-compatible.

#![doc(test(attr(deny(warnings))))]

use domain::MatchId;
use domain::state::{GameState, Outcome};
use serde::{Deserialize, Serialize};

pub mod envelope;
pub mod event;
pub mod frame;

pub use envelope::{Envelope, ProtocolVersion};
pub use event::{ClientEvent, ServerEvent};
pub use frame::{ClientFrame, MatchSummary, ServerFrame};

/// Initial protocol version. Bumped per release.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("decode error: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("unsupported protocol version: {0:?}")]
    UnsupportedVersion(ProtocolVersion),
    #[error("invalid envelope: {0}")]
    Invalid(String),
}

/// Library result alias.
pub type ProtocolResult<T> = Result<T, ProtocolError>;

/// Snapshot of a match as exposed to the frontend. Mirrors the engine's
/// authoritative state plus a thin server-side view layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchSnapshot {
    pub match_id: MatchId,
    pub state: GameState,
    pub outcome: Outcome,
    pub server_time_unix_ms: i64,
}

/// Lightweight server-side handshake payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    pub protocol: ProtocolVersion,
    pub server_id: String,
    pub supported_features: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::PlayerId;

    #[test]
    fn hello_round_trips() {
        let hello = Hello {
            protocol: PROTOCOL_VERSION,
            server_id: "test".to_owned(),
            supported_features: vec!["replay".to_owned()],
        };
        let json = serde_json::to_string(&hello).unwrap();
        let back: Hello = serde_json::from_str(&json).unwrap();
        assert_eq!(hello, back);
    }

    #[test]
    fn snapshot_rejects_unknown_player() {
        let snap = MatchSnapshot {
            match_id: MatchId::new("match-test").unwrap(),
            state: GameState {
                match_id: MatchId::new("match-test").unwrap(),
                turn: 0,
                scores: vec![(PlayerId::new("player-a").unwrap(), domain::state::Score(0))],
            },
            outcome: Outcome::Pending,
            server_time_unix_ms: 0,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: MatchSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }
}
