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
//!    kinds vector is chosen by [`agent_kinds_for`]: an explicit
//!    [`CoordinatorLoopConfig::agent_kinds`] pattern (the live
//!    `--kinds` flag, Issue #238/#243) is expanded over the team's
//!    slots and wins for **every** solver; when it is `None` the
//!    historical rule from `solver_name` applies unchanged — real
//!    solvers (`"loop"`, `"mcts"`, anything other than the debug-only
//!    `"wait"`) alternate `KIND_0` / `KIND_1` so half the agents
//!    patrol and half supply, which gives a richer test of the full
//!    protocol than the MVP's all-`KIND_0` placeholder.
//! 3. **Wire -> engine.** Convert the setting, build team + match ids.
//!    With an explicit `--kinds` the composition that was just posted is
//!    handed to `protocol_setting_to_engine_with_kinds`, so the planning
//!    model (patrol fuel caps included) describes the team the server was
//!    told about; without the flag the historical conversion is unchanged.
//! 4. **Start wait.** Re-poll `GET /setting` until `starts_at > 0`.
//! 5. **Per-day loop.** `GET /` -> `solver.decide(&ctx)` -> adapter ->
//!    `POST /`, then sleep until `ends_at`. The loop follows the day the
//!    server reports and keeps going when a day is lost (Issue #385): a plan
//!    with no verdict is sent again while the answer window is open, a
//!    rejected plan (`revision < 0`) or a solver that misses the window costs
//!    that day only, and [`CoordinatorLoopError::Rejected`] is returned after
//!    the last day rather than at the first rejection.
//!
//! A process started while a match is already running (`starts_at > 0` and
//! `GET /` answers) skips phases 2 and 4 and joins at the server's day with
//! the composition the server reports.
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

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use domain::{MatchId, PlayerId};
use engine::model::AgentKind as EngineAgentKind;
use engine::model::MatchSetting as EngineSetting;
use protocol::http_client::SubmitAck;
use protocol::match_config::{
    ActionPlanBody, AgentKind, AgentState as WireAgentState, MatchState as WireMatchState,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::action_adapter::decision_to_actions;
use crate::arena::Secret;
use crate::candidate_bridge::{apply_live_agents, apply_live_traffic};
use crate::match_record::{DayStatus, MatchRecorder, RecordEvent, SubmitOutcome};
use crate::protocol_to_engine::{
    protocol_setting_to_engine, protocol_setting_to_engine_with_kinds,
};
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
    /// Maximum `GET /setting` polls before the loop gives up — applied to the
    /// Phase 1 pre-start probe *and* the Phase 4 `starts_at == 0` wait.
    ///
    /// Both loops used to spin forever: `get_match_setting` returns promptly
    /// on `403 AccessTimeError` without retrying, so a cancelled match or a
    /// drifted API hung an unattended run with no log line at all, while
    /// every other retry in this file is bounded (`kind_submit_max_attempts`,
    /// `match_state_max_attempts`). Default `300` × `setting_poll_interval =
    /// 2s` = `10min` — far beyond a normal kind window plus the official
    /// `-match-start-delay`, but finite, so the wait ends in
    /// [`CoordinatorLoopError::SettingPollTimeout`] instead of silence.
    pub setting_poll_max_attempts: u32,
    /// Stable identifier of the solver the CLI wired up. Used only to
    /// decide what `POST /agent` kinds vector to send **when
    /// [`agent_kinds`](Self::agent_kinds) is `None`**: real solvers
    /// (`"loop"`, `"mcts"`, anything other than `"wait"`) alternate
    /// `KIND_0` / `KIND_1`; the debug `WaitSolver` sticks to all `KIND_0`
    /// to keep its degenerate contract observable on the wire.
    ///
    /// Defaults to `"loop"` so the production path matches the documented
    /// Phase 2 strategy (`docs/contest/strategy.md`).
    pub solver_name: String,
    /// Explicit agent-kind composition (live `--kinds <pattern>`,
    /// Issues #238/#243): the parsed pattern, one kind per pattern slot,
    /// cycled over the team's `n` agent slots by [`agent_kinds_for`].
    ///
    /// `None` (the default, i.e. the flag was omitted) keeps today's
    /// rule exactly — `solver_name` decides, byte for byte as before.
    /// `Some` wins for **every** solver, including the debug `wait` /
    /// `wait-mvp` pair: the operator asked for a composition, so the
    /// composition is what gets submitted.
    ///
    /// The parsed pattern is what [`bench::kinds::parse_kinds`] produces,
    /// so the live flag and the offline `procon-eval --kinds` axis share
    /// one grammar and one rejection set.
    pub agent_kinds: Option<Vec<EngineAgentKind>>,
    /// Choose the composition by simulating the match before `POST /agent`
    /// (live `--kinds-auto`, Issue #371): one Supply (slot 0), or no Supply
    /// when the simulation says the tanks last. Ignored when
    /// [`agent_kinds`](Self::agent_kinds) is `Some`.
    pub auto_kinds: bool,
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
    /// Optional shared slot the loop refreshes with the **live** wire
    /// `MatchState` at the start of every day (Phase 5, after `GET /`).
    /// Solvers that mirror the offline candidate model (see
    /// [`crate::candidate_bridge::CandidateBridge`]) clone this slot so
    /// each day's plan starts from the server's agent positions, fuel and
    /// traffic instead of day-0 defaults. `None` on every default path —
    /// injecting into a slot nobody reads cannot change behaviour.
    pub live_state_slot: Option<Arc<Mutex<Option<protocol::match_config::MatchState>>>>,
    /// Optional secret scrubbed out of snapshot error text before it is
    /// written (the arena token in arena mode). `None` outside arena runs;
    /// [`Secret`]'s redacted `Debug` keeps the token out of any render of
    /// this config either way.
    pub secret_redaction: Option<Secret>,
    /// Join a match that is already running instead of declaring kinds
    /// (Issue #385): when `GET /setting` shows `starts_at > 0` and `GET /`
    /// answers with a day, skip `POST /agent`, take the composition from the
    /// server's agent list and continue from the server's day.
    ///
    /// Off by default because it trusts one thing about the server: that a
    /// positive `starts_at` means the kind window is shut. The official
    /// server works that way (it publishes `starts_at` only after the kind
    /// deadline); a server that announces `starts_at` early and still expects
    /// `POST /agent` would be left without our kinds.
    pub resume_running_match: bool,
    /// Durable match record (`--record-dir`, Issue #394). Disabled by
    /// default, in which case every call on it is a no-op; when enabled the
    /// loop only ever hands it records through a non-blocking queue.
    pub recorder: MatchRecorder,
}

impl Default for CoordinatorLoopConfig {
    fn default() -> Self {
        Self {
            team_id: "mvp-player".to_owned(),
            setting_poll_interval: Duration::from_secs(2),
            setting_poll_max_attempts: 300,
            solver_name: "loop".to_owned(),
            agent_kinds: None,
            auto_kinds: false,
            kind_submit_max_attempts: 30,
            kind_submit_backoff: Duration::from_millis(500),
            match_state_max_attempts: 30,
            match_state_backoff: Duration::from_millis(500),
            state_output_path: None,
            server_url: None,
            days_total_hint: None,
            engine_setting_slot: None,
            live_state_slot: None,
            secret_redaction: None,
            resume_running_match: false,
            recorder: MatchRecorder::disabled(),
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
            setting_poll_max_attempts: 300,
            solver_name: "loop".to_owned(),
            // No env override for the composition: `--kinds` is a CLI flag
            // so an invocation records exactly what it asked for.
            agent_kinds: None,
            auto_kinds: false,
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
            live_state_slot: None,
            secret_redaction: None,
            resume_running_match: false,
            recorder: MatchRecorder::disabled(),
        })
    }
}

