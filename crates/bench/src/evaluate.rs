//! The canonical evaluation path — the single place a candidate meets a case.
//!
//! ```text
//! candidate solver
//!     ↓   (decide → actions only; a solver never scores itself)
//! actions / decisions
//!     ↓
//! engine::MatchSim
//!     ↓
//! final MatchState / RunResult
//!     ↓
//! this module: evaluator
//!     ↓
//! official score tuple (variety, daily_variety_sum, total_balls, response_time)
//!     ↓
//! durable RunRecord
//! ```
//!
//! # Invariant: solvers never score themselves
//!
//! A candidate returns [`engine::model::AgentActions`] and nothing else. The
//! competitive score comes from [`engine::MatchSim`]'s final state, and the
//! official tuple is assembled here from that state plus a measurement of how
//! long the solver took to answer. There is no code path in which a candidate
//! can declare its own score.
//!
//! # Score tuple and ranking order
//!
//! The official score is [`domain::OfficialScore`] and the ranking order is
//! [`domain::compare_scores`] — lexicographic, never a weighted sum:
//!
//! 1. `variety` — distinct series ever acquired (higher is better)
//! 2. `daily_variety_sum` — Σ per-day distinct series (higher is better)
//! 3. `total_balls` — total balls acquired (higher is better)
//! 4. `response_time` — cumulative per-day answer latency (**lower** is better)
//!
//! The first three are deterministic given (commit, candidate config, case).
//! The fourth is a *measurement* and therefore varies run to run; it is kept
//! strictly separate from wall-clock [`RunRecord::elapsed_us`], which is
//! evidence only and never enters the ranking.
//!
//! # Failure isolation
//!
//! Each `(candidate, case)` pair runs on its own worker thread with a wall
//! deadline. A candidate that panics, hangs, or emits a malformed plan yields
//! a [`RunRecord`] with a [`RunStatus`] describing what happened — it never
//! aborts the batch. Every other candidate and case still evaluates.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use domain::{OfficialScore, PlayerId};
use engine::model::{AgentActions, MatchSetting, RunResult, Solver, SolverContext};
use engine::{MatchSim, SimConfig};
use serde::{Deserialize, Serialize};

use crate::candidate::Candidate;
use crate::case::CaseSpec;
use crate::kinds::render_kinds;

/// What happened to one `(candidate, case)` evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The match ran to completion and the candidate produced well-shaped
    /// plans for every day. The score is authoritative.
    Success,
    /// The evaluation ran to completion but the candidate violated the plan
    /// contract on at least one day (or the engine could not score the run).
    /// The score is still recorded — `engine::MatchSim` degrades a malformed
    /// plan to all-wait rather than aborting — but the candidate is flagged.
    Invalid,
    /// The run exceeded the configured wall deadline. No score is available.
    Timeout,
    /// The candidate panicked. No score is available.
    Panic,
}

impl RunStatus {
    /// True when this status *can* carry an authoritative score.
    ///
    /// Note that [`Self::Invalid`] is the one ambiguous case: the run either
    /// completed with a degraded-but-real score (a malformed plan), or never
    /// produced one (the engine rejected the case). Use
    /// [`RunRecord::is_scored`] to know which — that is the authoritative
    /// flag. This method only answers "is this status *capable* of scoring".
    #[must_use]
    pub const fn can_carry_score(self) -> bool {
        matches!(self, Self::Success | Self::Invalid)
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Success => "success",
            Self::Invalid => "invalid",
            Self::Timeout => "timeout",
            Self::Panic => "panic",
        })
    }
}

