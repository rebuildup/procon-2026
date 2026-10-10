//! Regression tests for the parallel evaluation path (Issue #335).
//!
//! `--jobs N` may change *how* a batch runs, never *what* it concludes.
//! These tests pin the contract CI can verify without a long run:
//!
//! 1. **Consistency** — `jobs = 1` and `jobs > 1` produce identical records
//!    (scores, status, provenance) once nondeterministic timing fields are
//!    zeroed.
//! 2. **Order determinism** — the output vector is case-major then candidate,
//!    regardless of which worker finished first.
//! 3. **Timeout** — a run that overruns the harness deadline is reported as
//!    [`RunStatus::Timeout`] in both sequential and parallel modes.
//! 4. **Worker termination** — a parallel batch with N workers returns (the
//!    scoped pool joins every handle), and a panicking *solver* is isolated to
//!    its own record.
//!
//! Pool-level panic isolation caveat: the worker pool does **not** catch a
//! panic raised inside `Evaluator::evaluate_one` itself. `evaluate_one`'s
//! contract is "never panics"; solver panics are caught one level down in
//! `run_isolated` (`panic::catch_unwind`) and recorded as
//! [`RunStatus::Panic`]. A panic that escaped `evaluate_one` would unwind
//! through `std::thread::scope` and abort the batch. That is intentional —
//! a bug in the harness is not a candidate failure — and is documented here
//! rather than faked with a pool that silently swallows harness bugs.

use std::sync::Arc;
use std::time::Duration;

use bench::CandidateRegistry;
use bench::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use bench::case::{CaseSpec, Corpus};
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation, RunRecord, RunStatus};
use domain::PlayerId;
use engine::model::{AgentActions, Solver, SolverContext};

// ---------------------------------------------------------------------------
// Shared setup — tiny fixtures, fast solvers only
// ---------------------------------------------------------------------------

/// Two small bundled fixtures: enough for a cross product, cheap enough for CI.
const LIGHT_CASES: &[&str] = &["small-8x8", "series-few"];

/// Cheap real solvers: one that does nothing and one that searches a little.
const LIGHT_CANDIDATES: &[&str] = &["wait", "loop:iterations=2:initial=greedy"];

fn team() -> PlayerId {
    PlayerId::new("player-a").expect("valid team id")
}

fn config_with(jobs: usize, harness_deadline: HarnessDeadline) -> EvalConfig {
    EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        harness_deadline,
        isolation: Isolation::Thread,
        kinds: None,
        oracle_refill: false,
        jobs,
    }
}

fn evaluator(jobs: usize) -> Evaluator {
    Evaluator::new(config_with(jobs, HarnessDeadline::default()))
}

fn light_candidates() -> Vec<bench::candidate::Candidate> {
    let registry = CandidateRegistry::builtin();
    LIGHT_CANDIDATES
        .iter()
        .map(|id| registry.build(id).unwrap_or_else(|e| panic!("{id}: {e}")))
        .collect()
}

fn light_cases() -> Vec<CaseSpec> {
    Corpus::bundled_subset(LIGHT_CASES).expect("light corpus loads").cases
}

/// Wall-clock fields only. The parallel path may reorder *when* a worker ran,
/// never *what* it concluded — zeroing these is the whole point of the
/// consistency comparison.
fn strip_timing(mut r: RunRecord) -> RunRecord {
    r.elapsed_us = 0;
    r.response_time_us = 0;
    r.daily_response_us.fill(0);
    r
}

fn stripped(records: &[RunRecord]) -> Vec<RunRecord> {
    records.iter().cloned().map(strip_timing).collect()
}

// ---------------------------------------------------------------------------
// 1. Sequential vs parallel consistency
// ---------------------------------------------------------------------------

