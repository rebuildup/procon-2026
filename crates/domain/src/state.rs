//! Core state types: [`GameState`], [`Turn`], [`Score`], [`Outcome`].
//!
//! These are minimal placeholders. The real contest model is introduced via
//! GitHub Issues / ADRs once the contest problem statement is final.

use serde::{Deserialize, Serialize};

use crate::id::{MatchId, PlayerId, TurnId};
use crate::{CoreError, CoreResult};

/// Aggregate game state at a point in time. Domain-agnostic; specifics live in
/// the engine and frontend layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    pub match_id: MatchId,
    pub turn: u32,
    pub scores: Vec<(PlayerId, Score)>,
}

/// Single turn of play. The action payload is intentionally generic until the
/// real game model lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub id: TurnId,
    pub author: PlayerId,
    pub action_index: u16,
    pub payload: serde_json::Value,
}

impl Turn {
    /// Construct a turn with an empty payload. Real validation is layered on top.
    pub fn new(id: TurnId, author: PlayerId, action_index: u16) -> Self {
        Self { id, author, action_index, payload: serde_json::Value::Null }
    }

    /// Validate that this turn is well-formed relative to the given state.
    ///
    /// This is a placeholder that rejects empty `action_index` until the
    /// concrete game model exists.
    pub fn validate(&self, state: &GameState) -> CoreResult<()> {
        if self.action_index == 0 {
            return Err(CoreError::IllegalMove(format!(
                "turn {} has zero action_index in match {}",
                self.id, state.match_id
            )));
        }
        if !state.scores.iter().any(|(p, _)| *p == self.author) {
            return Err(CoreError::InvariantViolation(format!(
                "author {} is not part of match {}",
                self.author, state.match_id
            )));
        }
        Ok(())
    }
}

/// Per-player score. Concrete scoring rules live in the engine layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Score(pub i64);

/// Terminal outcome of a match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Pending,
    Draw,
    Winner(PlayerId),
}