/// Durable evidence for one `(candidate, case)` evaluation.
///
/// This is the machine-readable unit that gets appended to a JSONL file and
/// re-aggregated later. Every field needed to reproduce and compare a run is
/// here; nothing is inferred from context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Stable candidate identity, e.g. `loop:iterations=32:initial=greedy`.
    pub candidate_id: String,
    /// Stable case identity, e.g. `fixture:small-8x8`.
    pub case_id: String,
    /// Case generation seed, when the case was generated.
    pub seed: Option<u64>,
    /// Case generation parameters, when the case was generated.
    pub generation_parameters: Option<serde_json::Value>,
    /// Commit the evaluation ran at.
    pub source_commit_sha: String,
    /// The team this candidate played as (`CaseSpec`'s `MatchSetting::team`).
    ///
    /// Recorded so a batch is attributable: under process isolation the team
    /// travels as a command-line argument, and this field is what proves it
    /// arrived instead of silently falling back to the default.
    pub team: String,

    // ---- official score tuple (lexicographic ranking order) ----
    /// Tier 1 — distinct series ever acquired. Higher is better.
    pub variety: u32,
    /// Tier 2 — Σ per-day distinct series. Higher is better.
    pub daily_variety_sum: u32,
    /// Tier 3 — total balls acquired. Higher is better.
    pub total_balls: u32,
    /// Tier 4 — Σ per-day answer latency, microseconds. **Lower is better.**
    ///
    /// This is *not* wall-clock elapsed: it is the time the solver spent
    /// producing each day's answer, summed over days, which is the offline
    /// stand-in for the contest's 「各日の最終回答時刻の累積」.
    pub response_time_us: u64,

    // ---- measurement evidence (never ranked) ----
    /// Whole-run wall clock, microseconds. Includes simulator stepping.
    /// Environmental noise lives here, which is exactly why it is kept out
    /// of the score tuple.
    pub elapsed_us: u64,

    // ---- outcome ----
    /// Explicit timeout flag (redundant with `status`, kept for cheap
    /// re-aggregation of JSONL without parsing the status string).
    pub timeout: bool,
    /// What happened.
    pub status: RunStatus,
    /// True when the three competitive tiers above are authoritative.
    ///
    /// This is the flag to trust — not [`RunStatus`], which cannot distinguish
    /// an `Invalid` run that completed with a real (degraded) score from one
    /// that never produced one.
    pub scored: bool,
    /// Error / panic message when the run did not succeed cleanly.
    pub status_detail: Option<String>,
    /// `(day, step)` of the latest *first*-acquisition. Diagnostic only —
    /// not a ranking tier.
    pub last_acquisition_at: Option<(u32, u32)>,
    /// Actions the simulator had to downgrade because they were not legal
    /// (out of bounds, impassable terrain, or out of fuel).
    ///
    /// The official rules reject an answer containing an invalid action
    /// (Q&A Q6), so a run with a non-zero count is recorded as
    /// [`RunStatus::Invalid`] and is **not** ranked — `MatchSim` degrades such
    /// an action to `Wait` so the match completes, but completing is not the
    /// same as being legal.
    pub illegal_action_count: u32,
    /// The subset of [`Self::illegal_action_count`] caused by fuel exhaustion.
    pub fuel_short_count: u32,
    /// Days (zero-based) whose measured answer time exceeded that day's
    /// official budget — the per-day `daySeconds` window (Q&A Q3).
    ///
    /// Non-empty means the official server would not have accepted that
    /// day's answer, while `MatchSim` took it at face value. The scores above
    /// would therefore be optimistic, so the run is marked `Invalid` and
    /// excluded from ranking.
    ///
    /// This is *not* [`EvalConfig::harness_deadline`], which merely kills a
    /// worker that has stopped making progress.
    pub day_budget_exceeded_days: Vec<u32>,
    /// Per-day answer latency in microseconds, one entry per simulated day.
    pub daily_response_us: Vec<u64>,
    /// Distinct series acquired on each day, one entry per simulated day.
    ///
    /// `daily_variety_sum` is the sum of this vector (tier 2). Recording the
    /// breakdown lets a later re-aggregation change the policy without
    /// re-running the batch.
    pub daily_varieties: Vec<u32>,
    /// The team composition this run actually fielded, one entry per agent
    /// slot in `ctx.agent_ids` order (Issue #235).
    ///
    /// `None` only on records written before the `--kinds` axis existed and
    /// on runs that never produced a final state (timeout / panic) — every
    /// scored record from this commit on carries `Some`. This is what makes
    /// `--kinds` auditable evidence rather than a claim about the batch: the
    /// composition is read back from the simulated setting, not from the flag.
    #[serde(default)]
    pub agent_kinds: Option<Vec<engine::model::AgentKind>>,
    /// Balls each agent slot finished with, one entry per agent in
    /// `ctx.agent_ids` order (Issue #243).
    ///
    /// `None` only on records written before this axis existed and on runs
    /// that never produced a final state. Its job is the Issue #243 ablation
    /// metric that `total_balls` cannot express: **how many agents reached
    /// the board at all** (a `0` entry = that agent never collected a ball).
    #[serde(default)]
    pub agent_ball_counts: Option<Vec<u32>>,
    /// True when this run simulated the `oracle_refill` counterfactual
    /// (Issue #243): Patrol tanks refilled to cap at each day boundary.
    ///
    /// `false` (the default, `#[serde(default)]` for records written before
    /// the axis existed) is the official refuel rule. Recorded because the
    /// counterfactual lives in `SimConfig`, which the final state does not
    /// retain — without this field a later re-aggregation could not tell a
    /// counterfactual run apart from evidence.
    #[serde(default)]
    pub oracle_refill: bool,
}

impl RunRecord {
    /// A record for a run that never had a usable case (the worker could not
    /// parse it, so the real case id is unavailable).
    #[must_use]
    pub fn failed(
        candidate_id: impl Into<String>,
        case_id: &str,
        source_commit_sha: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::unscored(
            candidate_id,
            &CaseSpec::unreadable(case_id),
            source_commit_sha,
            RunStatus::Invalid,
            Some(detail.into()),
            0,
        )
    }

    /// True when [`Self::scored`] is set.
    #[must_use]
    pub const fn is_scored(&self) -> bool {
        self.scored
    }

    /// The official score tuple. Only meaningful when [`Self::is_scored`];
    /// otherwise the fields are zero.
    #[must_use]
    pub fn to_score(&self) -> OfficialScore {
        OfficialScore::new(
            self.variety,
            self.daily_variety_sum,
            self.total_balls,
            self.response_time_us / 1000,
        )
    }

    /// A record for a run that produced no score.
    #[must_use]
    pub fn unscored(
        candidate_id: impl Into<String>,
        case: &CaseSpec,
        source_commit_sha: impl Into<String>,
        status: RunStatus,
        status_detail: Option<String>,
        elapsed_us: u64,
    ) -> Self {
        Self {
            candidate_id: candidate_id.into(),
            case_id: case.case_id.clone(),
            seed: case.seed,
            generation_parameters: case.generation_parameters.clone(),
            source_commit_sha: source_commit_sha.into(),
            team: String::new(),
            variety: 0,
            daily_variety_sum: 0,
            total_balls: 0,
            response_time_us: 0,
            elapsed_us,
            timeout: matches!(status, RunStatus::Timeout),
            status,
            scored: false,
            status_detail,
            last_acquisition_at: None,
            illegal_action_count: 0,
            fuel_short_count: 0,
            day_budget_exceeded_days: Vec::new(),
            daily_response_us: Vec::new(),
            daily_varieties: Vec::new(),
            // No final state was reached, so there is no team to report —
            // `None` rather than a composition that was never observed.
            agent_kinds: None,
            agent_ball_counts: None,
            // No run happened, so no counterfactual was simulated.
            oracle_refill: false,
        }
    }
}

