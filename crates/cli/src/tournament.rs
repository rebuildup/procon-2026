//! **Wire / protocol smoke harness** — N solvers × 1 real-server match.
//!
//! This drives each solver through a full match against the **official
//! server** via
//! [`run_coordinator_loop`](crate::coordinator_loop::run_coordinator_loop)
//! and reports how much of the protocol each solver got through. It is an
//! end-to-end smoke test of the server / protocol / coordinator stack.
//!
//! # This is NOT the performance evaluator
//!
//! The number this produces is [`protocol_progress`] —
//! `days_completed * 100 + sum(revisions)`. It measures **how far the
//! protocol handshake and match loop got**, not how well the solver played.
//! A solver that completes every day with one revision each scores the same
//! as one that barely scrapes through; nothing here looks at what the agents
//! actually collected.
//!
//! The canonical performance evaluator — the one that ranks solvers on the
//! official competitive score tuple (variety, daily variety sum, total balls,
//! response time) — is offline and lives in the `bench` crate. See
//! `docs/evaluation.md`. To compare solver quality, use that, not this.
//!
//! # What this is good for
//!
//! - proving the wire protocol still works against the real server
//! - catching regressions in the coordinator / answer-submission loop
//! - smoke-testing a solver's integration (does it answer at all, does the
//!   server accept its answers) without needing a corpus
//!
//! Matches run **sequentially** because the official contest server only
//! allows one match at a time per `MatchClient` connection.
//!
//! # Solver registry
//!
//! The registry mirrors [`crate::solver_adapter`] so that `--solver <name>`
//! values accepted by `procon coordinator` are also accepted by
//! `procon tournament --solvers <name,...>`. Unknown names produce
//! [`TournamentError::UnknownSolver`] (no silent fallback).
//!
//! | name   | implementation                              | notes |
//! |--------|---------------------------------------------|-------|
//! | `wait` | [`WaitSolver`](crate::solver_adapter::WaitSolver) | debug-only no-op plan |
//! | `loop` | [`EngineLoopAdapter`](crate::solver_adapter::EngineLoopAdapter) wrapping [`solver_loop::LoopSolver`] | production iterative-improvement strategy |

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engine::model::MatchSetting as EngineSetting;
use engine::solver::Solver;
use serde::Serialize;
use solver_loop::InitialStrategy;

use crate::coordinator_loop::{
    CoordinatorLoopConfig, CoordinatorLoopError, CoordinatorOutcome, run_coordinator_loop,
};
use crate::solver_adapter::{EngineLoopAdapter, WaitSolver};

/// Errors produced by [`run_tournament`].
#[derive(Debug, thiserror::Error)]
pub enum TournamentError {
    /// `--solvers` did not include any entries.
    #[error("no solvers specified")]
    NoSolvers,
    /// `--solvers` referenced a name that is not in the registry.
    #[error("invalid solver name: {0}")]
    UnknownSolver(String),
    /// The match driver returned an error for one of the solvers.
    #[error("match failed for solver {solver}: {source}")]
    MatchFailed {
        solver: String,
        #[source]
        source: CoordinatorLoopError,
    },
    /// The optional `auto_restart_server` hook failed (non-zero exit, or
    /// the harness could not spawn the shell). Surfaced before any per-solver
    /// HTTP traffic so the operator sees the root cause rather than a
    /// confusing follow-up protocol error from the still-down server.
    #[error("server restart failed: {0}")]
    ServerRestartFailed(String),
    /// Building a [`protocol::MatchClient`] failed (e.g. malformed URL).
    #[error("match client build failed: {0}")]
    ClientBuild(#[from] protocol::HttpClientError),
}

/// Per-solver tournament result.
///
/// `outcome` is the raw [`CoordinatorOutcome`] for programmatic access;
/// the JSON sidecar serialises it via the flat [`OutcomeView`] helper
/// because `CoordinatorOutcome` does not derive `Serialize` (intentional
/// — exposing it on the wire is an unrelated decision).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TournamentEntry {
    /// Name the user passed to `--solvers`.
    pub solver: String,
    /// Raw coordinator outcome (days completed + per-day revisions).
    pub outcome: CoordinatorOutcome,
    /// Wall-clock time the per-solver match took, in milliseconds.
    pub elapsed_ms: u64,
    /// **Protocol progress**, not a competitive score.
    ///
    /// `days_completed * 100 + sum(revisions)`. Higher means the solver got
    /// further through the match's protocol, which is a smoke-test signal —
    /// see the module docs. It is deliberately *not* named `score` so it can
    /// never be mistaken for the official competitive score tuple.
    pub protocol_progress: u32,
}

