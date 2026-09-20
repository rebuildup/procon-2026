//! Per-match orchestration body.
//!
//! Drives the official 4-endpoint protocol against a [`protocol::MatchClient`]
//! and an [`engine::solver::Solver`] stored as a trait object. The loop is
//! deliberately small and inspectable — the goal of issue #135 is to wire
//! the existing pieces together, not to optimise.
//!
//! # Phases
//!
//! 1. **Setting probe.** `GET /setting` repeatedly until the server
//!    responds with any `MatchSetting` (including `starts_at == 0`).
//!    A `403 AccessTimeError` envelope is treated the same as a setting
//!    with `starts_at == 0`: we back off and retry.
//! 2. **Agent submit.** `POST /agent` with retry-on-`AccessTime`. The
//!    kinds vector is chosen from [`CoordinatorLoopConfig::solver_name`]:
//!    real solvers (`"loop"`, `"mcts"`, anything other than the
//!    debug-only `"wait"`) alternate `KIND_0` / `KIND_1` so half the
//!    agents patrol and half supply, which gives a richer test of the
//!    full protocol than the MVP's all-`KIND_0` placeholder.
//! 3. **Wire -> engine.** Convert the setting, build team + match ids.
//! 4. **Start wait.** Re-poll `GET /setting` until `starts_at > 0`.
//! 5. **Per-day loop.** `GET /` -> `solver.decide(&ctx)` -> adapter ->
//!    `POST /`. If `revision < 0`, return [`CoordinatorLoopError::Rejected`]
//!    without retrying. Otherwise sleep until `ends_at` and continue.
//!
//! ## Order rationale (issue #156)
//!
//! The official contest server returns `starts_at == 0` while the kind
//! submission window is **open** and only sets `starts_at` to a UNIX
//! timestamp **after** the kind deadline closes. An earlier version of
//! this loop polled `GET /setting` until `starts_at > 0` and only then
//! issued `POST /agent`, which by then always returned `403
//! AccessTimeError` ("agent kind submission is closed"). The fix moves
//! `POST /agent` ahead of the `starts_at` wait so kinds land in the
//! open window, and adds explicit retry-on-`AccessTime` to absorb the
//! small clock skew between the server's deadline and our poll loop.
//!
//! [`protocol::MatchClient`]: protocol::MatchClient

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use domain::{MatchId, PlayerId};
use engine::model::MatchSetting as EngineSetting;
use protocol::http_client::SubmitAck;
use protocol::match_config::AgentKind;
use serde::Serialize;
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
    /// Maximum number of `POST /agent` attempts before giving up. Each
    /// attempt that returns `403 AccessTimeError` (kind window not yet
    /// open, or already closed) is retried up to this many times before
    /// the loop surfaces [`CoordinatorLoopError::AgentSubmit`].
    ///
    /// Default `30` × `kind_submit_backoff = 500ms` = `15s` of retry
    /// budget, which comfortably covers the contest server's default
    /// 5s/60s kind deadlines plus poll-jitter.
    pub kind_submit_max_attempts: u32,
    /// Sleep between `POST /agent` retries on `403 AccessTimeError`.
    ///
    /// Default `500ms` keeps us under the official server's
    /// `kind-deadline` default (5s) and the manual-test 60s budget.
    pub kind_submit_backoff: Duration,
    /// Retry budget for `GET /` while waiting for the server's
    /// `-match-start-delay` to elapse (issue #164). The official
    /// contest server can briefly return `403 AccessTimeError` from
    /// `GET /` between the moment `starts_at` flips positive (visible
    /// in `GET /setting`) and the moment the match actually starts
    /// (the `-match-start-delay` is up to 5s by default).
    ///
    /// Default `30` × `match_state_backoff = 500ms` = `15s` of retry
    /// budget, which comfortably covers the documented worst case
    /// (`-match-start-delay 5s`) plus poll jitter. Tunable via
    /// `PROCON_MATCH_STATE_*` env vars if needed.
    pub match_state_max_attempts: u32,
    /// Sleep between `GET /` retries on `403 AccessTimeError`. Used
    /// only inside the post-`starts_at > 0` per-day loop; the
    /// pre-`starts_at` poll loop in Phase 4 keeps the broader
    /// `setting_poll_interval` so a slow start does not regress.
    ///
    /// Default `500ms` keeps us under the official server's
    /// `-match-start-delay` worst case (5s).
    pub match_state_backoff: Duration,
    /// Path to write `MatchStateSnapshot` JSON after each phase transition.
    /// If `None`, no snapshot is written. Atomic write via tmp file + rename.
    /// The frontend Live page polls this path via Vite's static-file
    /// serving; the CLI is intentionally agnostic about who consumes the
    /// file downstream.
    pub state_output_path: Option<std::path::PathBuf>,
    /// Server base URL recorded in the snapshot's `server` field. Set by
    /// the CLI after constructing `MatchClient`; if `None`, the snapshot
    /// uses `"unknown"`. `MatchClient::base_url()` is not exposed publicly
    /// (we must not modify `crates/protocol`), so callers are expected to
    /// forward the same URL they handed to the client builder.
    pub server_url: Option<String>,
    /// Optional pre-known match length (in days). When the caller already
    /// knows how many days the match will run for — e.g. the CLI loaded
    /// the `OpenAPI` fixture in `--dry-run` mode — the snapshot's
    /// `daysTotal` field can be populated before Phase 1 fetches the
    /// server's setting. When `None`, `daysTotal` is read from
    /// `setting.day_steps` at Phase 1 completion.
    pub days_total_hint: Option<u32>,
    /// Optional shared slot the loop populates with the parsed
    /// [`EngineSetting`] after Phase 1+3 succeed. Concrete solvers that
    /// want map-aware planning (see
    /// [`crate::solver_adapter::EngineLoopAdapter::setting_handle`])
    /// clone this slot before constructing the `Box<dyn Solver>` and
    /// pass the clone back here, so the loop can hand the real map +
    /// agents to the solver without going through `Box<dyn Solver>`'s
    /// type-erased boundary. When `None`, no injection happens (the
    /// solver falls back to its legacy, map-blind mode).
    pub engine_setting_slot: Option<Arc<Mutex<Option<EngineSetting>>>>,
}