/// Configuration shared by every evaluation in a batch.
#[derive(Debug, Clone)]
pub struct EvalConfig {
    /// Commit SHA stamped onto every record.
    pub source_commit_sha: String,
    /// Team the candidate plays as.
    pub team: PlayerId,
    /// How long a single `(candidate, case)` run may take before it is
    /// killed. See [`HarnessDeadline`].
    ///
    /// This is a **safety net that kills a runaway worker**, not the official
    /// per-day answer window. The official budget is `daySeconds`, judged per
    /// day against `daily_response_us` (see `RunRecord::day_budget_exceeded_days`).
    /// One total deadline cannot stand in for N daily budgets: raising it
    /// would let a day that blew its own window look like a success, and
    /// lowering it would kill runs that were never actually late.
    pub harness_deadline: HarnessDeadline,
    /// How a run is executed when the deadline expires.
    pub isolation: Isolation,
    /// Agent-kind composition pattern for every run in the batch
    /// (Issue #235's `--kinds`).
    ///
    /// `None` is the default: [`engine::from_wire`]'s historical
    /// even-slot-patrol / odd-slot-supply alternation, byte-for-byte. A value
    /// is a *pattern* expanded per case with [`crate::kinds::resolve_kinds`],
    /// because the corpus fields teams of 3, 4, 5, 6 and 8 agents.
    pub kinds: Option<Vec<engine::model::AgentKind>>,
    /// Maximum number of `(candidate, case)` evaluations running concurrently.
    /// `1` (default) preserves the historical sequential behaviour and is the
    /// regression baseline. Values > 1 use a bounded worker pool.
    ///
    /// **Response-time fairness**: when `jobs > 1`, CPU contention inflates
    /// `response_time_us`. Axes 1–3 (variety, daily_variety_sum, total_balls)
    /// are unaffected; axis 4 must be re-measured sequentially for final
    /// ranking (Issue #331).
    pub jobs: usize,
    /// Counterfactual: top every Patrol tank up to `fuel_cap` at each day
    /// boundary (Issue #243's `oracle_refill`).
    ///
    /// **Default `false` — byte-identical, and never a default evidence
    /// path.** The official rule refuels only on supply contact, so this
    /// knob exists solely to measure how much a result is explained by fuel
    /// exhaustion versus the absence of supply rendezvous (an ablation axis,
    /// not a strategy). The flag travels to the `--worker` child so process
    /// isolation cannot silently drop it.
    pub oracle_refill: bool,
}

/// How the harness deadline for one run is chosen.
///
/// A flat default is wrong for this job: `daySeconds = [60, 60, 60]` is 180 s
/// of *legal* answering time, so a flat 60 s cap would kill a solver that is
/// comfortably inside its official window on every day. The default therefore
/// derives the deadline from the case itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessDeadline {
    /// `sum(daySeconds) + margin` — a legal run is never cut short, and a
    /// genuinely hung run is killed after the official window closes.
    CaseWindow {
        /// Slack on top of the official window, to absorb process startup and
        /// serialization rather than having them eat the solver's budget.
        margin: Duration,
    },
    /// An absolute deadline regardless of the case.
    ///
    /// Explicit override: use it to fail fast instead of waiting out the
    /// official window. Never the default, because it is where false
    /// timeouts come from.
    Fixed(Duration),
}

/// Fallback window for a case that declares no usable `daySeconds`.
///
/// Such a case is already rejected by `engine::from_wire`; this only keeps
/// the deadline finite while the run is being refused.
const UNKNOWN_CASE_WINDOW: Duration = Duration::from_secs(60);

impl HarnessDeadline {
    /// The default: derive from the case, with 5 s of overhead slack.
    #[must_use]
    pub const fn case_derived(margin: Duration) -> Self {
        Self::CaseWindow { margin }
    }

    /// Resolve the policy against one case.
    #[must_use]
    pub fn resolve(&self, case: &CaseSpec) -> Duration {
        match *self {
            Self::Fixed(d) => d,
            Self::CaseWindow { margin } => {
                let total_secs: u64 = case.day_seconds().iter().map(|secs| u64::from(*secs)).sum();
                if total_secs == 0 {
                    return UNKNOWN_CASE_WINDOW + margin;
                }
                Duration::from_secs(total_secs).saturating_add(margin)
            }
        }
    }
}

impl Default for HarnessDeadline {
    fn default() -> Self {
        Self::CaseWindow { margin: Duration::from_secs(5) }
    }
}

impl EvalConfig {
    /// Build a config, resolving the commit SHA from the environment when the
    /// caller does not pass one.
    ///
    /// Defaults to [`Isolation::Thread`]. Prefer [`Self::with_process_isolation`]
    /// for batches: only a child process can be killed when it overruns.
    #[must_use]
    pub fn new(team: PlayerId, harness_deadline: HarnessDeadline) -> Self {
        Self {
            source_commit_sha: source_commit_sha(),
            team,
            harness_deadline,
            isolation: Isolation::Thread,
            kinds: None,
            oracle_refill: false,
            jobs: 1,
        }
    }

    /// Enable the `oracle_refill` counterfactual (Issue #243).
    ///
    /// Off by default; call this only from an analysis invocation that is
    /// deliberately measuring the fuel axis.
    #[must_use]
    pub fn with_oracle_refill(mut self, oracle_refill: bool) -> Self {
        self.oracle_refill = oracle_refill;
        self
    }

    /// Set the agent-kind composition pattern (Issue #235's `--kinds` axis).
    #[must_use]
    pub fn with_kinds(mut self, kinds: Option<Vec<engine::model::AgentKind>>) -> Self {
        self.kinds = kinds;
        self
    }

    /// The kind composition this config fields for a team of `n_agents`
    /// slots: the expanded pattern, or [`engine::from_wire::default_agent_kinds`].
    #[must_use]
    pub fn kinds_for(&self, n_agents: usize) -> Vec<engine::model::AgentKind> {
        crate::kinds::resolve_kinds(self.kinds.as_deref(), n_agents)
    }

    /// Same as [`Self::new`], but each run executes in a child `procon-eval`
    /// process that is **killed** when it overruns the deadline.
    #[must_use]
    pub fn with_process_isolation(mut self, worker_exe: impl Into<std::path::PathBuf>) -> Self {
        self.isolation = Isolation::Process { worker_exe: worker_exe.into() };
        self
    }
}

/// How a single `(candidate, case)` run is executed — specifically, what
/// happens when it overruns [`EvalConfig::harness_deadline`].
#[derive(Debug, Clone)]
pub enum Isolation {
    /// Run on an in-process worker thread.
    ///
    /// A timeout is *reported*, but Rust threads cannot be forcibly
    /// cancelled: the solver keeps burning CPU after the record is written.
    /// Acceptable for a trusted, hand-picked candidate; not acceptable for a
    /// sweep, where one hung candidate would corrupt the timing of every run
    /// behind it.
    Thread,
    /// Run in a child process and **kill it** on timeout.
    ///
    /// This is what provides the guarantee that nothing keeps computing after
    /// a timeout: the child is terminated and reaped, so CPU and memory are
    /// released before the next run starts.
    Process {
        /// The `procon-eval` executable, which has a `--worker` mode.
        worker_exe: std::path::PathBuf,
    },
}