/// Aggregate leaderboard for a single tournament run.
///
/// Entries are kept sorted by [`TournamentEntry::protocol_progress`] descending. The
/// JSON sidecar uses the flat [`ReportView`] helper for the same reason
/// as [`OutcomeView`]: `CoordinatorOutcome` is not `Serialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TournamentReport {
    /// Per-solver entries, sorted by [`TournamentEntry::protocol_progress`] descending.
    pub entries: Vec<TournamentEntry>,
    /// Server URL the harness drove every solver against.
    pub server: String,
    /// Total wall-clock time the tournament took, in milliseconds.
    pub elapsed_ms: u64,
    /// Schema version. Bumped on any breaking change to the wire format.
    pub schema_version: u32,
}

/// Serialisable view of a [`CoordinatorOutcome`] (days completed + per-day
/// revisions). Used by the JSON sidecar at
/// `<state_output_dir>/leaderboard.json`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OutcomeView {
    pub days_completed: usize,
    pub revisions: Vec<i32>,
}

impl From<&CoordinatorOutcome> for OutcomeView {
    fn from(o: &CoordinatorOutcome) -> Self {
        Self { days_completed: o.days_completed, revisions: o.revisions.clone() }
    }
}

/// Serialisable view of a [`TournamentEntry`] (solver + protocol progress + outcome).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EntryView {
    pub solver: String,
    /// See [`TournamentEntry::protocol_progress`].
    pub protocol_progress: u32,
    pub elapsed_ms: u64,
    pub outcome: OutcomeView,
}

impl From<&TournamentEntry> for EntryView {
    fn from(e: &TournamentEntry) -> Self {
        Self {
            solver: e.solver.clone(),
            protocol_progress: e.protocol_progress,
            elapsed_ms: e.elapsed_ms,
            outcome: OutcomeView::from(&e.outcome),
        }
    }
}

/// Serialisable view of a [`TournamentReport`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReportView {
    pub entries: Vec<EntryView>,
    pub server: String,
    pub elapsed_ms: u64,
    pub schema_version: u32,
}

impl From<&TournamentReport> for ReportView {
    fn from(r: &TournamentReport) -> Self {
        Self {
            entries: r.entries.iter().map(EntryView::from).collect(),
            server: r.server.clone(),
            elapsed_ms: r.elapsed_ms,
            schema_version: r.schema_version,
        }
    }
}

/// Behavioural knobs for [`run_tournament`].
///
/// Most fields are populated by the CLI layer from flags. The struct is
/// `Clone` so it can be cheaply forwarded through CLI parsing pipelines.
#[derive(Debug, Clone)]
pub struct TournamentConfig {
    /// Server base URL (e.g. `http://localhost:8080`).
    pub server: String,
    /// Bearer token handed to `protocol::MatchClient`.
    pub token: String,
    /// Solver names to evaluate. Must be non-empty.
    pub solvers: Vec<String>,
    /// Initial strategy for the `loop` solver. Parsed from
    /// `--loop-initial-strategy` by
    /// [`crate::initial_strategy::parse_initial_strategy`] — the same
    /// parser `procon coordinator` uses — which **rejects unknown labels**
    /// rather than falling back (Issue #170). Its offline counterpart is
    /// the `loop:initial=<label>` candidate parameter.
    pub loop_initial_strategy: InitialStrategy,
    /// Iteration cap for the `loop` solver's local search.
    pub loop_iterations: u32,
    /// Team id baked into `CoordinatorLoopConfig::team_id`.
    pub team_id: String,
    /// Optional path where the harness writes a JSON leaderboard. When
    /// `None`, the harness only prints the markdown table to stdout.
    pub state_output_dir: Option<std::path::PathBuf>,
    /// Optional shell command invoked via `sh -c` BEFORE each solver run
    /// except the first. The official `procon-server-linux-amd64` only
    /// supports one match per process, so multi-solver tournaments need a
    /// restart hook (e.g. `./scripts/restart-official-server.sh 8080`).
    /// When `None`, every solver shares the same server process — only
    /// works when the server accepts multiple matches (e.g. the local
    /// axum mock used in the integration tests).
    pub auto_restart_server: Option<String>,
}

