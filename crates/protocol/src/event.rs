//! Client / Server event payloads.

use domain::{MatchId, PlayerId};
use serde::{Deserialize, Serialize};

/// Events emitted by the frontend to the engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    Hello { player: PlayerId },
    SubmitTurn { match_id: MatchId, action_index: u16, payload: serde_json::Value },
    RequestSnapshot { match_id: MatchId },
    Ping { nonce: u64 },
}

/// Events emitted by the engine to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Welcome { server_id: String, features: Vec<String> },
    Snapshot { match_id: MatchId, turn: u32 },
    TurnAccepted { match_id: MatchId, turn_id: String },
    TurnRejected { match_id: MatchId, reason: String },
    Pong { nonce: u64 },
    Error { message: String },
}