/// Resolve the source commit SHA.
///
/// Reads `PROCON_SOURCE_COMMIT_SHA`. Falls back to `"unknown"` so tests and
/// ad-hoc runs still produce a well-formed record; callers that care about
/// reproducibility should set the variable (the `just` targets do).
#[must_use]
pub fn source_commit_sha() -> String {
    std::env::var("PROCON_SOURCE_COMMIT_SHA").unwrap_or_else(|_| "unknown".to_owned())
}

/// The canonical evaluator: candidate × case → [`RunRecord`].
#[derive(Debug, Clone)]
pub struct Evaluator {
    config: EvalConfig,
}

impl Evaluator {
    /// Construct an evaluator.
    #[must_use]
    pub const fn new(config: EvalConfig) -> Self {
        Self { config }
    }

    /// Borrow the configuration.
    #[must_use]
    pub fn config(&self) -> &EvalConfig {
        &self.config
    }

    /// Evaluate one `(candidate, case)` pair.
    ///
    /// Never panics and never propagates a candidate failure: panics,
    /// timeouts and malformed plans all come back as a [`RunRecord`] with the
    /// appropriate [`RunStatus`].
    pub fn evaluate_one(&self, candidate: &Candidate, case: &CaseSpec) -> RunRecord {
        let record = match &self.config.isolation {
            // Child process: killed on timeout, so nothing keeps computing
            // after the record is written.
            Isolation::Process { worker_exe } => self.run_in_process(worker_exe, candidate, case),
            // Same-process thread: a timeout is *reported* but not enforced.
            Isolation::Thread => self.run_on_thread(candidate, case),
        };
        self.stamp_identity(record, candidate, case)
    }

    /// Normalise the identity fields of a record produced by either
    /// isolation mode.
    ///
    /// Doing this in exactly one place is what makes a worker-side failure
    /// record — or a run that failed before the case was even readable — come
    /// back with the parent's real ids rather than whatever the child guessed.
    fn stamp_identity(
        &self,
        mut record: RunRecord,
        candidate: &Candidate,
        case: &CaseSpec,
    ) -> RunRecord {
        candidate.id().clone_into(&mut record.candidate_id);
        record.case_id.clone_from(&case.case_id);
        record.seed = case.seed;
        record.generation_parameters.clone_from(&case.generation_parameters);
        record.source_commit_sha.clone_from(&self.config.source_commit_sha);
        self.config.team.as_str().clone_into(&mut record.team);
        record
    }

    /// Run one pair in a child `procon-eval --worker` process and kill it if
    /// it overruns [`EvalConfig::harness_deadline`].
    ///
    /// This is the mode that holds the "no compute remains after a timeout"
    /// guarantee: the child is terminated and reaped, releasing its CPU and
    /// memory before the next run starts. Without it a single hung candidate
    /// would keep burning a core for as long as the batch lasts, and its
    /// noise would corrupt the timing of every run behind it.
    fn run_in_process(
        &self,
        worker_exe: &std::path::Path,
        candidate: &Candidate,
        case: &CaseSpec,
    ) -> RunRecord {
        let candidate_id = candidate.id();
        let started = Instant::now();
        let elapsed_us = |t: Instant| micros_u64(t.elapsed().as_micros());
        let failed = |status: RunStatus, detail: String, elapsed_us: u64| {
            RunRecord::unscored(
                candidate_id.clone(),
                case,
                self.config.source_commit_sha.clone(),
                status,
                Some(detail),
                elapsed_us,
            )
        };

        let case_json = match serde_json::to_string(case) {
            Ok(v) => v,
            Err(e) => {
                return failed(
                    RunStatus::Invalid,
                    format!("case could not be serialised for the worker: {e}"),
                    0,
                );
            }
        };
        let deadline = self.config.harness_deadline.resolve(case);
        let child = match self.spawn_worker(worker_exe, &candidate_id, case_json, deadline) {
            Ok(c) => c,
            Err(e) => return failed(RunStatus::Invalid, e, 0),
        };

        match read_child(child, deadline) {
            WorkerOutcome::TimedOut => failed(
                RunStatus::Timeout,
                format!(
                    "exceeded harness_deadline of {}ms; worker process was killed",
                    deadline.as_millis()
                ),
                elapsed_us(started),
            ),
            WorkerOutcome::Failed(detail) => failed(RunStatus::Panic, detail, elapsed_us(started)),
            WorkerOutcome::Done(stdout) => {
                let line = stdout.lines().next_back().unwrap_or_default().to_string();
                if line.is_empty() {
                    return failed(
                        RunStatus::Panic,
                        "evaluation worker produced no output (it likely panicked before reporting)"
                            .to_owned(),
                        elapsed_us(started),
                    );
                }
                match serde_json::from_str::<RunRecord>(&line) {
                    Ok(mut record) => {
                        // The worker measures its own wall clock; keep the
                        // parent's view so every record in a batch is timed
                        // the same way. Identity is normalised afterwards by
                        // `stamp_identity`.
                        record.elapsed_us = elapsed_us(started);
                        record
                    }
                    Err(e) => failed(
                        RunStatus::Panic,
                        format!("evaluation worker returned malformed output: {e}"),
                        elapsed_us(started),
                    ),
                }
            }
        }
    }

