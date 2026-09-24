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
    /// Simulator configuration.
    pub sim: SimConfig,
}

impl EvalConfig {
    /// Build a config, resolving the commit SHA from the environment when the
    /// caller does not pass one.
    #[must_use]
    pub fn new(team: PlayerId, run_timeout: Duration) -> Self {
        Self {
            source_commit_sha: source_commit_sha(),
            team,
            run_timeout,
            sim: SimConfig::default(),
        }
    }
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

        self.run_isolated(candidate_id, case, solver, setting)
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
    ) -> RunRecord {
        let sha = self.config.source_commit_sha.clone();
        let sim = MatchSim::new(self.config.sim);
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
        let status = if timings.malformed_plan { RunStatus::Invalid } else { RunStatus::Success };

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
            scored: true,
            status_detail: if timings.malformed_plan {
                Some("candidate returned a malformed day plan on at least one day".to_owned())
            } else {
                status_detail
            },
            last_acquisition_at: run.last_acquisition_at,
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
