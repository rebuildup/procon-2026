//! Bulk frame payloads (replays, summaries, etc.).

use domain::{MatchId, PlayerId};
use serde::{Deserialize, Serialize};

use super::MatchSnapshot;
use super::event::ServerEvent;

/// Server → client batch frame. Replays stream multiple events at once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerFrame {
    pub events: Vec<ServerEvent>,
}

/// Client → server batch frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientFrame {
    pub events: Vec<super::event::ClientEvent>,
}

/// Lightweight summary for Kanban / leaderboard surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchSummary {
    pub match_id: MatchId,
    pub players: Vec<PlayerId>,
    pub turn: u32,
    pub outcome_label: String,
}

impl From<&MatchSnapshot> for MatchSummary {
    fn from(snap: &MatchSnapshot) -> Self {
        let outcome_label = match &snap.outcome {
            domain::state::Outcome::Pending => "In progress".to_owned(),
            domain::state::Outcome::Draw => "Draw".to_owned(),
            domain::state::Outcome::Winner(p) => format!("Winner {p}"),
        };
        Self {
            match_id: snap.match_id.clone(),
            players: snap.state.scores.iter().map(|(p, _)| p.clone()).collect(),
            turn: snap.state.turn,
            outcome_label,
        }
    }
}