    /// Build the `procon-eval --worker` child process.
    fn spawn_worker(
        &self,
        worker_exe: &std::path::Path,
        candidate_id: &str,
        case_json: String,
        deadline: Duration,
    ) -> Result<std::process::Child, String> {
        let deadline_ms = u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX);
        std::process::Command::new(worker_exe)
            .arg("--worker")
            .arg("--candidate-id")
            .arg(candidate_id)
            .arg("--case-json")
            .arg(case_json)
            .arg("--team")
            .arg(self.config.team.as_str())
            .arg("--source-commit-sha")
            .arg(&self.config.source_commit_sha)
            // The kind composition travels as the pattern the batch declared,
            // so the child expands it against the very same case body.
            .arg("--kinds")
            .arg(self.config.kinds.as_ref().map_or_else(String::new, |p| render_kinds(p)))
            .arg("--harness-timeout-ms")
            .arg(deadline_ms.to_string())
            // Analysis-only counterfactual (Issue #243): forwarded explicitly
            // so a batch that asked for `oracle_refill` does not silently
            // fall back to the official refuel rule under process isolation.
            .args(self.config.oracle_refill.then_some("--oracle-refill"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start evaluation worker {}: {e}", worker_exe.display()))
    }

    /// Run one pair on an in-process worker thread.
    fn run_on_thread(&self, candidate: &Candidate, case: &CaseSpec) -> RunRecord {
        let candidate_id = candidate.id();
        let sha = self.config.source_commit_sha.clone();

        // A candidate that cannot even be constructed is an `Invalid` run,
        // not a batch abort.
        let solver = match candidate.build_solver() {
            Ok(s) => s,
            Err(err) => {
                return RunRecord::unscored(
                    candidate_id,
                    case,
                    sha,
                    RunStatus::Invalid,
                    Some(err.to_string()),
                    0,
                );
            }
        };

        // Expand the batch's `--kinds` pattern for *this* case's team size
        // before converting: the pattern is what the batch declared, this is
        // what the board actually fields.
        let kinds = self.config.kinds_for(case.setting.agents.len());
        let setting = match case.to_engine_setting_with_kinds(&self.config.team, Some(&kinds)) {
            Ok(s) => s,
            Err(err) => {
                return RunRecord::unscored(
                    candidate_id,
                    case,
                    sha,
                    RunStatus::Invalid,
                    Some(err.to_string()),
                    0,
                );
            }
        };

        // Traffic thresholds are a property of the *case*. Taking them from
        // `SimConfig::default()` instead would evaluate the case under a
        // different promotion/jamming schedule than it declares.
        let mut sim_config = match case.sim_config() {
            Ok(c) => c,
            Err(err) => {
                return RunRecord::unscored(
                    candidate_id,
                    case,
                    sha,
                    RunStatus::Invalid,
                    Some(err.to_string()),
                    0,
                );
            }
        };
        // The counterfactual rides on the batch config, never on the case: a
        // case file is a board, and `oracle_refill` is an analysis axis
        // (Issue #243). With the default `false`, `sim_config` stays exactly
        // as the case declared it.
        sim_config.oracle_refill = self.config.oracle_refill;

        self.run_isolated(candidate_id, case, solver, setting, sim_config)
    }

    /// Evaluate the full cross product of candidates × cases.
    ///
    /// The order is cases-major then candidates, so the result is stable for a
    /// given input order. Every pair yields a record — a failure in one never
    /// short-circuits the rest.
    pub fn evaluate_all(&self, candidates: &[Candidate], cases: &[CaseSpec]) -> Vec<RunRecord> {
        let jobs = self.config.jobs.max(1);
        if jobs == 1 {
            // Sequential: historical order (case-major, then candidate).
            let mut out = Vec::with_capacity(candidates.len().saturating_mul(cases.len()));
            for case in cases {
                for candidate in candidates {
                    out.push(self.evaluate_one(candidate, case));
                }
            }
            return out;
        }
        self.evaluate_all_parallel(candidates, cases, jobs)
    }

    /// Parallel evaluation with a bounded worker pool (Issue #331).
    ///
    /// Jobs are `(case_idx, candidate_idx)` pairs. Workers pull from a shared
    /// queue via a mutex. Results are stored in a pre-allocated slice indexed
    /// by `case_idx * n_candidates + candidate_idx`, so the output order is
    /// deterministic regardless of completion order.
    ///
    /// **Axis-4 caveat**: `response_time_us` is measured under CPU contention
    /// and must NOT be used for final ranking. Re-measure sequentially.
    fn evaluate_all_parallel(
        &self,
        candidates: &[Candidate],
        cases: &[CaseSpec],
        jobs: usize,
    ) -> Vec<RunRecord> {
        let n_candidates = candidates.len();
        let n_cases = cases.len();
        let total = n_candidates * n_cases;
        let jobs = jobs.min(total).max(1);

        // Shared job queue: (case_idx, candidate_idx).
        let queue: std::sync::Arc<std::sync::Mutex<Vec<(usize, usize)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(
                (0..n_cases)
                    .flat_map(|ci| (0..n_candidates).map(move |ki| (ci, ki)))
                    .collect(),
            ));

        // Pre-allocated result slots (None until filled).
        let results: std::sync::Arc<std::sync::Mutex<Vec<Option<RunRecord>>>> =
            std::sync::Arc::new(std::sync::Mutex::new((0..total).map(|_| None).collect()));

        std::thread::scope(|scope| {
            for _ in 0..jobs {
                let queue = std::sync::Arc::clone(&queue);
                let results = std::sync::Arc::clone(&results);
                scope.spawn(move || loop {
                    let job = {
                        let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                        q.pop()
                    };
                    let Some((ci, ki)) = job else { break };
                    let record = self.evaluate_one(&candidates[ki], &cases[ci]);
                    let slot = ci * n_candidates + ki;
                    let mut r = results.lock().unwrap_or_else(|e| e.into_inner());
                    r[slot] = Some(record);
                });
            }
        });

        // Collect and unwrap: every slot must be filled.
        let mut out = Vec::with_capacity(total);
        let mut r = results.lock().unwrap_or_else(|e| e.into_inner());
        for slot in r.iter_mut() {
            out.push(slot.take().expect("every (case, candidate) slot filled"));
        }
        out
    }

