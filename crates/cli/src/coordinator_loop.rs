//! Per-match orchestration body.
//!
//! Drives the official 4-endpoint protocol against a [`protocol::MatchClient`]
//! and an [`engine::solver::Solver`] stored as a trait object. The loop is
//! deliberately small and inspectable — the goal of issue #135 is to wire
//! the existing pieces together, not to optimise.
//!
//! # Phases
//!
//! 1. **Setting poll.** `GET /setting` repeatedly until `starts_at > 0`;
//!    on `403 AccessTimeError` we just sleep and retry (the server returns
//!    that envelope before start).
//! 2. **Agent submit.** `POST /agent` exactly once. The kinds vector is
//!    chosen from [`CoordinatorLoopConfig::solver_name`]: real solvers
//!    (`"loop"`, `"mcts"`, anything other than the debug-only `"wait"`)
//!    alternate `KIND_0` / `KIND_1` so half the agents patrol and half
//!    supply, which gives a richer test of the full protocol than the
//!    MVP's all-`KIND_0` placeholder.
//! 3. **Per-day loop.** `GET /` -> `solver.decide(&ctx)` -> adapter ->
//!    `POST /`. If `revision < 0`, return [`CoordinatorLoopError::Rejected`]
//!    without retrying. Otherwise sleep until `ends_at` and continue.
//!
//! [`protocol::MatchClient`]: protocol::MatchClient

use std::time::Duration;

use chrono::Utc;
use domain::{MatchId, PlayerId};
use protocol::match_config::AgentKind;
use thiserror::Error;

use crate::action_adapter::{AdapterError, decision_to_actions};
use crate::protocol_to_engine::protocol_setting_to_engine;
use crate::solver_adapter::{dummy_match_snapshot, match_state_to_snapshot};

/// Behavioural knobs for the coordinator loop. Server URL is carried by the
/// [`protocol::MatchClient`] itself.
#[derive(Debug, Clone)]
pub struct CoordinatorLoopConfig {
    /// Team id used to build `engine::SolverContext::my_team_id`. Also seeds
    /// the dummy `MatchId` (`"match-<team>"`) — the wire does not publish a
    /// match id, and the MVP does not need one beyond logs.
    pub team_id: String,
    /// Sleep between `GET /setting` polls while `starts_at == 0`.
    pub setting_poll_interval: Duration,
    /// Stable identifier of the solver the CLI wired up. Used only to
    /// decide what `POST /agent` kinds vector to send: real solvers
    /// (`"loop"`, `"mcts"`, anything other than `"wait"`) alternate
    /// `KIND_0` / `KIND_1`; the debug `WaitSolver` sticks to all `KIND_0`
    /// to keep its degenerate contract observable on the wire.
    ///
    /// Defaults to `"loop"` so the production path matches the documented
    /// Phase 2 strategy (`docs/contest/strategy.md`).
    pub solver_name: String,
}

impl Default for CoordinatorLoopConfig {
    fn default() -> Self {
        Self {
            team_id: "mvp-player".to_owned(),
            setting_poll_interval: Duration::from_secs(2),
            solver_name: "loop".to_owned(),
        }
    }
}

impl CoordinatorLoopConfig {
    /// Build a config from the coordinator runtime env vars.
    ///
    /// `PROCON_SERVER` and `PROCON_TOKEN` are mandatory — the loop is only
    /// meaningful when the server URL and bearer token are present, so we
    /// surface their absence as an `Err` rather than falling back to a
    /// localhost default. The legacy `PROCON_AGENT_URL` / `PROCON_SETTING_URL`
    /// overrides are intentionally NOT honoured: the loop talks to the server
    /// exclusively through `protocol::MatchClient`, which constructs its own
    /// endpoint paths.
    ///
    /// `PROCON_TEAM_ID` is optional; the MVP default of `"mvp-player"` is used
    /// when unset.
    #[allow(dead_code)] // Public API for external callers; not consumed in-process today.
    pub fn from_env() -> Result<Self, String> {
        std::env::var("PROCON_SERVER").map_err(|_| "PROCON_SERVER not set".to_owned()).and_then(
            |_| std::env::var("PROCON_TOKEN").map_err(|_| "PROCON_TOKEN not set".to_owned()),
        )?;
        let team_id = std::env::var("PROCON_TEAM_ID").unwrap_or_else(|_| "mvp-player".to_owned());
        Ok(Self {
            team_id,
            setting_poll_interval: Duration::from_secs(2),
            solver_name: "loop".to_owned(),
        })
    }
}