/// Aggregated result of a single match run.
///
/// Per-day evidence sits **alongside** the revisions (Issue #236): the
/// coordinator already sees everything a postmortem needs — the day-start
/// `GET /` state and the plan it submitted — so it is summarised here
/// instead of being re-fetchable only as a replay JSON download.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoordinatorOutcome {
    /// Number of days that completed (revision >= 0 returned).
    pub days_completed: usize,
    /// Per-day revision id (as returned by `POST /`).
    pub revisions: Vec<i32>,
    /// `GET /setting`'s `daySteps` — the per-day step budget, recorded once
    /// from the same setting fetch that drove the loop.
    pub day_steps: Vec<i32>,
    /// `GET /setting`'s `daySeconds` — the per-day answer window.
    pub day_seconds: Vec<i32>,
    /// One [`DaySummary`] per completed day (Issue #236).
    pub days: Vec<DaySummary>,
    /// The agent-kind composition this run actually declared in
    /// `POST /agent`, one entry per agent slot (Issue #243): what the
    /// server was told, expanded — so an arena evidence line can be
    /// compared with the offline `RunRecord.agent_kinds` that selected
    /// the candidate without replaying the match.
    ///
    /// Empty only when the run never reached Phase 2 (the match loop
    /// failed before `POST /agent`).
    pub kinds: Vec<EngineAgentKind>,
}

/// Day-start state of one of our agents, taken verbatim from the wire
/// `GET /` payload of that day (Issue #236).
///
/// `kind / pos / fuel` are game state, never account data — the same three
/// fields the spec's `AgentState` carries — so this is safe by construction
/// to persist in evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayAgentState {
    /// Agent kind as reported when the day opened (the kind we declared in
    /// `POST /agent`, echoed back by the server).
    pub kind: AgentKind,
    /// Cell index at day start (row-major linearisation of `Map.cells`).
    pub pos: i32,
    /// Fuel at day start — the first sample of this day's fuel timeline;
    /// consecutive days' values give the per-day fuel delta a postmortem
    /// needs (#233: diagnosing a fuel-exhaustion loss without replay JSON).
    pub fuel: i32,
}

/// Movement summary of the plan **we submitted** for one agent on one day
/// (Issue #236).
///
/// Deliberately a summary, never the raw answer body: a 9..=10-day plan
/// would cost hundreds of bytes per agent per day, while these four counters
/// carry what a postmortem actually asks — did the agent move at all, how
/// much of the day was spent waiting, and which direction it walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayMovement {
    /// Directional move actions (wire codes `0..=5`) — one count per element.
    pub moves: u32,
    /// Total wait steps: the summed length of the `-N` runs.
    pub waits: u32,
    /// Number of wait runs (negative entries), i.e. how often we idled.
    pub runs: u32,
    /// Move-direction histogram indexed by the official wire code
    /// (`0` 左上, `1` 右上, `2` 右, `3` 右下, `4` 左下, `5` 左).
    pub dirs: [u16; 6],
}

/// Per-day evidence for one played day (Issue #236): our agents' state at
/// day start plus a movement summary of the plan submitted that day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaySummary {
    /// 0-based day index — the wire `GET /` `day` of that day.
    pub day: u32,
    /// Our agents at day start, in the wire `GET /` order.
    pub agents: Vec<DayAgentState>,
    /// Movement summary of that day's submitted plan — one entry per agent
    /// of the answer body, in submission order. This matches `agents.len()`
    /// on a real server; a test double that omits `agents` shortens that
    /// side only, never this one.
    pub movement: Vec<DayMovement>,
}

impl DaySummary {
    /// Summarise one day from the wire day state and the plan we are about
    /// to submit. Pure and total: an out-of-range direction code (which the
    /// server can only reject anyway) is skipped rather than panicking, and
    /// a plan shorter than the agent list simply yields fewer summaries.
    #[must_use]
    pub fn new(day: u32, wire_agents: &[WireAgentState], plan: &ActionPlanBody) -> Self {
        Self {
            day,
            agents: wire_agents
                .iter()
                .map(|a| DayAgentState { kind: a.kind, pos: a.pos, fuel: a.fuel })
                .collect(),
            movement: plan.iter().map(|actions| summarise_actions(actions)).collect(),
        }
    }
}