    /// Run one pair on a worker thread with a wall deadline.
    fn run_isolated(
        &self,
        candidate_id: String,
        case: &CaseSpec,
        solver: Box<dyn Solver>,
        setting: MatchSetting,
        sim_config: SimConfig,
    ) -> RunRecord {
        let sha = self.config.source_commit_sha.clone();
        let sim = MatchSim::new(sim_config);
        let started = Instant::now();

        let (tx, rx) = mpsc::channel();
        // A refused thread spawn must not abort the batch: `evaluate_one`'s
        // contract is "never panics, never propagates a candidate failure",
        // and dropping `tx` here makes the `Disconnected` arm below record a
        // failed run instead of unwinding through `evaluate_all`.
        let spawn_failure = thread::Builder::new()
            .name(format!("bench-eval-{candidate_id}"))
            .spawn(move || {
                let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                    let timed = TimingSolver::new(solver);
                    let result = sim.run(setting, &timed);
                    (result, timed.finish())
                }));
                let _ = tx.send(outcome);
            })
            .err();

        let elapsed_us = |t: Instant| micros_u64(t.elapsed().as_micros());

        let deadline = self.config.harness_deadline.resolve(case);
        match rx.recv_timeout(deadline) {
            Err(mpsc::RecvTimeoutError::Timeout) => RunRecord::unscored(
                candidate_id,
                case,
                sha,
                RunStatus::Timeout,
                Some(format!("exceeded harness_deadline of {}ms", deadline.as_millis())),
                elapsed_us(started),
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => RunRecord::unscored(
                candidate_id,
                case,
                sha,
                RunStatus::Panic,
                Some(match spawn_failure {
                    Some(e) => format!("evaluation worker could not be spawned: {e}"),
                    None => "evaluation worker exited without a result".to_owned(),
                }),
                elapsed_us(started),
            ),
            Ok(Err(panic_payload)) => RunRecord::unscored(
                candidate_id,
                case,
                sha,
                RunStatus::Panic,
                Some(panic_message(&panic_payload)),
                elapsed_us(started),
            ),
            Ok(Ok((result, timings))) => {
                self.record_scored(candidate_id, case, result, timings, elapsed_us(started))
            }
        }
    }

    /// Fold a completed simulation into a scored [`RunRecord`].
    fn record_scored(
        &self,
        candidate_id: String,
        case: &CaseSpec,
        result: Result<RunResult, String>,
        timings: Timings,
        elapsed_us: u64,
    ) -> RunRecord {
        let sha = self.config.source_commit_sha.clone();

        let (run, status_detail) = match result {
            Ok(run) => (run, None),
            Err(err) => {
                return RunRecord::unscored(
                    candidate_id,
                    case,
                    sha,
                    RunStatus::Invalid,
                    Some(err),
                    elapsed_us,
                );
            }
        };

        let daily_varieties = run.final_state.daily_varieties();
        let response_time_us: u64 = timings.daily_response_us.iter().sum();

        // A run is only a *clean* success if the plan was well-shaped AND
        // every action the solver asked for was legal. `MatchSim` downgrades
        // illegal actions to `Wait` so the match still completes, but under
        // the official rules an answer containing an invalid action is
        // rejected outright — so the run is `Invalid`, recorded with its
        // observed numbers but excluded from ranking.
        let mut reasons: Vec<String> = Vec::new();
        if timings.malformed_plan {
            reasons.push("returned a malformed day plan on at least one day".to_owned());
        }
        if run.illegal_action_count > 0 {
            reasons.push(format!(
                "emitted {} non-legal action(s) ({} from fuel exhaustion);                  the official rules reject such an answer",
                run.illegal_action_count, run.fuel_short_count
            ));
        }
        // Official per-day answer window: day i's answer must fit inside
        // `daySeconds[i]`. `MatchSim` has no clock, so it accepted the plan
        // regardless — if it arrived late the scores above are optimistic and
        // the run must not be ranked. This is separate from
        // `harness_deadline`, which only bounds how long a worker may hang.
        let day_budget_exceeded_days = match case.day_budget_us() {
            Ok(budgets) => days_over_budget(&timings.daily_response_us, &budgets),
            Err(err) => {
                reasons.push(format!("case has an unusable per-day budget: {err}"));
                Vec::new()
            }
        };
        if !day_budget_exceeded_days.is_empty() {
            let days = day_budget_exceeded_days
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            reasons.push(format!(
                "exceeded the official per-day answer window (daySeconds) on day(s) {days}; \
                 the server would have discarded those answers"
            ));
        }
        let status = if reasons.is_empty() { RunStatus::Success } else { RunStatus::Invalid };

        RunRecord {
            candidate_id,
            case_id: case.case_id.clone(),
            seed: case.seed,
            generation_parameters: case.generation_parameters.clone(),
            source_commit_sha: sha,
            team: self.config.team.as_str().to_owned(),
            variety: run.variety,
            daily_variety_sum: run.daily_variety_sum,
            total_balls: run.total_balls,
            response_time_us,
            elapsed_us,
            timeout: false,
            status,
            // `Invalid` is *observed but not rankable*: the numbers above are
            // real, but a submission the server would refuse must not win a
            // case. `AggregateReport` filters on this flag.
            scored: status == RunStatus::Success,
            status_detail: if reasons.is_empty() {
                status_detail
            } else {
                Some(reasons.join("; "))
            },
            last_acquisition_at: run.last_acquisition_at,
            illegal_action_count: run.illegal_action_count,
            fuel_short_count: run.fuel_short_count,
            day_budget_exceeded_days,
            daily_response_us: timings.daily_response_us,
            daily_varieties,
            // Read back from the simulated setting, not from the flag: what
            // the engine actually fielded is the evidence (Issue #235).
            agent_kinds: Some(
                run.final_state.setting.agents.iter().map(|a| a.kind).collect::<Vec<_>>(),
            ),
            // Per-agent ball tallies (Issue #243's "zero-arrival" axis):
            // read back from the final inventories, one entry per agent in
            // `ctx.agent_ids` order (ascending id, matching `agent_kinds`).
            agent_ball_counts: Some(agent_ball_counts(&run.final_state)),
            // The config that actually built *this* run's `SimConfig` — the
            // child's own config under process isolation, so a dropped
            // `--oracle-refill` flag shows up as `false` here instead of
            // being masked by the parent's claim.
            oracle_refill: self.config.oracle_refill,
        }
    }
}

/// Balls finished by each agent, in ascending agent-id order.
///
/// `total_balls` is the sum of this vector; the vector is what shows *which*
/// agents never reached a ball (Issue #243's zero-arrival metric).
fn agent_ball_counts(state: &engine::model::MatchState) -> Vec<u32> {
    let mut agents: Vec<&engine::model::Agent> = state.agents.iter().collect();
    agents.sort_by_key(|a| a.id);
    agents.iter().map(|a| a.total_inventory()).collect()
}

/// Per-run timing and contract-compliance evidence.
#[derive(Debug, Default)]
struct Timings {
    /// Wall latency of each `decide` call, in microseconds, in day order.
    daily_response_us: Vec<u64>,
    /// True when at least one day's plan was not well-shaped.
    malformed_plan: bool,
}

