//! Top-level engine server wiring. Concrete transport (WebSocket / IPC / CLI)
//! is layered on this façade; for now it exposes async methods only.

use std::sync::Arc;

use domain::id::{MatchId, PlayerId};
use protocol::{Hello, MatchSnapshot, PROTOCOL_VERSION, ServerEvent};
use tokio::sync::RwLock;
use tracing::{info, instrument};

use crate::match_table::MatchTable;

/// Engine server configuration.
#[derive(Debug, Clone)]
pub struct EngineServerConfig {
    pub server_id: String,
    pub supported_features: Vec<String>,
}

impl Default for EngineServerConfig {
    fn default() -> Self {
        Self {
            server_id: "procon-2026-engine".to_owned(),
            supported_features: vec!["snapshot".to_owned(), "replay".to_owned()],
        }
    }
}

/// Engine server façade. Owns the [`MatchTable`] and produces server-side
/// events in response to [`protocol::ClientEvent`]s.
#[derive(Debug)]
pub struct EngineServer {
    config: EngineServerConfig,
    table: Arc<RwLock<MatchTable>>,
}

impl EngineServer {
    #[must_use]
    pub fn new(config: EngineServerConfig) -> Self {
        Self { config, table: Arc::new(RwLock::new(MatchTable::new())) }
    }

    /// Produce the welcome payload sent on connect.
    #[must_use]
    pub fn hello(&self) -> Hello {
        Hello {
            protocol: PROTOCOL_VERSION,
            server_id: self.config.server_id.clone(),
            supported_features: self.config.supported_features.clone(),
        }
    }

    /// Snapshot a match.
    #[instrument(skip(self))]
    pub async fn snapshot(&self, match_id: &MatchId) -> anyhow::Result<MatchSnapshot> {
        let guard = self.table.read().await;
        guard.snapshot(match_id).await.map_err(anyhow::Error::from)
    }

    /// Submit a turn and return the resulting event.
    #[instrument(skip(self, payload))]
    pub async fn submit(
        &self,
        match_id: &MatchId,
        author: &PlayerId,
        action_index: u16,
        payload: serde_json::Value,
    ) -> anyhow::Result<ServerEvent> {
        let guard = self.table.read().await;
        let (event, _turn_id) = guard.submit_turn(match_id, author, action_index, payload).await?;
        info!(?event, "turn submitted");
        Ok(event)
    }
}