impl Default for TournamentConfig {
    fn default() -> Self {
        Self {
            server: "http://localhost:8080".to_owned(),
            token: "token-p0".to_owned(),
            solvers: Vec::new(),
            loop_initial_strategy: InitialStrategy::GreedyNearestNeighbor,
            loop_iterations: 32,
            team_id: "team-p0".to_owned(),
            state_output_dir: None,
            auto_restart_server: None,
        }
    }
}

/// Optional slot the coordinator loop populates with the parsed engine
/// setting after Phase 1+3 succeed. Concrete solvers that want map-aware
/// planning clone this slot and pass the clone to
/// [`CoordinatorLoopConfig::engine_setting_slot`] so the loop can inject
/// the real map + agents into the adapter (see issue #158).
type EngineSettingSlot = Arc<Mutex<Option<EngineSetting>>>;

/// Tuple returned by [`build_solver`]: the solver trait object + the
/// optional setting slot for map-aware planning.
type BuiltSolver = (Box<dyn Solver>, Option<EngineSettingSlot>);

/// Build a `Box<dyn Solver>` for the given registry name, plus the
/// optional `engine_setting_slot` to thread through
/// [`CoordinatorLoopConfig`] so the loop solver can plan against the real
/// map (issue #158). Returns `None` for the slot when the solver does not
/// expose one (e.g. `WaitSolver`).
fn build_solver(
    name: &str,
    initial_strategy: InitialStrategy,
    max_iterations: u32,
) -> Result<BuiltSolver, TournamentError> {
    match name {
        "wait" => Ok((Box::new(WaitSolver), None)),
        "loop" => {
            let adapter = EngineLoopAdapter::new(initial_strategy, max_iterations);
            let slot = adapter.setting_handle();
            Ok((Box::new(adapter), Some(slot)))
        }
        other => Err(TournamentError::UnknownSolver(other.to_owned())),
    }
}

/// Drive `cfg.solvers` through one full match each, in order, and return
/// a [`TournamentReport`] sorted by protocol progress descending. Matches run
/// sequentially — the contest server only honours one match per session,
/// and parallel eval would break it.
///
/// # Errors
///
/// See [`TournamentError`]. A single failed match aborts the tournament
/// and surfaces the failing solver's name; later solvers are not run.
pub async fn run_tournament(cfg: TournamentConfig) -> Result<TournamentReport, TournamentError> {
    if cfg.solvers.is_empty() {
        return Err(TournamentError::NoSolvers);
    }
    let started = Instant::now();

    let client = protocol::MatchClient::builder(&cfg.server, &cfg.token)
        .retry_policy(protocol::RetryPolicy::default())
        .request_timeout(Duration::from_secs(5))
        .build()?;

    let mut entries = Vec::with_capacity(cfg.solvers.len());
    for (idx, solver_name) in cfg.solvers.iter().enumerate() {
        // Restart the match server BEFORE every solver except the first:
        // the official `procon-server-linux-amd64` closes its kind window
        // after the first match, so multi-solver tournaments need a fresh
        // process for each solver. The restart hook is opt-in via
        // `--auto-restart-server` / `PROCON_AUTO_RESTART_SERVER`; when
        // unset, the harness shares the server process across solvers
        // (useful with the local axum mock used in integration tests).
        if idx > 0 {
            if let Some(cmd) = cfg.auto_restart_server.as_deref() {
                maybe_restart_server(cmd)?;
            }
        }

        let (solver, engine_setting_slot) =
            build_solver(solver_name, cfg.loop_initial_strategy, cfg.loop_iterations)?;

        let loop_cfg = CoordinatorLoopConfig {
            team_id: cfg.team_id.clone(),
            solver_name: solver_name.clone(),
            server_url: Some(cfg.server.clone()),
            engine_setting_slot,
            ..CoordinatorLoopConfig::default()
        };

        let t0 = Instant::now();
        let result = run_coordinator_loop(client.clone(), solver, loop_cfg).await;
        let elapsed_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);

        let outcome = result.map_err(|source| TournamentError::MatchFailed {
            solver: solver_name.clone(),
            source,
        })?;

        let protocol_progress = compute_protocol_progress(&outcome);
        entries.push(TournamentEntry {
            solver: solver_name.clone(),
            outcome,
            elapsed_ms,
            protocol_progress,
        });
    }

    // Stable sort by protocol progress desc, then by solver name asc as a
    // deterministic tie-break so two identical values order identically across runs.
    entries.sort_by(|a, b| {
        b.protocol_progress.cmp(&a.protocol_progress).then_with(|| a.solver.cmp(&b.solver))
    });

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let report = TournamentReport { entries, server: cfg.server, elapsed_ms, schema_version: 1 };

    if let Some(dir) = cfg.state_output_dir.as_ref() {
        if let Err(e) = write_leaderboard_json(dir, &report) {
            tracing::warn!(?dir, error = %e, "tournament: leaderboard JSON write failed");
        }
    }

    Ok(report)
}