impl Default for CoordinatorLoopConfig {
    fn default() -> Self {
        Self {
            team_id: "mvp-player".to_owned(),
            setting_poll_interval: Duration::from_secs(2),
            solver_name: "loop".to_owned(),
            kind_submit_max_attempts: 30,
            kind_submit_backoff: Duration::from_millis(500),
            match_state_max_attempts: 30,
            match_state_backoff: Duration::from_millis(500),
            state_output_path: None,
            server_url: None,
            days_total_hint: None,
            engine_setting_slot: None,
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
    /// when unset. `PROCON_STATE_OUTPUT` (when set) populates
    /// [`state_output_path`](Self::state_output_path).
    #[allow(dead_code)] // Public API for external callers; not consumed in-process today.
    pub fn from_env() -> Result<Self, String> {
        std::env::var("PROCON_SERVER").map_err(|_| "PROCON_SERVER not set".to_owned()).and_then(
            |_| std::env::var("PROCON_TOKEN").map_err(|_| "PROCON_TOKEN not set".to_owned()),
        )?;
        let team_id = std::env::var("PROCON_TEAM_ID").unwrap_or_else(|_| "mvp-player".to_owned());
        let server_url = std::env::var("PROCON_SERVER").ok();
        let state_output_path =
            std::env::var_os("PROCON_STATE_OUTPUT").map(std::path::PathBuf::from);
        Ok(Self {
            team_id,
            setting_poll_interval: Duration::from_secs(2),
            solver_name: "loop".to_owned(),
            // Use the documented defaults: 30 attempts × 500ms = 15s of
            // retry budget on `POST /agent`, which covers the official
            // 5s/60s `kind-deadline` plus poll jitter. Tweakable later via
            // PROCON_KIND_SUBMIT_* env vars if needed.
            kind_submit_max_attempts: 30,
            kind_submit_backoff: Duration::from_millis(500),
            // 30 attempts × 500ms = 15s of retry budget on `GET /`,
            // which covers the official `-match-start-delay` worst case
            // (5s by default, configurable up to 5s in the contest
            // binary) plus poll jitter. Tweakable later via
            // PROCON_MATCH_STATE_* env vars if needed.
            match_state_max_attempts: 30,
            match_state_backoff: Duration::from_millis(500),
            state_output_path,
            server_url,
            days_total_hint: None,
            engine_setting_slot: None,
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

/// Top-level status of a single coordinator run, as observed by the CLI.
///
/// Wire-mapping for the JSON snapshot is `lowercase` (see `MatchStatus`
/// derives). The frontend Live page treats this as the source of truth
/// for what to render in the per-day timeline.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchStatus {
    /// Pre-match: `GET /setting` responded but kinds not yet submitted.
    Waiting,
    /// Kinds submitted, per-day loop in progress.
    Running,
    /// All days accepted, the loop returned `Ok`.
    Completed,
    /// Server returned `revision < 0` on some day.
    Rejected,
    /// Transport / adapter / conversion error.
    Error,
}

/// JSON snapshot written to `--state-output` after each phase transition.
///
/// The frontend Live page polls this file via the Vite dev server's
/// static-file serving. Schema version `1` is wire-stable — bump
/// `schema_version` on any breaking change.
///
/// `Option` fields use `null` (serde default) rather than being omitted
/// so the frontend can rely on the field being present in every snapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchStateSnapshot {
    /// Schema version. Increment on any breaking change to the wire format.
    pub schema_version: u32,
    /// `MatchClient` base URL (e.g. `http://localhost:8080`). `null` only
    /// in degenerate / test harnesses that omit the field.
    pub server: String,
    /// `--solver` value used by this run.
    pub solver: String,
    /// Current top-level status (see [`MatchStatus`]).
    pub status: MatchStatus,
    /// Current day index (0-based). `null` before day 0 starts and after
    /// successful completion.
    pub day: Option<u32>,
    /// Total days in this match (`setting.day_steps.len()` or
    /// `days_total_hint` when known up-front).
    pub days_total: u32,
    /// Last accepted revision. `null` before the first acceptance.
    pub revision: Option<i32>,
    /// Per-day revision array as it grows. Empty until the first day is
    /// accepted.
    pub revisions: Vec<i32>,
    /// Unix seconds when this snapshot was written.
    pub last_update_unix: i64,
    /// Error message. `Some` only when `status == "error"` or
    /// `status == "rejected"` (the rejection carries a brief summary).
    pub error: Option<String>,
}

impl MatchStateSnapshot {
    /// Build the canonical "match finished successfully" snapshot from a
    /// [`CoordinatorOutcome`]. The frontend treats this as the terminal
    /// state of the Live page timeline.
    #[allow(dead_code)] // Public API for external callers (e.g. integration tests).
    pub fn completed(
        outcome: &CoordinatorOutcome,
        server: &str,
        solver: &str,
        days_total: u32,
    ) -> Self {
        Self {
            schema_version: 1,
            server: server.to_owned(),
            solver: solver.to_owned(),
            status: MatchStatus::Completed,
            // No "current" day after successful completion.
            day: None,
            days_total,
            revision: outcome.revisions.last().copied(),
            revisions: outcome.revisions.clone(),
            last_update_unix: unix_now(),
            error: None,
        }
    }
}

/// Wall-clock seconds since the Unix epoch. Returns `0` if the system
/// clock is somehow before 1970 — the frontend treats `0` as "synthetic /
/// test fixture" rather than panicking.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Best-effort write of `snap` to `cfg.state_output_path` (if set).
///
/// Atomic via tmp + rename: a half-written file would otherwise be visible
/// to the frontend during the brief window between `write` and `rename`.
/// Snapshot writing is observability, not correctness — every failure
/// is logged via `tracing::warn!` and never propagated. The match loop
/// continues whether or not the snapshot was successfully written.
fn write_snapshot(cfg: &CoordinatorLoopConfig, snap: &MatchStateSnapshot) {
    let Some(path) = cfg.state_output_path.as_ref() else {
        return;
    };
    let Ok(json) = serde_json::to_vec_pretty(snap) else {
        tracing::warn!("state snapshot: serde_json serialization failed");
        return;
    };
    let tmp = path.with_extension("json.tmp");
    let write_result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = write_result {
        tracing::warn!(?path, error = %e, "state snapshot write failed");
    }
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
    // Initial Waiting snapshot — written BEFORE Phase 1 so the frontend
    // sees a stable file as soon as the CLI starts.
    let server_str = cfg.server_url.clone().unwrap_or_else(|| "unknown".to_owned());
    let mut snap = MatchStateSnapshot {
        schema_version: 1,
        server: server_str.clone(),
        solver: cfg.solver_name.clone(),
        status: MatchStatus::Waiting,
        day: None,
        // `days_total_hint` lets the snapshot carry a sensible total
        // before Phase 1 reads it from the server. `0` otherwise; the
        // field is overwritten with the real total once Phase 1 returns.
        days_total: cfg.days_total_hint.unwrap_or(0),
        revision: None,
        revisions: Vec::new(),
        last_update_unix: unix_now(),
        error: None,
    };
    write_snapshot(&cfg, &snap);

    // All match phases run inside the inner helper. Each successful day
    // notifies the outer caller via `on_day_accepted` so the outer
    // function can write a `Running` snapshot with the new revision.
    // Errors propagate to the outer caller, which writes the terminal
    // snapshot (Rejected / Error) before returning.
    let on_day_accepted = |day: u32, revision: i32, s: &mut MatchStateSnapshot| {
        s.status = MatchStatus::Running;
        s.day = Some(day);
        s.revision = Some(revision);
        s.revisions.push(revision);
        s.last_update_unix = unix_now();
        write_snapshot(&cfg, s);
    };

    let result =
        run_coordinator_loop_inner(client, solver, cfg.clone(), &mut snap, on_day_accepted).await;

    // Final snapshot: terminal status (Completed / Rejected / Error).
    match &result {
        Ok(outcome) => {
            snap.status = MatchStatus::Completed;
            snap.day = None;
            snap.revision = outcome.revisions.last().copied();
            snap.revisions.clone_from(&outcome.revisions);
            snap.last_update_unix = unix_now();
            snap.error = None;
            write_snapshot(&cfg, &snap);
        }
        Err(CoordinatorLoopError::Rejected { day, revision }) => {
            snap.status = MatchStatus::Rejected;
            snap.day = Some(*day);
            snap.revision = Some(*revision);
            snap.revisions.push(*revision);
            snap.last_update_unix = unix_now();
            snap.error = Some(format!("rejected day={day} revision={revision}"));
            write_snapshot(&cfg, &snap);
        }
        Err(e) => {
            snap.status = MatchStatus::Error;
            snap.last_update_unix = unix_now();
            snap.error = Some(e.to_string());
            write_snapshot(&cfg, &snap);
        }
    }

    result
}

/// Inner driver. Mirrors `run_coordinator_loop` byte-for-byte but
/// returns the per-day `(day, revision)` pairs to a closure so the
/// caller can materialise `Running` snapshots between phases. The outer
/// function is responsible for the terminal snapshot.
#[allow(clippy::too_many_lines)]
async fn run_coordinator_loop_inner<F>(
    client: protocol::MatchClient,
    solver: Box<dyn engine::solver::Solver>,
    cfg: CoordinatorLoopConfig,
    snap: &mut MatchStateSnapshot,
    mut on_day_accepted: F,
) -> Result<CoordinatorOutcome, CoordinatorLoopError>
where
    F: FnMut(u32, i32, &mut MatchStateSnapshot),
{
    // Phase 1: probe /setting until the server is responsive (any
    // `MatchSetting`, including `starts_at == 0`). The official server
    // returns `starts_at == 0` while the kind-submission window is
    // OPEN, then sets `starts_at` only AFTER the kind deadline closes —
    // so we MUST NOT gate Phase 2 on `starts_at > 0` (issue #156).
    let wire_setting = loop {
        match client.get_match_setting().await {
            Ok(s) => break s,
            Err(protocol::HttpClientError::AccessTime { .. }) => {
                // Pre-start window: server returns 403 before the first
                // setting snapshot exists. Just back off and retry.
                tokio::time::sleep(cfg.setting_poll_interval).await;
            }
            Err(e) => return Err(CoordinatorLoopError::Setting(e)),
        }
    };

    // Update the Waiting snapshot now that Phase 1 has revealed the
    // real day count — this is also when we flip to `Running` after
    // Phase 2 (kinds accepted) below.
    snap.days_total = u32::try_from(wire_setting.day_steps.len()).unwrap_or(0);

    // Phase 2: declare agent kinds. Real solvers alternate KIND_0 / KIND_1
    // so half the agents patrol and half supply; the debug `WaitSolver`
    // keeps the MVP's all-`KIND_0` placeholder so the wire shape stays
    // degenerate and trivially debuggable.
    //
    // We submit DURING the kind window (right after Phase 1 succeeds)
    // rather than waiting for `starts_at > 0`, because `starts_at` flips
    // to a non-zero timestamp only AFTER the server closes the kind
    // window — by which time `POST /agent` always returns 403.
    //
    // `submit_agent_with_retry` retries `403 AccessTimeError` for the
    // window between "server became responsive" and "server flipped
    // starts_at", which is exactly the open kind window.
    let n_agents = wire_setting.agents.len();
    let kinds = agent_kinds_for(n_agents, &cfg.solver_name);
    submit_agent_with_retry(&client, kinds, cfg.kind_submit_max_attempts, cfg.kind_submit_backoff)
        .await?;

    // Kinds accepted -> Running snapshot. `day` stays `None` because
    // we have not yet entered the per-day driver.
    snap.status = MatchStatus::Running;
    snap.day = None;
    snap.last_update_unix = unix_now();
    write_snapshot(&cfg, snap);

    // Phase 3: convert wire -> engine setting, build team + match ids.
    let team = PlayerId::new(&cfg.team_id)
        .map_err(|e| CoordinatorLoopError::Conversion(format!("invalid team id: {e}")))?;
    let engine_setting = protocol_setting_to_engine(&wire_setting, team.clone())
        .map_err(CoordinatorLoopError::Conversion)?;
    let agent_ids: Vec<u8> = engine_setting.agent_ids();

    // Hand the parsed engine setting (real map + agent positions) to any
    // solver that pre-registered a setting slot (see
    // [`crate::solver_adapter::EngineLoopAdapter::setting_handle`]).
    // Without this injection the adapter falls back to a stand-in
    // problem that ignores the real map, and the official server
    // rejects the first day with `revision=-1` (issue #158).
    if let Some(slot) = cfg.engine_setting_slot.as_ref() {
        let mut guard = slot.lock().map_err(|_| {
            CoordinatorLoopError::Conversion("engine setting slot mutex poisoned".to_owned())
        })?;
        *guard = Some(engine_setting.clone());
    }
    let match_id = MatchId::new(format!("match-{}", cfg.team_id))
        .map_err(|e| CoordinatorLoopError::Conversion(format!("invalid match id: {e}")))?;
    let total_days = wire_setting.day_steps.len();

    // Phase 4: wait for `starts_at > 0`. Phase 1 may have caught the
    // setting while `starts_at` was still 0 (kind window OPEN), so we
    // re-poll here until the server announces the start. The rest of
    // the setting (map / day_steps / day_seconds / agents) is static so
    // the snapshot from Phase 1 stays valid.
    if wire_setting.starts_at == 0 {
        loop {
            match client.get_match_setting().await {
                Ok(s) if s.starts_at > 0 => break,
                Ok(_) => tokio::time::sleep(cfg.setting_poll_interval).await,
                Err(protocol::HttpClientError::AccessTime { .. }) => {
                    tokio::time::sleep(cfg.setting_poll_interval).await;
                }
                Err(e) => return Err(CoordinatorLoopError::Setting(e)),
            }
        }
    }

    let mut revisions: Vec<i32> = Vec::with_capacity(total_days);
    let mut days_completed = 0_usize;

    // Phase 5: per-day loop.
    for day_idx in 0..total_days {
        let day = u32::try_from(day_idx).unwrap_or(0);
        let day_steps = u32::try_from(wire_setting.day_steps[day_idx])
            .map_err(|e| CoordinatorLoopError::Conversion(format!("day_steps[{day_idx}]={e}")))?;
        let day_seconds = i64::from(wire_setting.day_seconds[day_idx]);

        let state =
            fetch_state_with_retry(&client, cfg.match_state_max_attempts, cfg.match_state_backoff)
                .await?;
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

        // Operator visibility: when DEBUG logging is on, dump the
        // exact wire payload we are about to send. Gated by the
        // `tracing::enabled!` check so the per-step formatting cost
        // stays out of the hot path in production.
        if tracing::enabled!(tracing::Level::DEBUG) {
            tracing::debug!(day, body = ?actions, "submitting plan");
        }

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

        // Per-day Running snapshot. The closure mutates `snap` and
        // calls `write_snapshot` so the frontend sees incremental
        // progress after each accepted day.
        on_day_accepted(day, accepted.revision, snap);

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

/// Submit the agent-kind vector to `POST /agent`, retrying on
/// `403 AccessTimeError`.
///
/// `MatchClient::submit_agent` already retries transient HTTP failures
/// (5xx, 408, 429, transport errors) internally, but it explicitly does
/// **not** retry on `403 AccessTimeError` — that envelope is reserved
/// for "the kind window is closed", which is conceptually permanent
/// from the client's point of view. For issue #156 we want exactly the
/// opposite behaviour: the loop calls `POST /agent` IMMEDIATELY after
/// `GET /setting` becomes responsive, so the *first* attempt can race
/// the kind-deadline on a slow boot and land in the closed window. We
/// therefore wrap the call in an outer retry loop that sleeps
/// `backoff` and tries again until either the call succeeds or we burn
/// through `max_attempts`.
///
/// Any non-`AccessTime` error is propagated immediately — those are
/// surface failures (auth, malformed body, transport, …) and must not
/// be retried blindly.
async fn submit_agent_with_retry(
    client: &protocol::MatchClient,
    kinds: Vec<AgentKind>,
    max_attempts: u32,
    backoff: Duration,
) -> Result<SubmitAck, CoordinatorLoopError> {
    let mut last_access_time: Option<String> = None;
    for attempt in 0..max_attempts {
        match client.submit_agent(kinds.clone()).await {
            Ok(ack) => return Ok(ack),
            Err(protocol::HttpClientError::AccessTime { message }) => {
                // The kind window is either not yet open (server still
                // in pre-start) or just closed (race against deadline).
                // Either way, retrying with backoff is the documented
                // behaviour. Cache the last message so we can surface
                // it once the budget is exhausted.
                tracing::debug!(attempt, message = %message, "POST /agent 403 AccessTime, retrying");
                last_access_time = Some(message);
                tokio::time::sleep(backoff).await;
            }
            Err(e) => return Err(CoordinatorLoopError::AgentSubmit(e)),
        }
    }
    Err(CoordinatorLoopError::AgentSubmit(protocol::HttpClientError::AccessTime {
        message: last_access_time
            .unwrap_or_else(|| "kind window did not open within retry budget".to_owned()),
    }))
}

/// Retry `GET /` while the server is still in the pre-start window
/// (`-match-start-delay` may cause a brief `403 AccessTimeError` between
/// `starts_at` flipping positive in `GET /setting` and the actual match
/// start, see issue #164). All other errors propagate immediately.
///
/// `max_attempts` × `backoff` covers the 1s default and 5s worst case
/// of `-match-start-delay` with comfortable headroom (e.g. 30 × 500ms
/// = 15s budget).
async fn fetch_state_with_retry(
    client: &protocol::MatchClient,
    max_attempts: u32,
    backoff: Duration,
) -> Result<protocol::match_config::MatchState, CoordinatorLoopError> {
    let mut last_access_time: Option<String> = None;
    for attempt in 0..max_attempts {
        match client.get_match_state().await {
            Ok(state) => return Ok(state),
            Err(protocol::HttpClientError::AccessTime { message }) => {
                // The match has been announced via `starts_at > 0` but
                // the server's `-match-start-delay` (default 1s,
                // configurable up to 5s) has not yet elapsed. Retry
                // with backoff until the budget is exhausted; cache
                // the last message so the terminal error is meaningful.
                tracing::debug!(attempt, message = %message, "GET / 403 AccessTime, retrying");
                last_access_time = Some(message);
                tokio::time::sleep(backoff).await;
            }
            Err(e) => return Err(CoordinatorLoopError::State(e)),
        }
    }
    Err(CoordinatorLoopError::State(protocol::HttpClientError::AccessTime {
        message: last_access_time
            .unwrap_or_else(|| "match-start-delay did not elapse within retry budget".to_owned()),
    }))
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
