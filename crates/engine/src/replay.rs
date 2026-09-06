//! Replay log: deterministic playback of an action log against a fresh state.

use domain::state::{GameState, Outcome, Score};
use domain::{CoreResult, MatchId, PlayerId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::match_table::MatchTable;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("core error: {0}")]
    Core(#[from] domain::CoreError),
    #[error("table error: {0}")]
    Table(#[from] crate::MatchTableError),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Action log entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub author: PlayerId,
    pub action_index: u16,
    pub payload: serde_json::Value,
}

/// Deterministic replay log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Replay {
    pub match_id: MatchId,
    pub players: Vec<PlayerId>,
    pub entries: Vec<LogEntry>,
}

impl Replay {
    /// Replay this log against an isolated [`MatchTable`]. Returns the final
    /// state as a snapshot.
    pub async fn play(&self, table: &MatchTable) -> CoreResult<GameState> {
        table
            .create(self.match_id.clone(), self.players.clone())
            .await
            .map_err(|e| domain::CoreError::InvariantViolation(e.to_string()))?;
        for entry in &self.entries {
            table
                .submit_turn(
                    &self.match_id,
                    &entry.author,
                    entry.action_index,
                    entry.payload.clone(),
                )
                .await
                .map_err(|e| domain::CoreError::InvariantViolation(e.to_string()))?;
        }
        let snap = table
            .snapshot(&self.match_id)
            .await
            .map_err(|e| domain::CoreError::InvariantViolation(e.to_string()))?;
        Ok(snap.state)
    }
}

/// Convenience wrapper for callers that want a single-flight replay.
#[derive(Debug, Default, Clone)]
pub struct ReplayRunner {
    table: Arc<RwLock<Option<Arc<MatchTable>>>>,
}

impl ReplayRunner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Run a replay using a shared, lazily-initialised table. Useful for
    /// tests that need multiple replays in parallel without state bleed.
    pub async fn run(&self, replay: Replay) -> CoreResult<GameState> {
        let table = {
            let guard = self.table.read().await;
            guard.clone()
        };
        let table = if let Some(table) = table {
            table
        } else {
            let mut guard = self.table.write().await;
            if guard.is_none() {
                guard.replace(Arc::new(MatchTable::new()));
            }
            guard.clone().expect("just inserted")
        };
        replay.play(&table).await
    }
}

/// Helper used by tests to build a fresh initial state.
#[must_use]
pub fn initial_state(match_id: MatchId, players: Vec<PlayerId>) -> GameState {
    GameState { match_id, turn: 0, scores: players.into_iter().map(|p| (p, Score(0))).collect() }
}

/// Helper used by tests to declare a winner.
#[must_use]
pub const fn declare_winner(p: PlayerId) -> Outcome {
    Outcome::Winner(p)
}