/// `days_completed * 100 + sum(revisions)` — **protocol progress**, higher
/// means the solver got further through the match. This is a smoke-test
/// signal, not a competitive score; see the module docs.
///
/// Negative revisions (which the loop never actually returns — `Rejected`
/// aborts the match) are clamped to zero so a malformed future payload can't
/// produce a negative value.
fn compute_protocol_progress(outcome: &CoordinatorOutcome) -> u32 {
    let days = u32::try_from(outcome.days_completed).unwrap_or(u32::MAX);
    let revisions: u32 = outcome
        .revisions
        .iter()
        .map(|&r| if r < 0 { 0_u32 } else { u32::try_from(r).unwrap_or(u32::MAX) })
        .sum();
    days.saturating_mul(100).saturating_add(revisions)
}

/// Invoke the operator-supplied server restart hook via `sh -c <cmd>` so
/// shell pipelines like `./restart.sh && echo ok` are passed verbatim
/// without argv splitting. A non-zero exit surfaces as
/// [`TournamentError::ServerRestartFailed`] and aborts the tournament —
/// silent continuation would just bury the real failure behind a
/// confusing protocol error on the still-down server.
///
/// A short grace period sleeps the slot after the command returns so the
/// freshly spawned process has time to bind its TCP port. The bundled
/// `scripts/restart-official-server.sh` polls for readiness, but
/// operator-supplied commands may not, and a defensive sleep keeps the
/// harness robust against simple `pkill && ./procon-server &` scripts.
fn maybe_restart_server(cmd: &str) -> Result<(), TournamentError> {
    tracing::info!(cmd, "restarting match server");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .map_err(|e| TournamentError::ServerRestartFailed(e.to_string()))?;
    if !status.success() {
        return Err(TournamentError::ServerRestartFailed(format!(
            "command exited with status {status}"
        )));
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    Ok(())
}

/// Render `report` as a markdown table.
pub fn render_markdown(report: &TournamentReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# Tournament leaderboard (server={})", report.server);
    out.push('\n');
    out.push_str("| Rank | Solver | Days | Revisions | Protocol progress | Wall ms |\n");
    out.push_str("|------|--------|------|-----------|-------|---------|\n");
    for (i, e) in report.entries.iter().enumerate() {
        let rank = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let _ = writeln!(
            out,
            "| {} | {} | {} | {:?} | {} | {} |",
            rank,
            e.solver,
            e.outcome.days_completed,
            e.outcome.revisions,
            e.protocol_progress,
            e.elapsed_ms,
        );
    }
    let _ = writeln!(out, "\nTotal elapsed: {} ms\n", report.elapsed_ms);
    out
}

/// Atomic write of `report` to `<dir>/leaderboard.json`. Best-effort:
/// failures are logged via `tracing::warn!` and never propagated (the
/// tournament is still successful even when the sidecar write fails).
/// Serialises via [`ReportView`] because [`TournamentReport`] contains a
/// non-`Serialize` field.
fn write_leaderboard_json(dir: &std::path::Path, report: &TournamentReport) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("leaderboard.json");
    let tmp = path.with_extension("json.tmp");
    let view = ReportView::from(report);
    let json = serde_json::to_vec_pretty(&view)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_progress_is_days_times_100_plus_sum_revisions() {
        let outcome = CoordinatorOutcome {
            days_completed: 4,
            revisions: vec![1, 2, 3, 4],
            ..CoordinatorOutcome::default()
        };
        assert_eq!(compute_protocol_progress(&outcome), 4 * 100 + 10);
    }

    #[test]
    fn protocol_progress_clamps_negative_revisions_to_zero() {
        let outcome = CoordinatorOutcome {
            days_completed: 2,
            revisions: vec![-1, 5],
            ..CoordinatorOutcome::default()
        };
        assert_eq!(compute_protocol_progress(&outcome), 2 * 100 + 5);
    }

    /// Label → `TournamentConfig.loop_initial_strategy` → label round-trip
    /// for the whole live label set, plus the Issue #170 guarantee that an
    /// unknown label errors instead of silently becoming `Identity`.
    /// (The parser itself lives in [`crate::initial_strategy`]; the CLI
    /// flag test for both subcommands is in `main.rs`.)
    #[test]
    fn tournament_config_round_trips_every_initial_label() {
        use crate::initial_strategy::{initial_strategy_label, parse_initial_strategy};

        for (label, expected) in [
            ("greedy", InitialStrategy::GreedyNearestNeighbor),
            ("identity", InitialStrategy::Identity),
            ("reverse", InitialStrategy::Reverse),
            ("sweep-row", InitialStrategy::SweepRow),
            ("sweep-col", InitialStrategy::SweepCol),
            ("random:42", InitialStrategy::Random { seed: 42 }),
        ] {
            let parsed = parse_initial_strategy(label).unwrap_or_else(|e| panic!("{label}: {e}"));
            let cfg =
                TournamentConfig { loop_initial_strategy: parsed, ..TournamentConfig::default() };
            assert_eq!(cfg.loop_initial_strategy, expected, "label {label}");
            assert_eq!(
                initial_strategy_label(cfg.loop_initial_strategy),
                label,
                "config must round-trip back to the canonical label"
            );
        }

        // Unknown label ⇒ explicit error, never a silent Identity fallback.
        let err = parse_initial_strategy("GreedyZoneCoverage")
            .expect_err("unknown labels must be rejected");
        assert!(
            err.to_string().contains("greedy | identity | reverse"),
            "error must list the accepted labels: {err}"
        );
        assert_ne!(
            parse_initial_strategy("GreedyZoneCoverage"),
            Ok(InitialStrategy::Identity),
            "a typo must not resolve to Identity"
        );
    }

    #[test]
    fn render_markdown_contains_all_solvers_and_header() {
        let report = TournamentReport {
            entries: vec![
                TournamentEntry {
                    solver: "wait".to_owned(),
                    outcome: CoordinatorOutcome {
                        days_completed: 4,
                        revisions: vec![1, 2, 3, 4],
                        ..CoordinatorOutcome::default()
                    },
                    elapsed_ms: 100,
                    protocol_progress: 410,
                },
                TournamentEntry {
                    solver: "loop".to_owned(),
                    outcome: CoordinatorOutcome {
                        days_completed: 4,
                        revisions: vec![1, 2, 3, 4],
                        ..CoordinatorOutcome::default()
                    },
                    elapsed_ms: 200,
                    protocol_progress: 410,
                },
            ],
            server: "http://localhost:8080".to_owned(),
            elapsed_ms: 300,
            schema_version: 1,
        };
        let md = render_markdown(&report);
        assert!(md.contains("# Tournament leaderboard"));
        assert!(md.contains("| Rank | Solver | Days | Revisions | Protocol progress | Wall ms |"));
        assert!(md.contains("wait"));
        assert!(md.contains("loop"));
        // Rank column: 1 and 2 must both appear (dense ranks).
        assert!(md.contains("| 1 |"));
        assert!(md.contains("| 2 |"));
        assert!(md.contains("Total elapsed: 300 ms"));
    }

    #[test]
    fn run_tournament_rejects_empty_solver_list() {
        // We don't have a server, but `NoSolvers` short-circuits before
        // any HTTP work happens, so this stays synchronous-friendly.
        let cfg = TournamentConfig::default();
        let rt =
            tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let result = rt.block_on(run_tournament(cfg));
        assert!(matches!(result, Err(TournamentError::NoSolvers)));
    }
}
