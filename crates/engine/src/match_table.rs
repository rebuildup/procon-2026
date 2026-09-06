//! In-memory match registry. Each match runs its own state machine; the table
//! multiplexes them under a single lock-free arena.

use domain::state::{GameState, Outcome, Score};
use domain::{CoreResult, MatchId, PlayerId, TurnId};
use std::collections::HashMap;
use std::sync::Arc;

use protocol::{MatchSnapshot, ServerEvent};
use tokio::sync::RwLock;
use tracing::{debug, instrument};

/// Error type for [`MatchTable`] operations.
#[derive(Debug, thiserror::Error)]
pub enum MatchTableError {
    #[error("match not found: {0}")]
    NotFound(MatchId),
    #[error("match already exists: {0}")]
    AlreadyExists(MatchId),
    #[error("core error: {0}")]
    Core(#[from] domain::CoreError),
}

type Result<T> = std::result::Result<T, MatchTableError>;

/// Internal entry.
#[derive(Debug)]
struct MatchEntry {
    state: GameState,
    outcome: Outcome,
}

/// Thread-safe registry of matches.
#[derive(Debug, Default)]
pub struct MatchTable {
    inner: Arc<RwLock<HashMap<MatchId, MatchEntry>>>,
}

impl MatchTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new match. Errors if the id is already in use.
    #[instrument(skip(self, players))]
    pub async fn create(&self, match_id: MatchId, players: Vec<PlayerId>) -> Result<()> {
        if players.is_empty() {
            return Err(MatchTableError::Core(domain::CoreError::InvariantViolation(
                "match requires at least one player".to_owned(),
            )));
        }
        let mut guard = self.inner.write().await;
        if guard.contains_key(&match_id) {
            return Err(MatchTableError::AlreadyExists(match_id));
        }
        let scores = players.into_iter().map(|p| (p, Score(0))).collect();
        let state = GameState { match_id: match_id.clone(), turn: 0, scores };
        guard.insert(match_id, MatchEntry { state, outcome: Outcome::Pending });
        debug!("match created");
        Ok(())
    }

    /// Snapshot the current authoritative state for a match.
    #[instrument(skip(self))]
    pub async fn snapshot(&self, match_id: &MatchId) -> Result<MatchSnapshot> {
        let guard = self.inner.read().await;
        let entry =
            guard.get(match_id).ok_or_else(|| MatchTableError::NotFound(match_id.clone()))?;
        Ok(MatchSnapshot {
            match_id: match_id.clone(),
            state: entry.state.clone(),
            outcome: entry.outcome.clone(),
            server_time_unix_ms: chrono::Utc::now().timestamp_millis(),
        })
    }

    /// Apply a turn. Returns the produced [`ServerEvent`] describing the result.
    ///
    /// This is a placeholder implementation that increments the turn counter
    /// and accepts any well-formed turn from a registered player. Real rules
    /// land in subsequent tickets.
    #[instrument(skip(self, payload))]
    pub async fn submit_turn(
        &self,
        match_id: &MatchId,
        author: &PlayerId,
        action_index: u16,
        payload: serde_json::Value,
    ) -> Result<(ServerEvent, TurnId)> {
        let mut guard = self.inner.write().await;
        let entry =
            guard.get_mut(match_id).ok_or_else(|| MatchTableError::NotFound(match_id.clone()))?;
        let turn_id = TurnId::new(format!("turn-{}", entry.state.turn + 1))
            .map_err(|e| MatchTableError::Core(domain::CoreError::InvariantViolation(e)))?;
        let turn = domain::state::Turn {
            id: turn_id.clone(),
            author: author.clone(),
            action_index,
            payload,
        };
        turn.validate(&entry.state)?;
        entry.state.turn += 1;
        Ok((
            ServerEvent::TurnAccepted { match_id: match_id.clone(), turn_id: turn_id.to_string() },
            turn_id,
        ))
    }
}

/// Convenience: [`MatchTable::create`] returning a [`CoreResult`].
pub async fn create_match(
    table: &MatchTable,
    match_id: MatchId,
    players: Vec<PlayerId>,
) -> CoreResult<()> {
    match table.create(match_id, players).await {
        Ok(()) => Ok(()),
        Err(MatchTableError::Core(e)) => Err(e),
        Err(other) => Err(domain::CoreError::InvariantViolation(other.to_string())),
    }
}