/// Fold one agent's wire plan into a [`DayMovement`] (see the field docs
/// for the counting rules).
#[must_use]
fn summarise_actions(actions: &[i32]) -> DayMovement {
    let mut m = DayMovement { moves: 0, waits: 0, runs: 0, dirs: [0; 6] };
    for &code in actions {
        if code <= -1 {
            m.waits = m.waits.saturating_add(code.unsigned_abs());
            m.runs = m.runs.saturating_add(1);
        } else if let Some(idx) = usize::try_from(code).ok().filter(|c| *c < 6) {
            m.moves = m.moves.saturating_add(1);
            m.dirs[idx] = m.dirs[idx].saturating_add(1);
        }
        // `code >= 6` is not a legal wire action; skip it silently — the
        // server's revision carries the verdict and evidence building must
        // never panic on a payload.
    }
    m
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
    // The run directory keeps its own copy of the snapshot (Issue #394),
    // written by the recorder thread; `--state-output` below is unchanged.
    if cfg.recorder.is_enabled() {
        if let Ok(json) = serde_json::to_vec_pretty(snap) {
            cfg.recorder.state_snapshot(json);
        }
    }
    let Some(path) = cfg.state_output_path.as_ref() else {
        return;
    };
    // Arena mode may register a secret (the arena token). Server-provided
    // error text lands in `snap.error`, so scrub the secret before this
    // durable artifact can contain it.
    let redacted;
    let payload: &MatchStateSnapshot = match cfg.secret_redaction.as_ref() {
        Some(secret) => {
            let mut copy = snap.clone();
            copy.error = copy.error.map(|e| secret.redact(&e));
            redacted = copy;
            &redacted
        }
        None => snap,
    };
    let Ok(json) = serde_json::to_vec_pretty(payload) else {
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
    /// `GET /setting` never produced a usable snapshot within the poll
    /// budget (`setting_poll_max_attempts`).
    #[error("`GET /setting` was not ready after {attempts} attempts during the {phase} phase")]
    SettingPollTimeout {
        /// Which poll loop ran out: `"phase-1 probe"` or `"phase-4 start wait"`.
        phase: &'static str,
        /// Polls issued before giving up.
        attempts: u32,
    },
    /// Protocol <-> engine conversion produced a malformed value.
    #[error("protocol/engine conversion failed: {0}")]
    Conversion(String),
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

    cfg.recorder.event_with(|| match &result {
        Ok(outcome) => RecordEvent::MatchEnd {
            status: "completed".to_owned(),
            days_completed: outcome.days_completed,
            revisions: outcome.revisions.clone(),
            error: None,
        },
        Err(e) => RecordEvent::MatchEnd {
            status: if matches!(e, CoordinatorLoopError::Rejected { .. }) {
                "rejected".to_owned()
            } else {
                "error".to_owned()
            },
            days_completed: snap.revisions.len(),
            revisions: snap.revisions.clone(),
            error: Some(e.to_string()),
        },
    });
    cfg.recorder.report_health();

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
            tracing::error!(day, revision, "plan rejected by server");
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
    let mut setting_polls = 0_u32;
    let wire_setting = loop {
        match client.get_match_setting().await {
            Ok(s) => {
                cfg.recorder.setting(&s);
                cfg.recorder
                    .event(RecordEvent::Setting { starts_at: s.starts_at, polls: setting_polls });
                break s;
            }
            Err(protocol::HttpClientError::AccessTime { .. }) => {
                // Pre-start window: server returns 403 before the first
                // setting snapshot exists. Back off and retry — but not
                // forever, or an unattended run waits on a server that
                // never comes up with nothing in the log to show for it.
                setting_polls += 1;
                if setting_polls >= cfg.setting_poll_max_attempts {
                    return Err(CoordinatorLoopError::SettingPollTimeout {
                        phase: "phase-1 probe",
                        attempts: setting_polls,
                    });
                }
                tracing::debug!(
                    attempt = setting_polls,
                    max = cfg.setting_poll_max_attempts,
                    "GET /setting still 403; waiting for the first snapshot"
                );
                tokio::time::sleep(cfg.setting_poll_interval).await;
            }
            Err(e) => return Err(CoordinatorLoopError::Setting(e)),
        }
    };

    // Update the Waiting snapshot now that Phase 1 has revealed the
    // real day count — this is also when we flip to `Running` after
    // Phase 2 (kinds accepted) below.
    snap.days_total = u32::try_from(wire_setting.day_steps.len()).unwrap_or(0);

    // Phase 2: declare agent kinds. An explicit `--kinds` composition
    // (`cfg.agent_kinds`) is submitted verbatim for **every** solver;
    // without one the historical rule applies — real solvers alternate
    // KIND_0 / KIND_1 so half the agents patrol and half supply, and the
    // debug `WaitSolver` keeps the MVP's all-`KIND_0` placeholder so the
    // wire shape stays degenerate and trivially debuggable.
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
    // Resume (Issue #385): a positive `starts_at` means the kind window is
    // shut, so if `GET /` already answers with a day, this process was started
    // (or restarted after a crash) in the middle of a match. `POST /agent`
    // can only answer 403 from here on, and the server's own agent list says
    // which composition is in play — pick the match up at the day it is on
    // instead of dying on the kind submission.
    let mut resumed_state = if cfg.resume_running_match && wire_setting.starts_at > 0 {
        match client.get_match_state().await {
            Ok(state) if state.agents.len() == n_agents => Some(state),
            _ => None,
        }
    } else {
        None
    };
    let resumed = resumed_state.is_some();
    let simulated_kinds = cfg.auto_kinds && cfg.agent_kinds.is_none();
    let submitted_kinds = if let Some(state) = resumed_state.as_ref() {
        let kinds: Vec<EngineAgentKind> = state
            .agents
            .iter()
            .map(|a| {
                if a.kind == AgentKind::KIND_1 {
                    EngineAgentKind::Supply
                } else {
                    EngineAgentKind::Patrol
                }
            })
            .collect();
        tracing::warn!(
            day = state.day,
            kinds = %bench::kinds::render_kinds(&kinds),
            "match already in progress: resuming with the composition the server reports"
        );
        cfg.recorder.event(RecordEvent::Resume { day: state.day });
        kinds
    } else if simulated_kinds {
        let team = PlayerId::new(&cfg.team_id)
            .map_err(|e| CoordinatorLoopError::Conversion(format!("invalid team id: {e}")))?;
        let (kinds, trials) =
            bench::adapters::fleet_solver::choose_kinds(&wire_setting, &team, AUTO_KINDS_WINDOW);
        for trial in &trials {
            tracing::info!(
                kinds = %bench::kinds::render_kinds(&trial.kinds),
                variety = trial.score.0,
                daily_variety = trial.score.1,
                balls = trial.score.2,
                "agent kinds trial"
            );
        }
        kinds
    } else {
        agent_kinds_for(n_agents, &cfg.solver_name, cfg.agent_kinds.as_deref())
    };
    cfg.recorder.event_with(|| RecordEvent::Kinds {
        source: if resumed {
            "server"
        } else if cfg.agent_kinds.is_some() {
            "explicit"
        } else if simulated_kinds {
            "auto"
        } else {
            "default"
        }
        .to_owned(),
        kinds: to_wire_kinds(&submitted_kinds).iter().map(|k| k.value()).collect(),
        pattern: bench::kinds::render_kinds(&submitted_kinds),
    });
    if !resumed {
        // Evidence (Issue #243): log the composition being declared in the same
        // `P,S,…` spelling `--kinds` takes, so a run log and the flag that
        // produced it read the same words. Agent kinds are game state, never
        // secrets — no token material reaches this line.
        tracing::info!(
            n_agents,
            kinds = %bench::kinds::render_kinds(&submitted_kinds),
            solver = %cfg.solver_name,
            "agent kinds selected for POST /agent"
        );
        let agent_submit_began = std::time::Instant::now();
        let submitted = submit_agent_with_retry(
            &client,
            to_wire_kinds(&submitted_kinds),
            cfg.kind_submit_max_attempts,
            cfg.kind_submit_backoff,
        )
        .await;
        cfg.recorder.event_with(|| RecordEvent::AgentSubmit {
            ok: submitted.is_ok(),
            elapsed_ms: elapsed_ms(agent_submit_began),
            error: submitted.as_ref().err().map(ToString::to_string),
        });
        submitted?;
    }

    // Kinds accepted -> Running snapshot. `day` stays `None` because
    // we have not yet entered the per-day driver.
    snap.status = MatchStatus::Running;
    snap.day = None;
    snap.last_update_unix = unix_now();
    write_snapshot(&cfg, snap);

    // Phase 3: convert wire -> engine setting, build team + match ids.
    //
    // An explicit `--kinds` feeds the composition just declared into the
    // conversion: the planning model (patrol fuel caps above all) must
    // describe the team the server was told about, so the flag changes
    // `POST /agent` and the solver's world together. Without the flag this
    // is the historical call, verbatim — including its `wait` quirk, where
    // `POST /agent` sends all patrol while the local model keeps the
    // alternation; `WaitSolver` never plans against that model. A resumed
    // match always plans against the composition the server reported.
    let team = PlayerId::new(&cfg.team_id)
        .map_err(|e| CoordinatorLoopError::Conversion(format!("invalid team id: {e}")))?;
    let engine_setting = if cfg.agent_kinds.is_some() || simulated_kinds || resumed {
        protocol_setting_to_engine_with_kinds(
            &wire_setting,
            team.clone(),
            Some(submitted_kinds.as_slice()),
        )
        .map_err(CoordinatorLoopError::Conversion)?
    } else {
        protocol_setting_to_engine(&wire_setting, team.clone())
            .map_err(CoordinatorLoopError::Conversion)?
    };
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
    if wire_setting.starts_at > 0 {
        cfg.recorder.event(RecordEvent::MatchStart { starts_at: wire_setting.starts_at });
    }
    if wire_setting.starts_at == 0 {
        let mut start_polls = 0_u32;
        loop {
            match client.get_match_setting().await {
                Ok(s) if s.starts_at > 0 => {
                    cfg.recorder.event(RecordEvent::MatchStart { starts_at: s.starts_at });
                    break;
                }
                Ok(_) | Err(protocol::HttpClientError::AccessTime { .. }) => {
                    // Kinds are registered but the match has not announced a
                    // start yet. Bounded, and visible: a cancelled match or a
                    // drifted `starts_at` must not hang an arena run silently.
                    start_polls += 1;
                    if start_polls >= cfg.setting_poll_max_attempts {
                        return Err(CoordinatorLoopError::SettingPollTimeout {
                            phase: "phase-4 start wait",
                            attempts: start_polls,
                        });
                    }
                    tracing::debug!(
                        attempt = start_polls,
                        max = cfg.setting_poll_max_attempts,
                        "waiting for `starts_at` to flip positive"
                    );
                    tokio::time::sleep(cfg.setting_poll_interval).await;
                }
                Err(e) => return Err(CoordinatorLoopError::Setting(e)),
            }
        }
    }

    let mut revisions: Vec<i32> = Vec::with_capacity(total_days);
    // Issue #236 — per-day evidence collected next to the revisions, from
    // the very same `GET /` state and submitted plan the loop already has.
    let mut day_summaries: Vec<DaySummary> = Vec::with_capacity(total_days);
    let mut days_completed = 0_usize;
    // Days that ended without an accepted plan, and the first rejection the
    // server reported. Neither ends the match (Issue #385): one lost day
    // costs that day's harvest, abandoning the loop costs every later day.
    let mut unanswered_days: Vec<u32> = Vec::new();
    let mut first_rejection: Option<(u32, i32)> = None;
    let solver: Arc<dyn engine::solver::Solver> = Arc::from(solver);
    let total_days_i32 = i32::try_from(total_days).unwrap_or(i32::MAX);

    // Phase 5: per-day loop, driven by the day the server reports.
    let mut expected_day = resumed_state.as_ref().map_or(0, |s| s.day.max(0));
    while expected_day < total_days_i32 {
        let fetch_began = std::time::Instant::now();
        let state = if let Some(state) = resumed_state.take() {
            state
        } else {
            // Keep asking for as long as the match can still be running:
            // a cable that is back two days later still gets the rest of
            // the match answered.
            let remaining_seconds: i64 = wire_setting
                .day_seconds
                .iter()
                .skip(usize::try_from(expected_day).unwrap_or(0))
                .map(|s| i64::from(*s))
                .sum();
            let patience = Duration::from_secs(u64::try_from(remaining_seconds).unwrap_or(0))
                + cfg.match_state_backoff * cfg.match_state_max_attempts.max(1);
            fetch_state_with_retry(
                &client,
                patience,
                cfg.match_state_backoff,
                expected_day,
                &cfg.recorder,
            )
            .await?
        };
        if state.day >= total_days_i32 {
            break;
        }
        cfg.recorder.event_with(|| RecordEvent::DayState {
            expected_day,
            fetch_ms: elapsed_ms(fetch_began),
            observed: state.clone(),
        });
        if state.day > expected_day {
            tracing::error!(
                expected_day,
                server_day = state.day,
                "the server is ahead of us: the days in between went unanswered"
            );
            unanswered_days.extend((expected_day..state.day).filter_map(|d| u32::try_from(d).ok()));
            cfg.recorder.event_with(|| RecordEvent::DaysSkipped {
                days: (expected_day..state.day).filter_map(|d| u32::try_from(d).ok()).collect(),
            });
        }
        let day_idx = usize::try_from(state.day).unwrap_or(0);
        let day = u32::try_from(state.day).unwrap_or(0);
        let day_steps = u32::try_from(wire_setting.day_steps[day_idx])
            .map_err(|e| CoordinatorLoopError::Conversion(format!("day_steps[{day_idx}]={e}")))?;
        expected_day = state.day + 1;

        // Hand the live wire state to solvers that registered a slot
        // (candidate bridge). Default path: no slot, nothing happens.
        if let Some(slot) = cfg.live_state_slot.as_ref() {
            let mut guard = slot.lock().map_err(|_| {
                CoordinatorLoopError::Conversion("live state slot mutex poisoned".to_owned())
            })?;
            *guard = Some(state.clone());
        }

        let snapshot =
            match_state_to_snapshot(&state, &match_id, &team).unwrap_or_else(dummy_match_snapshot);
        let now_ms = Utc::now().timestamp_millis();
        let remaining_ms = solver_budget_ms(state.ends_at, now_ms);
        let ctx = engine::solver::SolverContext::new(
            snapshot,
            team.clone(),
            agent_ids.clone(),
            day,
            day_steps,
            remaining_ms,
        );

        // The solver runs off the async thread under a watchdog that fires
        // `SUBMIT_RESERVE_MS` before the server's deadline: a plan that is not
        // ready by then could only arrive after the day has rolled over, where
        // it would be judged against the next day's positions.
        let watchdog = Duration::from_millis(
            u64::try_from(
                state.ends_at.saturating_mul(1000).saturating_sub(now_ms) - SUBMIT_RESERVE_MS,
            )
            .unwrap_or(0)
            .max(MIN_SOLVER_WINDOW_MS),
        );
        let planned = {
            let solver = Arc::clone(&solver);
            let engine_setting = engine_setting.clone();
            let state = state.clone();
            let task = tokio::task::spawn_blocking(move || {
                let decision = solver.decide(&ctx);
                let actions = decision_to_actions(&decision, &ctx)
                    .map_err(|e| format!("action adapter: {e}"))?;
                // Issue #226 — official conformance at the submission boundary.
                // The server judges a *terrain-weighted* step sum
                // (Σ origin-cell costs + waits == daySteps, see
                // `engine::wire_plan`), while candidates and
                // `decision_to_actions` speak the engine's element clock.
                // Normalize here: both solver paths (CandidateBridge and
                // EngineLoopAdapter) funnel through this single point, so the
                // body we actually POST always fits the official budget.
                normalize_submission(&engine_setting, &state, day_steps, &actions)
            });
            match tokio::time::timeout(watchdog, task).await {
                Ok(Ok(result)) => result,
                Ok(Err(join)) => Err(format!("solver task failed: {join}")),
                Err(_) => Err(format!("no plan within {watchdog:?} of the answer window")),
            }
        };
        let actions = match planned {
            Ok(actions) => actions,
            Err(reason) => {
                tracing::error!(day, reason = %reason, "no plan for this day; leaving it unanswered");
                cfg.recorder.event_with(|| RecordEvent::PlanFailed {
                    day,
                    solve_ms: millis_since(now_ms),
                    reason: reason.clone(),
                });
                cfg.recorder.event_with(|| RecordEvent::DayResult {
                    day,
                    status: DayStatus::Unanswered,
                    revision: None,
                    attempts: 0,
                    answer_ms: Utc::now().timestamp_millis() - now_ms,
                    margin_ms: state.ends_at.saturating_mul(1000) - Utc::now().timestamp_millis(),
                    reason: Some(reason.clone()),
                });
                cfg.recorder.report_health();
                unanswered_days.push(day);
                sleep_until_day_end(&state, expected_day < total_days_i32).await;
                continue;
            }
        };

        // Issue #236 — summarise the day *before* `actions` is moved into
        // the submit call: day-start agent state (kind / pos / fuel) plus
        // the movement shape of the normalized plan we are about to send.
        let summary = DaySummary::new(day, &state.agents, &actions);
        // Issue #394 — the full body, not a summary: this is the one record
        // a route can be redrawn from. Queued before the POST so a crash
        // during delivery still leaves what was about to be sent.
        cfg.recorder.event_with(|| RecordEvent::Plan {
            day,
            day_steps,
            budget_ms: remaining_ms,
            solve_ms: millis_since(now_ms),
            actions: actions.clone(),
        });

        // Operator visibility: when DEBUG logging is on, dump the
        // exact wire payload we are about to send. Gated by the
        // `tracing::enabled!` check so the per-step formatting cost
        // stays out of the hot path in production.
        //
        // The `traffics` snapshot rides along (Issue #245): the body's
        // weighted sum only makes sense next to the road states the
        // normalizer billed, and a live `revision=-1` is otherwise
        // unexplainable from the log alone. Only non-smooth roads are
        // spelled out — the rest bill 1 either way — and the entry count
        // records how complete the payload was.
        if tracing::enabled!(tracing::Level::DEBUG) {
            let non_smooth: Vec<(i32, i32)> = state
                .traffics
                .iter()
                .filter(|t| t.status.value() != 0)
                .map(|t| (t.pos, t.status.value()))
                .collect();
            tracing::debug!(
                day,
                traffic_entries = state.traffics.len(),
                non_smooth = ?non_smooth,
                body = ?actions,
                "submitting plan"
            );
        }

        let submitted_at_ms = Utc::now().timestamp_millis();
        let (submission, attempts) =
            submit_plan_until_deadline(&client, &actions, state.ends_at, day, &cfg.recorder)
                .await?;
        cfg.recorder.event_with(|| {
            let (status, revision, reason) = match &submission {
                DaySubmission::Accepted(revision) => (DayStatus::Accepted, Some(*revision), None),
                DaySubmission::Rejected(revision) => (DayStatus::Rejected, Some(*revision), None),
                DaySubmission::Unanswered(reason) => {
                    (DayStatus::Unanswered, None, Some(reason.clone()))
                }
            };
            RecordEvent::DayResult {
                day,
                status,
                revision,
                attempts,
                answer_ms: Utc::now().timestamp_millis() - now_ms,
                margin_ms: state.ends_at.saturating_mul(1000) - Utc::now().timestamp_millis(),
                reason,
            }
        });
        cfg.recorder.report_health();
        match submission {
            DaySubmission::Accepted(revision) => {
                tracing::info!(
                    day,
                    revision,
                    answer_ms = Utc::now().timestamp_millis() - now_ms,
                    submit_ms = Utc::now().timestamp_millis() - submitted_at_ms,
                    margin_ms = state.ends_at.saturating_mul(1000) - Utc::now().timestamp_millis(),
                    "plan accepted by server"
                );
                revisions.push(revision);
                day_summaries.push(summary);
                days_completed += 1;
                // Per-day Running snapshot. The closure mutates `snap` and
                // calls `write_snapshot` so the frontend sees incremental
                // progress after each accepted day.
                on_day_accepted(day, revision, snap);
            }
            DaySubmission::Rejected(revision) => {
                tracing::error!(
                    day,
                    revision,
                    "plan rejected by server; continuing with the next day"
                );
                first_rejection.get_or_insert((day, revision));
                unanswered_days.push(day);
            }
            DaySubmission::Unanswered(reason) => {
                tracing::error!(day, reason = %reason, "plan not delivered; continuing with the next day");
                unanswered_days.push(day);
            }
        }

        sleep_until_day_end(&state, expected_day < total_days_i32).await;
    }

    if !unanswered_days.is_empty() {
        tracing::error!(
            days = ?unanswered_days,
            "match finished with days that have no accepted plan"
        );
    }
    if let Some((day, revision)) = first_rejection {
        return Err(CoordinatorLoopError::Rejected { day, revision });
    }

    Ok(CoordinatorOutcome {
        days_completed,
        revisions,
        day_steps: wire_setting.day_steps.clone(),
        day_seconds: wire_setting.day_seconds.clone(),
        days: day_summaries,
        kinds: submitted_kinds,
    })
}

/// Time kept back from the answer window for delivering the plan: the solver
/// watchdog fires this long before the server's deadline.
const SUBMIT_RESERVE_MS: i64 = 2500;

/// The watchdog never gives the solver less than this, however little of the
/// day is left (a late resume still gets one honest attempt).
const MIN_SOLVER_WINDOW_MS: u64 = 500;

/// Pause between `POST /` attempts while a plan is not through yet.
const SUBMIT_RETRY_PAUSE: Duration = Duration::from_millis(200);

/// What became of one day's plan.
enum DaySubmission {
    /// The server answered `revision >= 0`.
    Accepted(i32),
    /// The server answered `revision < 0`: the plan is illegal.
    Rejected(i32),
    /// No verdict before the day ended.
    Unanswered(String),
}

/// Sleep to the day's deadline, unless this was the last day.
async fn sleep_until_day_end(state: &WireMatchState, more_days: bool) {
    if more_days {
        let diff = state.ends_at - Utc::now().timestamp();
        if diff > 0 {
            tokio::time::sleep(Duration::from_secs(u64::try_from(diff).unwrap_or(0))).await;
        }
    }
}

/// `POST /` one day's plan until the server gives a verdict or the day ends.
///
/// Sending the same plan again is safe by the official contract: within a day
/// the submission with the highest revision stands, so a repeat can only
/// replace the plan with itself. That makes "the connection dropped, was it
/// accepted?" answerable the simple way — ask again — while the answer window
/// is still open. The window is the limit: a repeat that lands after the day
/// has rolled over would be read as the *next* day's plan, so nothing is sent
/// once the local clock passes `ends_at`.
///
/// A rejection (`revision < 0`) is a verdict on the plan, not a delivery
/// problem, and is returned at once without another attempt.
///
/// # Errors
/// Only an authentication failure — the one error no later attempt and no
/// later day can recover from.
async fn submit_plan_until_deadline(
    client: &protocol::MatchClient,
    actions: &ActionPlanBody,
    ends_at: i64,
    day: u32,
    recorder: &MatchRecorder,
) -> Result<(DaySubmission, u32), CoordinatorLoopError> {
    let deadline_ms = ends_at.saturating_mul(1000);
    let mut attempt = 0_u32;
    loop {
        attempt += 1;
        let began = std::time::Instant::now();
        let response = client.submit_actions(actions.clone()).await;
        let attempted = |outcome: SubmitOutcome, revision: Option<i32>, error: Option<String>| {
            recorder.event_with(|| RecordEvent::SubmitAttempt {
                day,
                attempt,
                elapsed_ms: elapsed_ms(began),
                outcome,
                revision,
                error,
            });
        };
        let error = match response {
            Ok(accepted) => {
                attempted(SubmitOutcome::Accepted, Some(accepted.revision), None);
                return Ok((DaySubmission::Accepted(accepted.revision), attempt));
            }
            Err(protocol::HttpClientError::Http { status: 200, message })
                if message.starts_with("rejected: revision=") =>
            {
                let revision =
                    message.trim_start_matches("rejected: revision=").parse::<i32>().unwrap_or(-1);
                attempted(SubmitOutcome::Rejected, Some(revision), Some(message));
                return Ok((DaySubmission::Rejected(revision), attempt));
            }
            Err(
                e @ (protocol::HttpClientError::Auth { .. }
                | protocol::HttpClientError::InvalidUrl(_)),
            ) => {
                attempted(SubmitOutcome::Fatal, None, Some(e.to_string()));
                return Err(CoordinatorLoopError::ActionSubmit(e));
            }
            Err(protocol::HttpClientError::AccessTime { message }) => {
                attempted(SubmitOutcome::WindowClosed, None, Some(message.clone()));
                return Ok((
                    DaySubmission::Unanswered(format!("answer window closed: {message}")),
                    attempt,
                ));
            }
            Err(protocol::HttpClientError::Http { status, message })
                if (400..500).contains(&status) && !matches!(status, 408 | 425 | 429) =>
            {
                let reason = format!("HTTP {status}: {message}");
                attempted(SubmitOutcome::ClientError, None, Some(reason.clone()));
                return Ok((DaySubmission::Unanswered(reason), attempt));
            }
            Err(e) => {
                attempted(SubmitOutcome::NoVerdict, None, Some(e.to_string()));
                e
            }
        };
        let left_ms = deadline_ms - Utc::now().timestamp_millis();
        if left_ms <= i64::try_from(SUBMIT_RETRY_PAUSE.as_millis()).unwrap_or(i64::MAX) {
            return Ok((
                DaySubmission::Unanswered(format!(
                    "no verdict after {attempt} attempt(s): {error}"
                )),
                attempt,
            ));
        }
        tracing::warn!(day, attempt, left_ms, error = %error, "POST / got no verdict; sending the same plan again");
        tokio::time::sleep(SUBMIT_RETRY_PAUSE).await;
    }
}

/// Normalize one day's answer body to the official terrain-weighted step
/// budget (Issue #226).
///
/// Builds the live engine state the plan will be judged against: the static
/// setting overlaid with the current `GET /` payload — road traffic always
/// (it changes the step cost of a road cell), agent positions and fuel when
/// the server reports them (the real server always does; some test doubles
/// omit `agents`, in which case the match-start setting stands in).
///
/// This is the single choke point both solver paths flow through
/// (`CandidateBridge::decide` and `EngineLoopAdapter` decisions both arrive
/// here via [`decision_to_actions`]); see [`engine::wire_plan`] for the rule
/// itself and the localhost probe evidence behind it.
fn normalize_submission(
    engine_setting: &EngineSetting,
    wire_state: &WireMatchState,
    day_steps: u32,
    body: &ActionPlanBody,
) -> Result<ActionPlanBody, String> {
    // Live view: static setting + today's overlay. Built from the bare
    // setting (no `MatchState::new`) so test-double settings that skip the
    // 3..=8 agent rule still normalize — the plan rules themselves live in
    // `engine::wire_plan`.
    let mut setting = engine_setting.clone();
    apply_live_traffic(&mut setting.map, wire_state)
        .map_err(|e| format!("traffic overlay: {e}"))?;
    if wire_state.agents.len() == setting.agents.len() {
        apply_live_agents(&mut setting.agents, setting.map.size, wire_state)
            .map_err(|e| format!("agent overlay: {e}"))?;
    } else {
        tracing::debug!(
            declared = setting.agents.len(),
            reported = wire_state.agents.len(),
            "live state carries no agent entries; normalizing from match-start positions"
        );
    }
    let ids = engine_setting.agent_ids();
    engine::wire_plan::normalize_day_plan(&setting.map, &setting.agents, &ids, day_steps, body)
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
    retry_on_access_time(
        "POST /agent",
        max_attempts,
        backoff,
        "kind window did not open within retry budget",
        || client.submit_agent(kinds.clone()),
    )
    .await
    .map_err(CoordinatorLoopError::AgentSubmit)
}

/// Bounded retry on `403 AccessTimeError` — the shared semantics fixed in
/// PR #165 for the coordinator's pre-start windows (issue #156 kind window,
/// issue #164 `-match-start-delay`).
///
/// `op` is attempted up to `max_attempts` times, sleeping `backoff` between
/// attempts. An [`protocol::HttpClientError::AccessTime`] means "the window
/// is not open yet / already closed" and is retried; **every other error is
/// propagated immediately** (auth, drift, transport — those must not be
/// retried blindly). When the budget runs out the last `AccessTime`
/// message is surfaced, or `exhausted_hint` if no attempt ran at all.
///
/// This is the *one* `AccessTime` retry path in the CLI: the arena's
/// pre-match registration phase reuses it instead of inventing a second
/// retry loop (Issue #179).
pub async fn retry_on_access_time<T, F, Fut>(
    what: &str,
    max_attempts: u32,
    backoff: Duration,
    exhausted_hint: &str,
    mut op: F,
) -> Result<T, protocol::HttpClientError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, protocol::HttpClientError>>,
{
    let mut last_access_time: Option<String> = None;
    for attempt in 0..max_attempts {
        match op().await {
            Ok(value) => return Ok(value),
            Err(protocol::HttpClientError::AccessTime { message }) => {
                tracing::debug!(attempt, what, message = %message, "403 AccessTime, retrying");
                last_access_time = Some(message);
                if attempt + 1 < max_attempts {
                    tokio::time::sleep(backoff).await;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(protocol::HttpClientError::AccessTime {
        message: last_access_time.unwrap_or_else(|| exhausted_hint.to_owned()),
    })
}

/// Poll `GET /` until the server reports `expected_day` or a later one.
///
/// Everything short of a verdict on our credentials is waited out, for up to
/// `patience`: the `403 AccessTimeError` of the `-match-start-delay` gap
/// (issue #164), a server still on the previous day because its clock runs
/// behind ours, and — Issue #385 — transport failures, 5xx answers and bodies
/// that do not parse. A venue switch that drops for ten seconds must cost at
/// most the day it dropped in, not the match.
///
/// The caller sizes `patience` to the time the match can still be running.
///
/// # Errors
/// An authentication failure at once; otherwise a `Conversion` error naming
/// the last obstacle when `patience` runs out.
async fn fetch_state_with_retry(
    client: &protocol::MatchClient,
    patience: Duration,
    backoff: Duration,
    expected_day: i32,
    recorder: &MatchRecorder,
) -> Result<protocol::match_config::MatchState, CoordinatorLoopError> {
    let started = tokio::time::Instant::now();
    loop {
        let obstacle = match client.get_match_state().await {
            Ok(state) if state.day >= expected_day => return Ok(state),
            Ok(state) => format!("server is still on day {}", state.day),
            Err(
                e @ (protocol::HttpClientError::Auth { .. }
                | protocol::HttpClientError::InvalidUrl(_)),
            ) => return Err(CoordinatorLoopError::State(e)),
            Err(protocol::HttpClientError::AccessTime { message }) => message,
            Err(e) => {
                tracing::warn!(expected_day, error = %e, "GET / failed; asking again");
                recorder.event_with(|| RecordEvent::RequestFailed {
                    endpoint: "GET /".to_owned(),
                    day: Some(expected_day),
                    error: e.to_string(),
                });
                e.to_string()
            }
        };
        if started.elapsed() >= patience {
            return Err(CoordinatorLoopError::Conversion(format!(
                "server did not advance to day {expected_day} within polling budget ({obstacle})"
            )));
        }
        tokio::time::sleep(backoff).await;
    }
}

/// Milliseconds since `began`, for the match record.
fn elapsed_ms(began: std::time::Instant) -> u64 {
    u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Milliseconds of wall clock since `then_ms` (UNIX milliseconds).
fn millis_since(then_ms: i64) -> u64 {
    u64::try_from(Utc::now().timestamp_millis().saturating_sub(then_ms)).unwrap_or(0)
}

/// Reserve one request timeout for submission after the solver finishes.
fn solver_budget_ms(ends_at: i64, now_ms: i64) -> u64 {
    let deadline_ms = ends_at.saturating_mul(1000);
    u64::try_from(deadline_ms.saturating_sub(now_ms).saturating_sub(5000)).unwrap_or(0)
}

/// How long `--kinds-auto` may simulate before `POST /agent`. A missed kind
/// deadline ends the run (`POST /agent` only ever answers 403 afterwards), and
/// the official server's default window is 5 s from its start, so this stays
/// far inside it; trials that are not done by then fall back to one Supply.
const AUTO_KINDS_WINDOW: Duration = Duration::from_millis(1500);

/// Resolve the `POST /agent` composition for a team of `n_agents`.
///
/// The result is in engine spelling ([`EngineAgentKind`]); pass it through
/// [`to_wire_kinds`] to get the `0 | 1` body the server accepts.
///
/// - **Explicit pattern** (live `--kinds <pattern>`, Issues #238/#243):
///   the pattern wins for **every** solver — `wait` / `wait-mvp`
///   included — cycled over the agent slots by
///   [`bench::kinds::resolve_kinds`], exactly like the offline
///   `procon-eval --kinds` axis. The operator asked for a composition, so
///   the composition is what goes on the wire.
/// - **No pattern** (flag omitted): today's rule, byte for byte —
///   `"wait"` / `"wait-mvp"` submit all patrol so the debug solver's
///   degenerate contract stays observable, and anything else (including
///   the default `"loop"`) alternates patrol/supply. The alternation comes
///   from [`engine::from_wire::default_agent_kinds`], the same function
///   the offline evaluator's default expands through, so the two defaults
///   cannot drift.
///
/// The `wait` special case is therefore *only* a no-`--kinds` behaviour;
/// an explicit pattern overrides it (documented on
/// [`CoordinatorLoopConfig::agent_kinds`] and in the `--kinds` flag help).
fn agent_kinds_for(
    n_agents: usize,
    solver_name: &str,
    pattern: Option<&[EngineAgentKind]>,
) -> Vec<EngineAgentKind> {
    match pattern {
        Some(pattern) if !pattern.is_empty() => {
            bench::kinds::resolve_kinds(Some(pattern), n_agents)
        }
        _ => {
            if solver_name == "wait" || solver_name == "wait-mvp" {
                vec![EngineAgentKind::Patrol; n_agents]
            } else {
                engine::from_wire::default_agent_kinds(n_agents)
            }
        }
    }
}

/// Map a resolved composition onto the wire's `0 | 1` spelling for
/// `POST /agent`: `Patrol` → `KIND_0`, `Supply` → `KIND_1`.
///
/// This is the mapping the coordinator has always used implicitly (even
/// slot → `KIND_0` → patrol, the `OpenAPI` `[0, 1, 0, 1]` example), and it
/// is what makes the omitted `--kinds` byte-identical to today's body —
/// pinned by `default_is_the_historical_alternation_for_every_team_size`.
fn to_wire_kinds(kinds: &[EngineAgentKind]) -> Vec<AgentKind> {
    kinds
        .iter()
        .map(|&kind| match kind {
            EngineAgentKind::Patrol => AgentKind::KIND_0,
            EngineAgentKind::Supply => AgentKind::KIND_1,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire body for one team size, spelled out the way the pre-#243
    /// coordinator did: even slot `KIND_0`, odd slot `KIND_1`.
    fn historical_alternation(n: usize) -> Vec<AgentKind> {
        (0..n).map(|i| if i % 2 == 0 { AgentKind::KIND_0 } else { AgentKind::KIND_1 }).collect()
    }

    fn pattern(spec: &str) -> Vec<EngineAgentKind> {
        bench::kinds::parse_kinds(spec).unwrap_or_else(|e| panic!("{spec}: {e}"))
    }

    #[test]
    fn solver_budget_uses_absolute_deadline_and_submission_margin() {
        assert_eq!(solver_budget_ms(100, 90_000), 5_000);
        assert_eq!(solver_budget_ms(100, 96_000), 0);
        assert_eq!(solver_budget_ms(100, 101_000), 0);
    }

    #[tokio::test]
    async fn previous_day_state_is_polled_until_expected_day() {
        let mut server = mockito::Server::new_async().await;
        let old = server
            .mock("GET", "/")
            .with_status(200)
            .with_body(r#"{"endsAt":100,"day":0,"agents":[],"others":[],"traffics":[]}"#)
            .expect(1)
            .create_async()
            .await;
        let new = server
            .mock("GET", "/")
            .with_status(200)
            .with_body(r#"{"endsAt":200,"day":1,"agents":[],"others":[],"traffics":[]}"#)
            .expect(1)
            .create_async()
            .await;
        let client = protocol::MatchClient::new(server.url(), "test").unwrap();
        let state = fetch_state_with_retry(
            &client,
            Duration::from_secs(5),
            Duration::ZERO,
            1,
            &MatchRecorder::disabled(),
        )
        .await
        .unwrap();
        assert_eq!(state.day, 1);
        old.assert_async().await;
        new.assert_async().await;
    }

    #[tokio::test]
    async fn previous_day_polling_is_bounded_and_future_days_are_not_retried() {
        for (reported, succeeds) in [(0, false), (2, true)] {
            let mut server = mockito::Server::new_async().await;
            let mock = server
                .mock("GET", "/")
                .with_status(200)
                .with_body(format!(
                    r#"{{"endsAt":200,"day":{reported},"agents":[],"others":[],"traffics":[]}}"#
                ))
                .expect(1)
                .create_async()
                .await;
            let client = protocol::MatchClient::new(server.url(), "test").unwrap();
            let result = fetch_state_with_retry(
                &client,
                Duration::ZERO,
                Duration::ZERO,
                1,
                &MatchRecorder::disabled(),
            )
            .await;
            assert_eq!(result.is_ok(), succeeds);
            if let Ok(state) = result {
                assert!(state.day > 1, "the caller follows the server to the later day");
            }
            mock.assert_async().await;
        }
    }

    /// Issue #385: a body that does not parse and a 503 are waited out like a
    /// 403, so a flaky venue link costs polls, not the match.
    #[tokio::test]
    async fn state_polling_outlasts_garbage_and_server_errors() {
        let mut server = mockito::Server::new_async().await;
        let garbage = server
            .mock("GET", "/")
            .with_status(200)
            .with_body("<html>502 Bad Gateway</html>")
            .expect(1)
            .create_async()
            .await;
        let good = server
            .mock("GET", "/")
            .with_status(200)
            .with_body(r#"{"endsAt":200,"day":1,"agents":[],"others":[],"traffics":[]}"#)
            .expect(1)
            .create_async()
            .await;
        let client = protocol::MatchClient::new(server.url(), "test").unwrap();
        let state = fetch_state_with_retry(
            &client,
            Duration::from_secs(5),
            Duration::ZERO,
            1,
            &MatchRecorder::disabled(),
        )
        .await
        .unwrap();
        assert_eq!(state.day, 1);
        garbage.assert_async().await;
        good.assert_async().await;
    }

    /// Issue #385: bad credentials are the one thing no later poll can fix.
    #[tokio::test]
    async fn state_polling_stops_at_once_on_bad_credentials() {
        let mut server = mockito::Server::new_async().await;
        let denied = server
            .mock("GET", "/")
            .with_status(401)
            .with_body(r#"{"code":"AuthError","message":"no"}"#)
            .expect(1)
            .create_async()
            .await;
        let client = protocol::MatchClient::new(server.url(), "test").unwrap();
        let result = fetch_state_with_retry(
            &client,
            Duration::from_secs(5),
            Duration::ZERO,
            0,
            &MatchRecorder::disabled(),
        )
        .await;
        assert!(matches!(result, Err(CoordinatorLoopError::State(_))), "{result:?}");
        denied.assert_async().await;
    }

    fn far_deadline() -> i64 {
        Utc::now().timestamp() + 30
    }

    /// Issue #385: a `POST /` that dies without a verdict is sent again while
    /// the window is open, and the verdict of the repeat is what counts.
    #[tokio::test]
    async fn plan_without_a_verdict_is_sent_again_until_accepted() {
        let mut server = mockito::Server::new_async().await;
        let broken = server.mock("POST", "/").with_status(503).expect(1).create_async().await;
        let unreadable = server
            .mock("POST", "/")
            .with_status(200)
            .with_body("<html>")
            .expect(1)
            .create_async()
            .await;
        let accepted = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(r#"{"revision":7}"#)
            .expect(1)
            .create_async()
            .await;
        let client = protocol::MatchClient::new(server.url(), "test").unwrap();
        let plan: ActionPlanBody = vec![vec![-3]];
        let verdict = submit_plan_until_deadline(
            &client,
            &plan,
            far_deadline(),
            0,
            &MatchRecorder::disabled(),
        )
        .await
        .unwrap()
        .0;
        assert!(matches!(verdict, DaySubmission::Accepted(7)));
        broken.assert_async().await;
        unreadable.assert_async().await;
        accepted.assert_async().await;
    }

    /// A rejection is a verdict on the plan: the same body is never repeated.
    #[tokio::test]
    async fn rejected_plan_is_not_sent_again() {
        let mut server = mockito::Server::new_async().await;
        let rejected = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(r#"{"revision":-1}"#)
            .expect(1)
            .create_async()
            .await;
        let client = protocol::MatchClient::new(server.url(), "test").unwrap();
        let plan: ActionPlanBody = vec![vec![-3]];
        let verdict = submit_plan_until_deadline(
            &client,
            &plan,
            far_deadline(),
            0,
            &MatchRecorder::disabled(),
        )
        .await
        .unwrap()
        .0;
        assert!(matches!(verdict, DaySubmission::Rejected(-1)));
        rejected.assert_async().await;
    }

    /// Past the local deadline nothing is repeated: a late copy would be read
    /// as the next day's plan.
    #[tokio::test]
    async fn plan_is_not_repeated_after_the_answer_window() {
        let mut server = mockito::Server::new_async().await;
        let broken = server.mock("POST", "/").with_status(503).expect(1).create_async().await;
        let client = protocol::MatchClient::new(server.url(), "test").unwrap();
        let plan: ActionPlanBody = vec![vec![-3]];
        let verdict = submit_plan_until_deadline(
            &client,
            &plan,
            Utc::now().timestamp() - 1,
            0,
            &MatchRecorder::disabled(),
        )
        .await
        .unwrap()
        .0;
        assert!(matches!(verdict, DaySubmission::Unanswered(_)));
        broken.assert_async().await;
    }

    #[test]
    fn wait_solver_name_yields_all_kind_0_without_an_explicit_pattern() {
        let kinds = to_wire_kinds(&agent_kinds_for(5, "wait", None));
        assert_eq!(kinds, vec![AgentKind::KIND_0; 5]);
        let kinds = to_wire_kinds(&agent_kinds_for(3, "wait-mvp", None));
        assert_eq!(kinds, vec![AgentKind::KIND_0; 3]);
    }

    /// The AC of Issues #238/#243: an omitted `--kinds` must POST exactly
    /// the body today's coordinator posts, for every official team size —
    /// both on the wire (`0 | 1`) and in the engine view the solver plans
    /// against.
    #[test]
    fn default_is_the_historical_alternation_for_every_team_size() {
        for n in 3..=8 {
            assert_eq!(
                to_wire_kinds(&agent_kinds_for(n, "loop", None)),
                historical_alternation(n),
                "n={n}: the omitted flag must be byte-identical to today's POST body"
            );
            assert_eq!(
                agent_kinds_for(n, "loop", None),
                engine::from_wire::default_agent_kinds(n),
                "n={n}: the live default must equal the offline default"
            );
            // Spelling the default out explicitly changes nothing.
            assert_eq!(
                agent_kinds_for(n, "loop", Some(&pattern(bench::kinds::DEFAULT_KINDS_PATTERN))),
                engine::from_wire::default_agent_kinds(n),
                "n={n}: `--kinds P,S` must equal the omitted flag"
            );
            // An unknown solver keeps the same default (no special case).
            assert_eq!(
                agent_kinds_for(n, "mcts", None),
                engine::from_wire::default_agent_kinds(n),
                "n={n}: unknown solver names stay on the alternating default"
            );
        }
    }

    #[test]
    fn loop_solver_name_alternates_kind_0_and_kind_1() {
        assert_eq!(to_wire_kinds(&agent_kinds_for(4, "loop", None)), historical_alternation(4));
        // Odd count: last agent is KIND_0.
        assert_eq!(to_wire_kinds(&agent_kinds_for(5, "loop", None)), historical_alternation(5));
    }

    #[test]
    fn unknown_solver_name_treated_as_real_solver() {
        // Anything other than "wait"/"wait-mvp" gets the alternating vector
        // so a future solver can ship without touching this helper.
        let kinds = to_wire_kinds(&agent_kinds_for(2, "mcts", None));
        assert_eq!(kinds, vec![AgentKind::KIND_0, AgentKind::KIND_1]);
    }

    #[test]
    fn zero_agents_yields_empty_kinds() {
        assert_eq!(agent_kinds_for(0, "loop", None).len(), 0);
        assert_eq!(agent_kinds_for(0, "loop", Some(&pattern("P,S"))).len(), 0);
    }

    /// Issue #238's composition AC, live: `P,P,P,P,S` fields 4 patrol +
    /// 1 supply on a 5-agent team, and — unlike the offline axis — it wins
    /// for **every** solver, `wait` included.
    #[test]
    fn an_explicit_pattern_wins_for_every_solver() {
        let pat = pattern("P,P,P,P,S");
        let expected = vec![
            EngineAgentKind::Patrol,
            EngineAgentKind::Patrol,
            EngineAgentKind::Patrol,
            EngineAgentKind::Patrol,
            EngineAgentKind::Supply,
        ];
        for solver in ["loop", "wait", "wait-mvp", "mcts", "candidate:loop:seed=1"] {
            assert_eq!(
                agent_kinds_for(5, solver, Some(&pat)),
                expected,
                "an explicit --kinds must override the wait special case ({solver})"
            );
        }
        assert_eq!(
            to_wire_kinds(&agent_kinds_for(5, "wait", Some(&pat))),
            vec![
                AgentKind::KIND_0,
                AgentKind::KIND_0,
                AgentKind::KIND_0,
                AgentKind::KIND_0,
                AgentKind::KIND_1,
            ]
        );
    }

    /// Slot cycling + the two single-kind extremes, at the live boundary.
    #[test]
    fn patterns_cycle_over_agent_slots() {
        // `P` → all patrol (the official deadline-miss default is legal).
        let all_patrol = agent_kinds_for(5, "loop", Some(&pattern("P")));
        assert_eq!(all_patrol, vec![EngineAgentKind::Patrol; 5]);
        // `S` → all supply.
        let all_supply = agent_kinds_for(5, "loop", Some(&pattern("supply")));
        assert_eq!(all_supply, vec![EngineAgentKind::Supply; 5]);
        // `P,S` cycles: 3 agents → P,S,P (an odd team stays patrol-heavy).
        assert_eq!(
            agent_kinds_for(3, "loop", Some(&pattern("P,S"))),
            vec![EngineAgentKind::Patrol, EngineAgentKind::Supply, EngineAgentKind::Patrol]
        );
        // A pattern longer than the team is its prefix.
        assert_eq!(
            agent_kinds_for(3, "loop", Some(&pattern("P,P,P,P,S"))),
            vec![EngineAgentKind::Patrol; 3]
        );
    }

    /// Grammar parity with `procon-eval --kinds` (Issue #238): the live
    /// flag parses through `bench::kinds`, so a typo here is rejected with
    /// the offline parser's message rather than a second, drifting parser.
    #[test]
    fn the_live_flag_uses_the_offline_grammar() {
        for ok in ["P,S", "p, S ,Patrol,supply", "P,P,P,P,S", "P"] {
            assert!(bench::kinds::parse_kinds(ok).is_ok(), "{ok} must parse");
        }
        for bad in ["", "3p2s", "X", "P,S,", ",P", "patroll", "P,S,P,S,P,S,P,S,P"] {
            match bench::kinds::parse_kinds(bad) {
                Ok(ok) => panic!("{bad} must be rejected, got {ok:?}"),
                Err(err) => assert!(err.contains("--kinds"), "{bad} → {err}"),
            }
        }
    }

    // ------------------------------------------------------------------
    // Issue #236 — per-day evidence summaries.
    // ------------------------------------------------------------------

    fn wire_agent(kind: AgentKind, pos: i32, fuel: i32) -> WireAgentState {
        WireAgentState { kind, pos, fuel }
    }

    #[test]
    fn day_summary_records_day_start_kind_pos_and_fuel() {
        let state = [wire_agent(AgentKind::KIND_0, 0, 30), wire_agent(AgentKind::KIND_1, 8, 12)];
        let plan: ActionPlanBody = vec![vec![-16]];
        let summary = DaySummary::new(3, &state, &plan);
        assert_eq!(summary.day, 3);
        assert_eq!(summary.agents.len(), 2);
        assert_eq!(summary.agents[0].kind, AgentKind::KIND_0);
        assert_eq!(summary.agents[0].pos, 0);
        assert_eq!(summary.agents[0].fuel, 30, "fuel timeline's first sample");
        assert_eq!(summary.agents[1].kind, AgentKind::KIND_1);
        assert_eq!(summary.agents[1].pos, 8);
        assert_eq!(summary.agents[1].fuel, 12);
        // One movement summary per submitted agent, in submission order.
        assert_eq!(summary.movement.len(), 1);
    }

    #[test]
    fn day_movement_summarises_moves_waits_runs_and_directions() {
        // 3 moves (right, right, upper-right) + a 4-step wait run + one
        // more move, then an 8-step trailing wait — the shape
        // `normalize_day_plan` produces (#226).
        let actions = [2_i32, 2, 1, -4, 0, -8];
        let m = summarise_actions(&actions);
        assert_eq!(m.moves, 4);
        assert_eq!(m.waits, 12, "wait steps are summed, not run-counted");
        assert_eq!(m.runs, 2, "two wait runs");
        assert_eq!(m.moves + m.waits, 16, "element sum of the plan");
        assert_eq!(m.dirs, [1, 1, 2, 0, 0, 0], "official wire order 0..=5");
    }

    #[test]
    fn day_movement_is_a_summary_not_the_raw_plan() {
        // The whole point of Issue #236: a full day plan collapses into a
        // fixed four-field record no matter how long the answer body was.
        let mut actions = vec![2_i32; 70];
        actions.push(-76);
        let m = summarise_actions(&actions);
        assert_eq!(m.moves, 70);
        assert_eq!(m.waits, 76);
        assert_eq!(m.runs, 1);
        // No plan elements survive into the summary struct itself.
        let json = serde_json::to_string(&m).expect("serialises");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parses");
        assert_eq!(
            value.as_object().expect("object").len(),
            4,
            "movement summary must stay a fixed four fields: {json}"
        );
        assert!(json.len() < 96, "summary bytes must not track plan length: {json}");
    }

    #[test]
    fn day_movement_skips_illegal_direction_codes_without_panicking() {
        let m = summarise_actions(&[6, -1, 99]);
        assert_eq!(m.moves, 0, "out-of-range codes are not moves");
        assert_eq!(m.waits, 1);
        assert_eq!(m.runs, 1);
        assert_eq!(m.dirs, [0; 6]);
        // i32::MIN must not overflow the wait sum either.
        let m = summarise_actions(&[i32::MIN]);
        assert_eq!(m.runs, 1);
    }
}