/// Aggregated result of a single match run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorOutcome {
    /// Number of days that completed (revision >= 0 returned).
    pub days_completed: usize,
    /// Per-day revision id (as returned by `POST /`).
    pub revisions: Vec<i32>,
}

/// Errors produced by the coordinator loop. Each variant points at one
/// phase so operators can attribute the failure quickly.
#[derive(Debug, Error)]
pub enum CoordinatorLoopError {
    /// `GET /setting` failed (network, auth, or other HTTP error).
    #[error("GET /setting failed: {0}")]
    Setting(#[from] protocol::HttpClientError),
    /// `POST /agent` failed.
    #[error("POST /agent failed: {0}")]
    AgentSubmit(protocol::HttpClientError),
    /// `GET /` failed.
    #[error("GET / failed: {0}")]
    State(protocol::HttpClientError),
    /// `POST /` failed (transport / non-200 / non-revision error).
    #[error("POST / failed: {0}")]
    ActionSubmit(protocol::HttpClientError),
    /// The server rejected the plan (`revision < 0`). Not retried.
    #[error("server rejected plan on day {day} (revision={revision})")]
    Rejected { day: u32, revision: i32 },
    /// Protocol <-> engine conversion produced a malformed value.
    #[error("protocol/engine conversion failed: {0}")]
    Conversion(String),
    /// The decision <-> action adapter produced a malformed value.
    #[error("action adapter failed on day {day}: {source}")]
    Adapter {
        day: u32,
        #[source]
        source: AdapterError,
    },
}

/// Drive one match end-to-end against `client` and `solver`.
///
/// # Errors
/// See [`CoordinatorLoopError`].
#[allow(clippy::too_many_lines)]
pub async fn run_coordinator_loop(
    client: protocol::MatchClient,
    solver: Box<dyn engine::solver::Solver>,
    cfg: CoordinatorLoopConfig,
) -> Result<CoordinatorOutcome, CoordinatorLoopError> {
    // Phase 1: poll /setting until start is announced.
    let wire_setting = loop {
        match client.get_match_setting().await {
            Ok(s) if s.starts_at > 0 => break s,
            Ok(_) => {
                tokio::time::sleep(cfg.setting_poll_interval).await;
            }
            Err(protocol::HttpClientError::AccessTime { .. }) => {
                // Pre-start: server returns 403 before match is announced.
                // Just back off and try again.
                tokio::time::sleep(cfg.setting_poll_interval).await;
            }
            Err(e) => return Err(CoordinatorLoopError::Setting(e)),
        }
    };

    // Phase 2: declare agent kinds. Real solvers alternate KIND_0 / KIND_1
    // so half the agents patrol and half supply; the debug `WaitSolver`
    // keeps the MVP's all-`KIND_0` placeholder so the wire shape stays
    // degenerate and trivially debuggable.
    let n_agents = wire_setting.agents.len();
    let kinds = agent_kinds_for(n_agents, &cfg.solver_name);
    client.submit_agent(kinds).await.map_err(CoordinatorLoopError::AgentSubmit)?;

    // Phase 3: convert wire -> engine setting, build team + match ids.
    let team = PlayerId::new(&cfg.team_id)
        .map_err(|e| CoordinatorLoopError::Conversion(format!("invalid team id: {e}")))?;
    let engine_setting = protocol_setting_to_engine(&wire_setting, team.clone())
        .map_err(CoordinatorLoopError::Conversion)?;
    let agent_ids: Vec<u8> = engine_setting.agent_ids();
    let match_id = MatchId::new(format!("match-{}", cfg.team_id))
        .map_err(|e| CoordinatorLoopError::Conversion(format!("invalid match id: {e}")))?;
    let total_days = wire_setting.day_steps.len();

    let mut revisions: Vec<i32> = Vec::with_capacity(total_days);
    let mut days_completed = 0_usize;

    // Phase 4: per-day loop.
    for day_idx in 0..total_days {
        let day = u32::try_from(day_idx).unwrap_or(0);
        let day_steps = u32::try_from(wire_setting.day_steps[day_idx])
            .map_err(|e| CoordinatorLoopError::Conversion(format!("day_steps[{day_idx}]={e}")))?;
        let day_seconds = i64::from(wire_setting.day_seconds[day_idx]);

        let state = client.get_match_state().await.map_err(CoordinatorLoopError::State)?;
        if state.day != i32::try_from(day).unwrap_or(i32::MAX) {
            return Err(CoordinatorLoopError::Conversion(format!(
                "server reported day {} while we expected {day}",
                state.day
            )));
        }

        let snapshot =
            match_state_to_snapshot(&state, &match_id, &team).unwrap_or_else(dummy_match_snapshot);
        let remaining_ms = u64::try_from(day_seconds.saturating_mul(1000)).unwrap_or(60_000);
        let ctx = engine::solver::SolverContext::new(
            snapshot,
            team.clone(),
            agent_ids.clone(),
            day,
            day_steps,
            remaining_ms,
        );

        let decision = solver.decide(&ctx);
        let actions = decision_to_actions(&decision, &ctx)
            .map_err(|source| CoordinatorLoopError::Adapter { day, source })?;

        let accepted = match client.submit_actions(actions).await {
            Ok(a) => a,
            Err(protocol::HttpClientError::Http { status: 200, message })
                if message.starts_with("rejected: revision=") =>
            {
                let revision =
                    message.trim_start_matches("rejected: revision=").parse::<i32>().unwrap_or(-1);
                return Err(CoordinatorLoopError::Rejected { day, revision });
            }
            Err(e) => return Err(CoordinatorLoopError::ActionSubmit(e)),
        };
        // submit_actions returns Err on revision<0, so reaching here means
        // the revision is non-negative. Store it and continue.
        revisions.push(accepted.revision);
        days_completed += 1;

        if day + 1 < u32::try_from(total_days).unwrap_or(u32::MAX) {
            let now = Utc::now().timestamp();
            let diff = state.ends_at - now;
            if diff > 0 {
                tokio::time::sleep(Duration::from_secs(u64::try_from(diff).unwrap_or(0))).await;
            }
        }
    }

    Ok(CoordinatorOutcome { days_completed, revisions })
}

/// Build the `POST /agent` kinds vector for a given solver name.
///
/// - `"wait"` / `"wait-mvp"` → all `KIND_0` (degenerate MVP placeholder).
/// - anything else (including the default `"loop"`) → alternating
///   `KIND_0` / `KIND_1` so half the agents patrol and half supply. This
///   exercises a wider surface of the contest server than the all-`KIND_0`
///   placeholder, catching kind-handling bugs early.
fn agent_kinds_for(n_agents: usize, solver_name: &str) -> Vec<AgentKind> {
    if solver_name == "wait" || solver_name == "wait-mvp" {
        return vec![AgentKind::KIND_0; n_agents];
    }
    (0..n_agents).map(|i| if i % 2 == 0 { AgentKind::KIND_0 } else { AgentKind::KIND_1 }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_solver_name_yields_all_kind_0() {
        let kinds = agent_kinds_for(5, "wait");
        assert_eq!(kinds, vec![AgentKind::KIND_0; 5]);
        let kinds = agent_kinds_for(3, "wait-mvp");
        assert_eq!(kinds, vec![AgentKind::KIND_0; 3]);
    }

    #[test]
    fn loop_solver_name_alternates_kind_0_and_kind_1() {
        let kinds = agent_kinds_for(4, "loop");
        assert_eq!(
            kinds,
            vec![AgentKind::KIND_0, AgentKind::KIND_1, AgentKind::KIND_0, AgentKind::KIND_1,]
        );
        // Odd count: last agent is KIND_0.
        let kinds = agent_kinds_for(5, "loop");
        assert_eq!(
            kinds,
            vec![
                AgentKind::KIND_0,
                AgentKind::KIND_1,
                AgentKind::KIND_0,
                AgentKind::KIND_1,
                AgentKind::KIND_0,
            ]
        );
    }

    #[test]
    fn unknown_solver_name_treated_as_real_solver() {
        // Anything other than "wait"/"wait-mvp" gets the alternating vector
        // so a future solver can ship without touching this helper.
        let kinds = agent_kinds_for(2, "mcts");
        assert_eq!(kinds, vec![AgentKind::KIND_0, AgentKind::KIND_1]);
    }

    #[test]
    fn zero_agents_yields_empty_kinds() {
        let kinds = agent_kinds_for(0, "loop");
        assert!(kinds.is_empty());
    }
}
