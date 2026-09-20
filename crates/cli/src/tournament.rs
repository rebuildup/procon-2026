//! Tournament harness: N solvers × 1 match → leaderboard.
//!
//! Drives each solver through a full match via
//! [`run_coordinator_loop`](crate::coordinator_loop::run_coordinator_loop)
//! and reports the per-solver [`CoordinatorOutcome`](crate::coordinator_loop::CoordinatorOutcome)
//! as a simple score (`days_completed * 100 + sum(revisions)`). Matches run
//! **sequentially** because the official contest server only allows one match
//! at a time per `MatchClient` connection; parallel eval is intentionally out
//! of scope for this iteration.
//!
//! # Loop engineering workflow
//!
//! Successive iterations can use this harness for the user's "loop
//! engineering" workflow: develop solver variants in parallel branches, run
//! the tournament, compare scores, pick the winner. The scoring formula is
//! intentionally trivial — the goal is to wire the harness to the real match
//! path, not to build a sophisticated ranking system.
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
    /// `days_completed * 100 + sum(revisions)`. Higher is better.
    pub score: u32,
}

/// Aggregate leaderboard for a single tournament run.
///
/// Entries are kept sorted by [`TournamentEntry::score`] descending. The
/// JSON sidecar uses the flat [`ReportView`] helper for the same reason
/// as [`OutcomeView`]: `CoordinatorOutcome` is not `Serialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TournamentReport {
    /// Per-solver entries, sorted by [`TournamentEntry::score`] descending.
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

/// Serialisable view of a [`TournamentEntry`] (solver + score + outcome view).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EntryView {
    pub solver: String,
    pub score: u32,
    pub elapsed_ms: u64,
    pub outcome: OutcomeView,
}

impl From<&TournamentEntry> for EntryView {
    fn from(e: &TournamentEntry) -> Self {
        Self {
            solver: e.solver.clone(),
            score: e.score,
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
    /// Initial-strategy label for the `loop` solver.
    /// `GreedyNearestNeighbor` (default) or `Identity` (fallback when the
    /// label is unrecognised — `Identity` is the safest deterministic
    /// stand-in and avoids producing a mystery behaviour).
    pub loop_initial_strategy: InitialStrategy,
    /// Iteration cap for the `loop` solver's local search.
    pub loop_iterations: u32,
    /// Team id baked into `CoordinatorLoopConfig::team_id`.
    pub team_id: String,
    /// Optional path where the harness writes a JSON leaderboard. When
    /// `None`, the harness only prints the markdown table to stdout.
    pub state_output_dir: Option<std::path::PathBuf>,
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
        }
    }
}

impl TournamentConfig {
    /// Parse a `GreedyNearestNeighbor` / `Identity` style label into an
    /// [`InitialStrategy`]. Unknown labels degrade to
    /// [`InitialStrategy::Identity`] so a future variant can ship without
    /// breaking the harness; callers that need to reject unknown names
    /// should validate before constructing the config.
    #[must_use]
    pub fn parse_initial_strategy(label: &str) -> InitialStrategy {
        match label {
            "GreedyNearestNeighbor" => InitialStrategy::GreedyNearestNeighbor,
            // Both `Identity` and unrecognised labels fall through to the
            // safest deterministic stand-in. Documented defensive fallback.
            _ => InitialStrategy::Identity,
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
/// a [`TournamentReport`] sorted by score descending. Matches run
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
    for solver_name in &cfg.solvers {
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

        let score = compute_score(&outcome);
        entries.push(TournamentEntry { solver: solver_name.clone(), outcome, elapsed_ms, score });
    }

    // Stable sort by score desc, then by solver name asc as a deterministic
    // tie-break so two identical scores produce the same ordering across runs.
    entries.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.solver.cmp(&b.solver)));

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let report = TournamentReport { entries, server: cfg.server, elapsed_ms, schema_version: 1 };

    if let Some(dir) = cfg.state_output_dir.as_ref() {
        if let Err(e) = write_leaderboard_json(dir, &report) {
            tracing::warn!(?dir, error = %e, "tournament: leaderboard JSON write failed");
        }
    }

    Ok(report)
}

/// `days_completed * 100 + sum(revisions)`. Higher is better. Negative
/// revisions (which the loop never actually returns — `Rejected` aborts
/// the match) are clamped to zero so a malformed future payload can't
/// produce a negative score.
fn compute_score(outcome: &CoordinatorOutcome) -> u32 {
    let days = u32::try_from(outcome.days_completed).unwrap_or(u32::MAX);
    let revisions: u32 = outcome
        .revisions
        .iter()
        .map(|&r| if r < 0 { 0_u32 } else { u32::try_from(r).unwrap_or(u32::MAX) })
        .sum();
    days.saturating_mul(100).saturating_add(revisions)
}

/// Render `report` as a markdown table.
pub fn render_markdown(report: &TournamentReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# Tournament leaderboard (server={})", report.server);
    out.push('\n');
    out.push_str("| Rank | Solver | Days | Revisions | Score | Wall ms |\n");
    out.push_str("|------|--------|------|-----------|-------|---------|\n");
    for (i, e) in report.entries.iter().enumerate() {
        let rank = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let _ = writeln!(
            out,
            "| {} | {} | {} | {:?} | {} | {} |",
            rank, e.solver, e.outcome.days_completed, e.outcome.revisions, e.score, e.elapsed_ms,
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
    fn compute_score_uses_days_times_100_plus_sum_revisions() {
        let outcome = CoordinatorOutcome { days_completed: 4, revisions: vec![1, 2, 3, 4] };
        assert_eq!(compute_score(&outcome), 4 * 100 + 10);
    }

    #[test]
    fn compute_score_clamps_negative_revisions_to_zero() {
        let outcome = CoordinatorOutcome { days_completed: 2, revisions: vec![-1, 5] };
        assert_eq!(compute_score(&outcome), 2 * 100 + 5);
    }

    #[test]
    fn parse_initial_strategy_recognises_known_labels() {
        assert_eq!(
            TournamentConfig::parse_initial_strategy("GreedyNearestNeighbor"),
            InitialStrategy::GreedyNearestNeighbor
        );
        assert_eq!(TournamentConfig::parse_initial_strategy("Identity"), InitialStrategy::Identity);
    }

    #[test]
    fn parse_initial_strategy_falls_back_to_identity_for_unknown_labels() {
        // Documented defensive fallback: future strategies should keep
        // working until the harness grows explicit support for them.
        assert_eq!(
            TournamentConfig::parse_initial_strategy("GreedyZoneCoverage"),
            InitialStrategy::Identity
        );
    }

    #[test]
    fn render_markdown_contains_all_solvers_and_header() {
        let report = TournamentReport {
            entries: vec![
                TournamentEntry {
                    solver: "wait".to_owned(),
                    outcome: CoordinatorOutcome { days_completed: 4, revisions: vec![1, 2, 3, 4] },
                    elapsed_ms: 100,
                    score: 410,
                },
                TournamentEntry {
                    solver: "loop".to_owned(),
                    outcome: CoordinatorOutcome { days_completed: 4, revisions: vec![1, 2, 3, 4] },
                    elapsed_ms: 200,
                    score: 410,
                },
            ],
            server: "http://localhost:8080".to_owned(),
            elapsed_ms: 300,
            schema_version: 1,
        };
        let md = render_markdown(&report);
        assert!(md.contains("# Tournament leaderboard"));
        assert!(md.contains("| Rank | Solver | Days | Revisions | Score | Wall ms |"));
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