/// Wraps a candidate solver to measure how long each day's answer takes and
/// to flag plans that violate the shape contract.
///
/// This is instrumentation, not scoring: it never looks at the plan's content
/// beyond its shape, and it never produces a score.
struct TimingSolver {
    inner: Box<dyn Solver>,
    daily_response_us: Mutex<Vec<u64>>,
    malformed_plan: AtomicBool,
}

impl TimingSolver {
    fn new(inner: Box<dyn Solver>) -> Self {
        Self {
            inner,
            daily_response_us: Mutex::new(Vec::new()),
            malformed_plan: AtomicBool::new(false),
        }
    }

    fn finish(self) -> Timings {
        let daily_response_us = self.daily_response_us.into_inner().unwrap_or_default();
        Timings { daily_response_us, malformed_plan: self.malformed_plan.load(Ordering::SeqCst) }
    }
}

impl Solver for TimingSolver {
    fn name(&self) -> &'static str {
        "timing-wrapper"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let started = Instant::now();
        let plan = self.inner.decide(ctx);
        let took = micros_u64(started.elapsed().as_micros());

        if let Ok(mut buf) = self.daily_response_us.lock() {
            buf.push(took);
        }
        // A day plan must cover exactly `agent_count * day_steps` actions in
        // `ctx.agent_ids` order. Anything else means the candidate broke the
        // contract; MatchSim degrades it to all-wait, but we flag it.
        if !plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps) {
            self.malformed_plan.store(true, Ordering::SeqCst);
        }
        plan
    }
}

/// Days (zero-based) whose measured answer time exceeded that day's budget.
///
/// This is the official per-day validity rule (Q&A Q3): an answer that
/// arrives after `daySeconds[i]` is not accepted, so any day listed here
/// makes the run unrankable. Budgets and measurements are both in
/// microseconds; a missing entry on either side means that day has nothing to
/// judge it against and is skipped rather than guessed.
#[must_use]
pub fn days_over_budget(daily_response_us: &[u64], budget_us: &[u64]) -> Vec<u32> {
    daily_response_us
        .iter()
        .zip(budget_us)
        .enumerate()
        .filter(|(_, (took, budget))| u128::from(**took) > u128::from(**budget))
        .map(|(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
        .collect()
}

/// How a worker child ended.
enum WorkerOutcome {
    /// It printed something.
    Done(String),
    /// It overran the deadline and was killed.
    TimedOut,
    /// The reader thread went away before any output arrived.
    Failed(String),
}

/// Wait for a worker child, killing it if it overruns `timeout`.
///
/// Killing happens on **every** path — including the success path, where the
/// child may not have exited yet after closing stdout — so a process is never
/// left behind.
fn read_child(mut child: std::process::Child, timeout: Duration) -> WorkerOutcome {
    // Drain stdout on a helper thread so a child that never closes the pipe
    // cannot wedge us; the child is killed below regardless.
    let (tx, rx) = mpsc::channel();
    let mut stdout = child.stdout.take();
    thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.read_to_string(&mut buf);
        }
        let _ = tx.send(buf);
    });

    let outcome = match rx.recv_timeout(timeout) {
        Ok(buf) => WorkerOutcome::Done(buf),
        Err(mpsc::RecvTimeoutError::Timeout) => WorkerOutcome::TimedOut,
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            WorkerOutcome::Failed("evaluation worker exited without a result".to_owned())
        }
    };

    // Whether we won the race or not, the child is finished with: terminate
    // and reap it so no orphan survives.
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

/// Saturating `u128 -> u64` for microsecond measurements.
fn micros_u64(us: u128) -> u64 {
    u64::try_from(us).unwrap_or(u64::MAX)
}