/// Same input, `jobs = 1` vs `jobs > 1` (and `jobs = 0` auto): identical
/// scores, status and provenance per `(candidate, case)`.
#[test]
fn sequential_and_parallel_agree_on_every_deterministic_field() {
    let candidates = light_candidates();
    let cases = light_cases();

    let sequential = stripped(&evaluator(1).evaluate_all(&candidates, &cases));
    assert_eq!(sequential.len(), LIGHT_CASES.len() * LIGHT_CANDIDATES.len());
    for r in &sequential {
        assert_eq!(
            r.status,
            RunStatus::Success,
            "baseline must be clean: {} on {}",
            r.candidate_id,
            r.case_id
        );
    }

    for jobs in [2, 4, 0] {
        let parallel = stripped(&evaluator(jobs).evaluate_all(&candidates, &cases));
        assert_eq!(
            sequential, parallel,
            "jobs={jobs} must match the sequential baseline field-for-field"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Result order determinism
// ---------------------------------------------------------------------------

/// The output order is the documented sequential order — case-major, then
/// candidate — no matter which worker finished first. The pool hands jobs out
/// LIFO, so completion order is roughly the reverse of the output order; the
/// pre-allocated slots must win.
#[test]
fn result_order_is_case_major_then_candidate_for_every_jobs_level() {
    let candidates = light_candidates();
    let cases = light_cases();
    let n_candidates = candidates.len();
    let n_cases = cases.len();

    for jobs in [1, 2, 3, 4, 0] {
        let records = evaluator(jobs).evaluate_all(&candidates, &cases);
        assert_eq!(records.len(), n_cases * n_candidates, "jobs={jobs} must fill every slot");

        for (i, r) in records.iter().enumerate() {
            let case_idx = i / n_candidates;
            let cand_idx = i % n_candidates;
            assert_eq!(
                r.case_id, cases[case_idx].case_id,
                "jobs={jobs} slot {i} must be the case-major-then-candidate case"
            );
            assert_eq!(
                r.candidate_id,
                candidates[cand_idx].id(),
                "jobs={jobs} slot {i} must be the case-major-then-candidate candidate"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Timeout in both modes
// ---------------------------------------------------------------------------

/// Test-only factory: a solver that refuses to answer inside any plausible
/// deadline. Synthetic and tiny — the run is cut at 50 ms, the solver would
/// have slept 2 s.
struct HangFactory;

impl CandidateFactory for HangFactory {
    fn algorithm(&self) -> &'static str {
        "hang-test"
    }
    fn params(&self) -> &'static [ParamSpec] {
        &[]
    }
    fn describe(&self) -> &'static str {
        "Test-only candidate that sleeps past the harness deadline."
    }
    fn build(&self, _spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        Ok(Box::new(HangSolver))
    }
}

struct HangSolver;

impl Solver for HangSolver {
    fn name(&self) -> &'static str {
        "hang-test"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        std::thread::sleep(Duration::from_secs(2));
        AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
    }
}

/// A run that overruns the harness deadline is [`RunStatus::Timeout`] whether
/// it ran alone or inside the pool. The deadline is a hard 50 ms so the test
/// is a synthetic spike, not a long run.
#[test]
fn timeout_is_reported_in_sequential_and_parallel_modes() {
    let mut registry = CandidateRegistry::builtin();
    registry.register(Arc::new(HangFactory));
    let hang = registry.build("hang-test").expect("test candidate builds");
    let cases = light_cases();
    let deadline = HarnessDeadline::Fixed(Duration::from_millis(50));

    for jobs in [1, 2] {
        let evaluator = Evaluator::new(config_with(jobs, deadline));
        let records = evaluator.evaluate_all(std::slice::from_ref(&hang), &cases);
        assert_eq!(records.len(), cases.len(), "jobs={jobs} must yield one record per case");
        for r in &records {
            assert_eq!(
                r.status,
                RunStatus::Timeout,
                "jobs={jobs} on {}: detail {:?}",
                r.case_id,
                r.status_detail
            );
            assert!(r.timeout, "jobs={jobs} on {} must set the timeout flag", r.case_id);
            assert!(!r.is_scored(), "a timeout is never ranked");
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Worker termination + solver-panic isolation
// ---------------------------------------------------------------------------

/// Test-only factory: a solver that panics on the first `decide`.
struct PanicFactory;

impl CandidateFactory for PanicFactory {
    fn algorithm(&self) -> &'static str {
        "panic-test"
    }
    fn params(&self) -> &'static [ParamSpec] {
        &[]
    }
    fn describe(&self) -> &'static str {
        "Test-only candidate that panics, to probe pool isolation."
    }
    fn build(&self, _spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        Ok(Box::new(PanicSolver))
    }
}

struct PanicSolver;

impl Solver for PanicSolver {
    fn name(&self) -> &'static str {
        "panic-test"
    }
    fn decide(&self, _ctx: &SolverContext<'_>) -> AgentActions {
        panic!("deliberate solver panic");
    }
}

/// Parallel mode with more workers than jobs must still exit cleanly: every
/// scoped handle is joined, and a panicking *solver* is isolated to its own
/// record by `run_isolated`'s `catch_unwind` — it does not poison the other
/// tasks in the pool.
///
/// See the module docs for the complementary caveat: a panic inside
/// `evaluate_one` itself is NOT isolated by the pool.
#[test]
fn parallel_pool_joins_and_isolates_solver_panics() {
    let mut registry = CandidateRegistry::builtin();
    registry.register(Arc::new(PanicFactory));
    let panic_candidate = registry.build("panic-test").expect("test candidate builds");
    let wait = registry.build("wait").expect("wait is builtin");
    let healthy = registry.build("loop:iterations=2:initial=greedy").expect("loop is builtin");

    let cases = light_cases();
    let n_cases = cases.len();
    // Three candidates × light cases, four workers: more workers than some
    // rows, more rows than workers on the panic path — both directions.
    let candidates = vec![wait, panic_candidate.clone(), healthy];
    let jobs = 4;

    // Returning from this call is itself half the assertion: `std::thread::scope`
    // joins every worker, so a leaked handle or a deadlocked queue hangs CI.
    let records = evaluator(jobs).evaluate_all(&candidates, &cases);
    assert_eq!(records.len(), n_cases * candidates.len());

    let panics: Vec<_> =
        records.iter().filter(|r| r.candidate_id == panic_candidate.id()).collect();
    assert_eq!(panics.len(), n_cases, "one panic record per case");
    for r in &panics {
        assert_eq!(r.status, RunStatus::Panic, "on {}: {:?}", r.case_id, r.status_detail);
        assert!(!r.is_scored());
        assert!(r.status_detail.is_some(), "the panic message must be recorded");
    }

    // The healthy candidates were not poisoned by the panicking sibling.
    for r in &records {
        if r.candidate_id == panic_candidate.id() {
            continue;
        }
        assert_eq!(
            r.status,
            RunStatus::Success,
            "{} on {} must be untouched by the panic: {:?}",
            r.candidate_id,
            r.case_id,
            r.status_detail
        );
    }
}
