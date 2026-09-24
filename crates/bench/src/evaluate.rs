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
    /// Per-day answer latency in microseconds, one entry per simulated day.
    pub daily_response_us: Vec<u64>,
    /// Distinct series acquired on each day, one entry per simulated day.
    ///
    /// `daily_variety_sum` is the sum of this vector (tier 2). Recording the
    /// breakdown lets a later re-aggregation change the policy without
    /// re-running the batch.
    pub daily_varieties: Vec<u32>,
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
            daily_response_us: Vec::new(),
            daily_varieties: Vec::new(),
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
    /// Wall deadline for a single `(candidate, case)` run.
    pub run_timeout: Duration,
    /// How a run is executed when the deadline expires.
    pub isolation: Isolation,
}

impl EvalConfig {
    /// Build a config, resolving the commit SHA from the environment when the
    /// caller does not pass one.
    ///
    /// Defaults to [`Isolation::Thread`]. Prefer [`Self::with_process_isolation`]
    /// for batches: only a child process can be killed when it overruns.
    #[must_use]
    pub fn new(team: PlayerId, run_timeout: Duration) -> Self {
        Self {
            source_commit_sha: source_commit_sha(),
            team,
            run_timeout,
            isolation: Isolation::Thread,
        }
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
/// happens when it overruns [`EvalConfig::run_timeout`].
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
        match &self.config.isolation {
            // Child process: killed on timeout, so nothing keeps computing
            // after the record is written.
            Isolation::Process { worker_exe } => self.run_in_process(worker_exe, candidate, case),
            // Same-process thread: a timeout is *reported* but not enforced.
            Isolation::Thread => self.run_on_thread(candidate, case),
        }
    }

    /// Run one pair in a child `procon-eval --worker` process and kill it if
    /// it overruns [`EvalConfig::run_timeout`].
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
        let child = match self.spawn_worker(worker_exe, &candidate_id, case_json) {
            Ok(c) => c,
            Err(e) => return failed(RunStatus::Invalid, e, 0),
        };

        match read_child(child, self.config.run_timeout) {
            WorkerOutcome::TimedOut => failed(
                RunStatus::Timeout,
                format!(
                    "exceeded run_timeout of {}ms; worker process was killed",
                    self.config.run_timeout.as_millis()
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
                        // the same way. Identity is re-stamped from the parent
                        // too, so a worker-side failure record still carries
                        // the real ids.
                        record.elapsed_us = elapsed_us(started);
                        record.candidate_id.clone_from(&candidate_id);
                        record.case_id.clone_from(&case.case_id);
                        record.seed = case.seed;
                        record.generation_parameters.clone_from(&case.generation_parameters);
                        record.source_commit_sha.clone_from(&self.config.source_commit_sha);
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
    ) -> Result<std::process::Child, String> {
        let timeout_ms = u64::try_from(self.config.run_timeout.as_millis()).unwrap_or(u64::MAX);
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
            .arg("--run-timeout-ms")
            .arg(timeout_ms.to_string())
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

        let setting = match case.to_engine_setting(&self.config.team) {
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
        let sim_config = match case.sim_config() {
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

        self.run_isolated(candidate_id, case, solver, setting, sim_config)
    }

    /// Evaluate the full cross product of candidates × cases.
    ///
    /// The order is cases-major then candidates, so the result is stable for a
    /// given input order. Every pair yields a record — a failure in one never
    /// short-circuits the rest.
    pub fn evaluate_all(&self, candidates: &[Candidate], cases: &[CaseSpec]) -> Vec<RunRecord> {
        let mut out = Vec::with_capacity(candidates.len().saturating_mul(cases.len()));
        for case in cases {
            for candidate in candidates {
                out.push(self.evaluate_one(candidate, case));
            }
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
        thread::Builder::new()
            .name(format!("bench-eval-{candidate_id}"))
            .spawn(move || {
                let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                    let timed = TimingSolver::new(solver);
                    let result = sim.run(setting, &timed);
                    (result, timed.finish())
                }));
                let _ = tx.send(outcome);
            })
            .expect("spawn evaluation worker");

        let elapsed_us = |t: Instant| micros_u64(t.elapsed().as_micros());

        match rx.recv_timeout(self.config.run_timeout) {
            Err(mpsc::RecvTimeoutError::Timeout) => RunRecord::unscored(
                candidate_id,
                case,
                sha,
                RunStatus::Timeout,
                Some(format!("exceeded run_timeout of {}ms", self.config.run_timeout.as_millis())),
                elapsed_us(started),
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => RunRecord::unscored(
                candidate_id,
                case,
                sha,
                RunStatus::Panic,
                Some("evaluation worker exited without a result".to_owned()),
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
        let status = if reasons.is_empty() { RunStatus::Success } else { RunStatus::Invalid };

        RunRecord {
            candidate_id,
            case_id: case.case_id.clone(),
            seed: case.seed,
            generation_parameters: case.generation_parameters.clone(),
            source_commit_sha: sha,
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
            daily_response_us: timings.daily_response_us,
            daily_varieties,
        }
    }
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
    use crate::candidate::CandidateRegistry;
    use crate::case::Corpus;

    fn config() -> EvalConfig {
        EvalConfig::new(PlayerId::new("player-a").unwrap(), Duration::from_secs(30))
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