/// Extract a readable message from a panic payload.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "candidate panicked with a non-string payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `stamp_identity` deliberately overwrites the identity fields with the
    /// parent's own. That normalisation is useful — it makes a worker-side
    /// failure record carry the parent's ids — but it also means an assertion
    /// on `record.team` *after* `evaluate_one` cannot detect a broken `--team`
    /// hand-off. The worker-side check lives in
    /// `tests/canonical_evaluation.rs::worker_reports_the_team_it_was_given`,
    /// which inspects the child's raw JSON instead.
    #[test]
    fn stamp_identity_overwrites_the_parent_identity() {
        let registry = CandidateRegistry::builtin();
        let candidate = registry.build("wait").expect("wait is builtin");
        let case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");

        let evaluator = Evaluator::new(EvalConfig {
            source_commit_sha: "parent-sha".to_owned(),
            team: PlayerId::new("parent-team").expect("valid team"),
            harness_deadline: HarnessDeadline::default(),
            isolation: Isolation::Thread,
            kinds: None,
            oracle_refill: false,
        jobs: 1,
        });

        // Pretend a worker came back with its own (wrong) identity.
        let mut child_record = RunRecord::unscored(
            "child-guess",
            &case,
            "child-sha",
            RunStatus::Panic,
            Some("child failed".to_owned()),
            42,
        );
        child_record.team = "child-team".to_owned();

        let stamped = evaluator.stamp_identity(child_record, &candidate, &case);
        assert_eq!(stamped.candidate_id, candidate.id());
        assert_eq!(stamped.case_id, case.case_id);
        assert_eq!(stamped.source_commit_sha, "parent-sha");
        assert_eq!(stamped.team, "parent-team");
    }

    /// A record's `candidate_id` comes from exactly one place —
    /// [`Evaluator::stamp_identity`] — and reads the resolved candidate's id,
    /// which the registry canonicalises. So a spec written in a shuffled
    /// parameter order can never reach `runs.jsonl` as a second identity for
    /// one configuration. (No simulation runs here: identity stamping is the
    /// whole subject.)
    #[test]
    fn stamped_candidate_id_is_the_canonical_schema_order() {
        let registry = CandidateRegistry::builtin();
        let candidate = registry
            .build("loop:initial=greedy:iterations=32")
            .expect("a shuffled id is still a valid id");
        let case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");

        let evaluator = Evaluator::new(EvalConfig {
            source_commit_sha: "parent-sha".to_owned(),
            team: PlayerId::new("parent-team").expect("valid team"),
            harness_deadline: HarnessDeadline::default(),
            isolation: Isolation::Thread,
            kinds: None,
            oracle_refill: false,
        jobs: 1,
        });

        // Pretend a worker came back spelling the id its own way.
        let child_record = RunRecord::unscored(
            "loop:initial=greedy:iterations=32",
            &case,
            "child-sha",
            RunStatus::Panic,
            Some("child failed".to_owned()),
            42,
        );

        let stamped = evaluator.stamp_identity(child_record, &candidate, &case);
        assert_eq!(stamped.candidate_id, "loop:iterations=32:initial=greedy");
        assert_eq!(stamped.candidate_id, candidate.id());
    }

    #[test]
    fn days_over_budget_flags_only_the_late_days() {
        assert_eq!(days_over_budget(&[100, 1_000_001], &[1_000_000, 1_000_000]), vec![1]);
        assert_eq!(days_over_budget(&[999_999], &[1_000_000]), Vec::<u32>::new());
        assert_eq!(days_over_budget(&[1_000_000], &[1_000_000]), Vec::<u32>::new(), "equal fits");
        assert_eq!(days_over_budget(&[10, 20, 30], &[5, 25, 5]), vec![0, 2]);
    }

    #[test]
    fn days_over_budget_skips_days_it_cannot_judge() {
        // Shorter measurement list: the unpaired day is not guessed at.
        assert_eq!(days_over_budget(&[10, 20], &[1, 1, 1]), vec![0, 1]);
        // Shorter budget list: the unmeasured day is not guessed at.
        assert_eq!(days_over_budget(&[10, 20, 30], &[1]), vec![0]);
        assert_eq!(days_over_budget(&[], &[1, 2]), Vec::<u32>::new());
    }
    use crate::candidate::CandidateRegistry;
    use crate::case::Corpus;

    fn config() -> EvalConfig {
        EvalConfig::new(PlayerId::new("player-a").unwrap(), HarnessDeadline::default())
    }

    fn registry() -> CandidateRegistry {
        CandidateRegistry::builtin()
    }

    #[test]
    fn wait_candidate_on_a_fixture_produces_a_scored_record() {
        let reg = registry();
        let candidate = reg.build("wait").unwrap();
        let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        let record = Evaluator::new(config()).evaluate_one(&candidate, &case);

        assert_eq!(record.candidate_id, "wait");
        assert_eq!(record.case_id, "fixture:small-8x8");
        assert_eq!(record.status, RunStatus::Success);
        assert!(!record.timeout);
        // Waiting scores zero on every competitive tier.
        assert_eq!(record.variety, 0);
        assert_eq!(record.daily_variety_sum, 0);
        assert_eq!(record.total_balls, 0);
        // But the run was real: days were simulated and timed.
        assert!(!record.daily_response_us.is_empty() || record.elapsed_us > 0);
    }

    /// Issue #243 — `oracle_refill` defaults to OFF end to end: an
    /// `EvalConfig::new` batch must produce records that say `false`, and
    /// only an explicit opt-in records `true`. The recorded flag is what
    /// keeps a counterfactual run distinguishable from evidence in
    /// `runs.jsonl`.
    #[test]
    fn oracle_refill_defaults_off_and_is_recorded_on_the_run() {
        assert!(!config().oracle_refill, "the counterfactual must default to off");

        let reg = registry();
        let candidate = reg.build("wait").unwrap();
        let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();

        let default_run = Evaluator::new(config()).evaluate_one(&candidate, &case);
        assert!(!default_run.oracle_refill, "a default batch must not record the counterfactual");

        let counterfactual = config().with_oracle_refill(true);
        assert!(counterfactual.oracle_refill, "with_oracle_refill(true) must flip the knob");
        let flagged_run = Evaluator::new(counterfactual).evaluate_one(&candidate, &case);
        assert!(flagged_run.oracle_refill, "the opt-in must reach the record");
    }

    /// Issue #243 — `agent_ball_counts` is the zero-arrival axis: one entry
    /// per agent, summing to `total_balls`, so "how many agents never
    /// collected" is readable straight off the record.
    #[test]
    fn agent_ball_counts_has_one_entry_per_agent_and_sums_to_total_balls() {
        let reg = registry();
        let candidate = reg.build("loop:iterations=2:initial=greedy").unwrap();
        let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        let record = Evaluator::new(config()).evaluate_one(&candidate, &case);
        assert_eq!(
            record.status,
            RunStatus::Success,
            "{}",
            record.status_detail.unwrap_or_default()
        );

        let counts = record.agent_ball_counts.expect("scored runs carry per-agent tallies");
        assert_eq!(counts.len(), 3, "small-8x8 fields 3 agents");
        assert_eq!(
            counts.iter().sum::<u32>(),
            record.total_balls,
            "the per-agent tallies must add up to tier 3"
        );
        // `wait` would be all zeros — prove the vector is not vacuous.
        assert!(counts.iter().sum::<u32>() > 0, "the greedy loop must collect at least one ball");
    }

    #[test]
    fn record_carries_case_provenance() {
        let reg = registry();
        let candidate = reg.build("wait").unwrap();
        let fixture = crate::fixture::MatchFixtureLoader::new().load_bundled("small-8x8").unwrap();
        let case = CaseSpec::generated(
            "generated:smoke:seed=7",
            7,
            serde_json::json!({"generator": "smoke"}),
            fixture.setting,
        )
        .unwrap();
        let record = Evaluator::new(config()).evaluate_one(&candidate, &case);
        assert_eq!(record.case_id, "generated:smoke:seed=7");
        assert_eq!(record.seed, Some(7));
        assert_eq!(record.generation_parameters.as_ref().unwrap()["generator"], "smoke");
    }

    #[test]
    fn evaluate_all_produces_the_full_cross_product() {
        let reg = registry();
        let candidates = vec![reg.build("wait").unwrap(), reg.build("loop:iterations=2").unwrap()];
        let corpus = Corpus::bundled_subset(&["small-8x8", "series-few"]).unwrap();
        let records = Evaluator::new(config()).evaluate_all(&candidates, &corpus.cases);
        assert_eq!(records.len(), 4);
        for r in &records {
            assert!(r.is_scored(), "{} on {}: {}", r.candidate_id, r.case_id, r.status);
        }
    }
}
