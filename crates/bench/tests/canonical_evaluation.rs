//! End-to-end proof of the canonical evaluation path.
//!
//! These tests deliberately drive **real** solvers through **real** fixtures
//! and assert on the resulting scores and durable records:
//!
//! ```text
//! real solver → actual actions → engine::MatchSim → actual score
//!             → ranking / result persistence
//! ```
//!
//! A test that returned a hand-written `ScoreReport` would prove nothing about
//! this path — the old suite did exactly that and is gone. What is verified
//! here:
//!
//! 1. 3 fixture cases × 2 real solver variants run through the whole path.
//! 2. The evaluator's tallies match what `engine::MatchSim` reports directly
//!    (solvers never score themselves).
//! 3. Competitive scores are reproducible: same commit + candidate config +
//!    case ⇒ identical tiers 1–3, ignoring wall-clock timing.
//! 4. A panicking candidate, a hanging candidate, and a candidate emitting
//!    invalid actions are each isolated to their own run record.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bench::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use bench::case::{CaseSpec, Corpus};
use bench::evaluate::{EvalConfig, Evaluator, RunRecord, RunStatus};
use bench::tournament::{TournamentConfig, TournamentRunner};
use bench::{AggregateReport, CandidateRegistry};
use domain::PlayerId;
use engine::model::{AgentActions, Solver, SolverContext};
use engine::{MatchSim, SimConfig};

// ---------------------------------------------------------------------------
// Shared setup
// ---------------------------------------------------------------------------

fn team() -> PlayerId {
    PlayerId::new("player-a").expect("valid team id")
}

fn evaluator() -> Evaluator {
    Evaluator::new(EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        run_timeout: Duration::from_secs(60),
        sim: SimConfig::default(),
    })
}

/// Three fixtures with genuinely different characteristics: a small warmup
/// map, a mid-size reference map, and a variety-heavy map.
const THREE_CASES: &[&str] = &["small-8x8", "medium-16x16", "series-many"];

/// Two **real** solver variants — the same `loop` algorithm at different
/// search budgets, so any score difference can only come from the parameter.
const TWO_VARIANTS: &[&str] =
    &["loop:iterations=1:initial=greedy", "loop:iterations=4:initial=greedy"];

// ---------------------------------------------------------------------------
// 1. Real solvers × real fixtures through the whole path
// ---------------------------------------------------------------------------

#[test]
fn three_cases_by_two_real_solver_variants_produce_real_scores() {
    let registry = CandidateRegistry::builtin();
    let candidates: Vec<_> = TWO_VARIANTS
        .iter()
        .map(|id| registry.build(id).unwrap_or_else(|e| panic!("{id}: {e}")))
        .collect();
    let corpus = Corpus::bundled_subset(THREE_CASES).expect("corpus loads");

    let records = evaluator().evaluate_all(&candidates, &corpus.cases);
    assert_eq!(records.len(), 6, "3 cases x 2 candidates");

    for r in &records {
        assert_eq!(
            r.status,
            RunStatus::Success,
            "{} on {}: {:?}",
            r.candidate_id,
            r.case_id,
            r.status
        );
        assert!(!r.timeout);
        // Per-day evidence is recorded for every simulated day.
        assert_eq!(
            r.daily_varieties.len(),
            r.daily_response_us.len(),
            "per-day vectors must align"
        );
        assert!(!r.daily_varieties.is_empty(), "at least one day was simulated");
    }

    // The real solvers must actually do something: at least one run collects
    // balls, and the wait baseline collects none.
    assert!(
        records.iter().any(|r| r.total_balls > 0),
        "real solvers must harvest something on at least one case"
    );

    let wait = registry.build("wait").unwrap();
    let wait_records = evaluator().evaluate_all(&[wait], &corpus.cases);
    for r in &wait_records {
        assert_eq!((r.variety, r.daily_variety_sum, r.total_balls), (0, 0, 0));
    }
}

#[test]
fn batch_writes_machine_readable_results_that_reaggregate() {
    let registry = CandidateRegistry::builtin();
    let ids: Vec<String> = TWO_VARIANTS.iter().map(|s| (*s).to_owned()).collect();
    let config = TournamentConfig::new(
        ids,
        Some(TWO_VARIANTS[0].to_owned()),
        "player-a",
        Duration::from_secs(60),
    )
    .with_source_commit_sha("test-commit")
    .with_output_dir(std::env::temp_dir().join("procon-bench-e2e-results"));

    let runner = TournamentRunner::new(config);
    let corpus = Corpus::bundled_subset(THREE_CASES).expect("corpus loads");
    let outcome = runner.run(&registry, &corpus).unwrap_or_else(|e| panic!("batch failed: {e}"));

    // 6 records, written and re-readable.
    assert_eq!(outcome.records.len(), 6);
    let paths = outcome.paths.clone().expect("output dir configured");
    let reread = bench::report::read_runs(&paths.runs).expect("runs.jsonl parses");
    assert_eq!(reread, outcome.records);

    // Every required durable field survives the round trip.
    for r in &reread {
        assert!(!r.candidate_id.is_empty());
        assert!(!r.case_id.is_empty());
        assert_eq!(r.source_commit_sha, "test-commit");
        assert!(r.is_scored());
    }

    // Aggregation is regenerable from the records alone.
    let regenerated = AggregateReport::build(reread, Some(TWO_VARIANTS[0]));
    assert_eq!(regenerated.cases, outcome.report.cases);
    assert_eq!(regenerated.candidates, outcome.report.candidates);
    for (a, b) in regenerated.per_case.iter().zip(&outcome.report.per_case) {
        assert_eq!(a.deterministic_projection(), b.deterministic_projection());
    }

    // Aggregates expose what the issue asks for.
    assert_eq!(outcome.report.per_case.len(), 3);
    for case in &outcome.report.per_case {
        assert_eq!(case.entries.len(), 2);
    }
    let summary = outcome.report.candidate(TWO_VARIANTS[0]).expect("baseline summary");
    assert_eq!(summary.runs, 3);
    assert_eq!(summary.wins + summary.draws + summary.losses + summary.not_compared, 3);

    // And a human-readable render exists.
    let md = outcome.to_markdown();
    assert!(md.contains(TWO_VARIANTS[0]));
    assert!(md.contains("fixture:small-8x8"));
}

// ---------------------------------------------------------------------------
// 2. Solvers never score themselves
// ---------------------------------------------------------------------------

#[test]
fn evaluator_score_matches_matchsim_run_result() {
    // Drive the very same solver through `MatchSim` directly and check the
    // evaluator reports the same tallies. If a solver could inject a score,
    // this would be the test that catches it.
    let registry = CandidateRegistry::builtin();
    let candidate = registry.build("loop:iterations=2:initial=sweep-row").unwrap();
    let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();

    let record = evaluator().evaluate_one(&candidate, &case);
    assert_eq!(record.status, RunStatus::Success);

    let solver = candidate.build_solver().expect("builds");
    let setting = case.to_engine_setting(&team()).expect("converts");
    let direct =
        MatchSim::new(SimConfig::default()).run(setting, solver.as_ref()).expect("sim runs");

    assert_eq!(record.variety, direct.variety);
    assert_eq!(record.daily_variety_sum, direct.daily_variety_sum);
    assert_eq!(record.total_balls, direct.total_balls);
    assert_eq!(record.last_acquisition_at, direct.last_acquisition_at);
    // And the per-day breakdown sums to the tier-2 aggregate.
    assert_eq!(record.daily_varieties.iter().sum::<u32>(), record.daily_variety_sum);
}

#[test]
fn candidate_exposes_no_way_to_declare_a_score() {
    // Compile-level evidence: the only thing a candidate produces is
    // `AgentActions`. This test constructs a solver whose `decide` is the
    // entire surface available to it and observes that the score comes back
    // from the simulator, not from anything the solver returned.
    let calls = Arc::new(AtomicU32::new(0));
    let probe = ProbeSolver { calls: Arc::clone(&calls) };

    let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
    let setting = case.to_engine_setting(&team()).expect("converts");
    let run = MatchSim::new(SimConfig::default()).run(setting, &probe).expect("sim runs");

    // The solver was asked for plans and never for a score.
    assert!(calls.load(Ordering::SeqCst) > 0);
    // The tallies come from the state the simulator produced.
    assert_eq!(run.variety, u32::try_from(run.final_state.series_taken.len()).unwrap_or(u32::MAX));
    assert_eq!(run.daily_variety_sum, run.final_state.daily_varieties().iter().sum::<u32>());
}

/// Minimal solver used to prove the contract: it can only return actions.
struct ProbeSolver {
    calls: Arc<AtomicU32>,
}

impl Solver for ProbeSolver {
    fn name(&self) -> &'static str {
        "probe"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        self.calls.fetch_add(1, Ordering::SeqCst);
        AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
    }
}

// ---------------------------------------------------------------------------
// 3. Determinism: same commit + candidate config + case ⇒ same competitive score
// ---------------------------------------------------------------------------

#[test]
fn competitive_score_is_reproducible_across_repeated_runs() {
    let registry = CandidateRegistry::builtin();
    let candidates: Vec<_> = TWO_VARIANTS.iter().map(|id| registry.build(id).unwrap()).collect();
    let corpus = Corpus::bundled_subset(THREE_CASES).expect("corpus loads");

    let first = evaluator().evaluate_all(&candidates, &corpus.cases);
    let second = evaluator().evaluate_all(&candidates, &corpus.cases);

    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(a.candidate_id, b.candidate_id);
        assert_eq!(a.case_id, b.case_id);
        assert_eq!(a.status, b.status);
        // Tiers 1-3 are the deterministic competitive score. Wall-clock
        // timing (`response_time_us`, `elapsed_us`) is excluded on purpose:
        // it is measured and therefore run-varying.
        assert_eq!(
            (a.variety, a.daily_variety_sum, a.total_balls),
            (b.variety, b.daily_variety_sum, b.total_balls),
            "competitive score must reproduce for {} on {}",
            a.candidate_id,
            a.case_id
        );
        // The per-day breakdown is deterministic too — it is derived from the
        // simulated state, not from a clock.
        assert_eq!(a.daily_varieties, b.daily_varieties);
        assert_eq!(a.last_acquisition_at, b.last_acquisition_at);
    }

    // Ranking derived from two independent batches is identical, on the
    // deterministic projection. The full entries differ because tier 4
    // (`response_time_us`) is measured; that is expected and documented.
    let r1 = AggregateReport::build(first, Some(TWO_VARIANTS[0]));
    let r2 = AggregateReport::build(second, Some(TWO_VARIANTS[0]));
    for (a, b) in r1.per_case.iter().zip(&r2.per_case) {
        assert_eq!(
            a.deterministic_projection(),
            b.deterministic_projection(),
            "ranking must reproduce for {}",
            a.case_id
        );
    }
}

#[test]
fn seeded_loop_candidate_is_reproducible() {
    // `loop:initial=random:seed=N` is the reproducibility-critical candidate
    // id: the same seed must give the same competitive score.
    let registry = CandidateRegistry::builtin();
    let c = registry.build("loop:iterations=3:initial=random:seed=42").unwrap();
    let case = CaseSpec::from_bundled_fixture("series-many").expect("fixture");

    let a = evaluator().evaluate_one(&c, &case);
    let b = evaluator().evaluate_one(&c, &case);
    assert_eq!(
        (a.variety, a.daily_variety_sum, a.total_balls),
        (b.variety, b.daily_variety_sum, b.total_balls)
    );
}

// ---------------------------------------------------------------------------
// 4. Failure isolation
// ---------------------------------------------------------------------------

/// Registry with the pathological candidate used below. One factory, three
/// modes — which is also how the parameter axis is meant to work.
fn registry_with_failures() -> CandidateRegistry {
    let mut registry = CandidateRegistry::builtin();
    registry.register(std::sync::Arc::new(BrokenFactory));
    registry
}

#[test]
fn panic_timeout_and_invalid_are_isolated_per_run() {
    let registry = registry_with_failures();
    let corpus = Corpus::bundled_subset(&["small-8x8"]).expect("corpus loads");

    // A short deadline so the hanging candidate is caught as a timeout.
    let evaluator = Evaluator::new(EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        run_timeout: Duration::from_millis(400),
        sim: SimConfig::default(),
    });

    let ids = [
        "wait",
        "broken:mode=panic",
        "broken:mode=hang",
        "broken:mode=malformed",
        "loop:iterations=1:initial=greedy",
    ];
    let candidates: Vec<_> =
        ids.iter().map(|id| registry.build(id).unwrap_or_else(|e| panic!("{id}: {e}"))).collect();

    // The whole batch completes despite three pathological candidates.
    let records = evaluator.evaluate_all(&candidates, &corpus.cases);
    assert_eq!(records.len(), 5);

    let by_id = |id: &str| -> &RunRecord {
        records.iter().find(|r| r.candidate_id == id).expect("record present")
    };

    // Each failure is attributed to its own candidate and nothing else broke.
    assert_eq!(by_id("wait").status, RunStatus::Success);
    assert_eq!(by_id("broken:mode=panic").status, RunStatus::Panic);
    assert!(by_id("broken:mode=panic").status_detail.is_some());
    assert_eq!(by_id("broken:mode=hang").status, RunStatus::Timeout);
    assert!(by_id("broken:mode=hang").timeout);
    assert_eq!(by_id("broken:mode=malformed").status, RunStatus::Invalid);
    // The healthy candidate still produced a real score.
    assert_eq!(by_id("loop:iterations=1:initial=greedy").status, RunStatus::Success);

    // Unscored runs report zeros and are excluded from scoring, not faked.
    let panic_run = by_id("broken:mode=panic");
    assert_eq!((panic_run.variety, panic_run.total_balls), (0, 0));
    assert!(!panic_run.is_scored());

    // The malformed run still has a score (MatchSim degrades it to waits) but
    // is flagged so aggregation can count it.
    let malformed = by_id("broken:mode=malformed");
    assert!(malformed.is_scored());
    assert!(malformed.status_detail.is_some());

    // Aggregation counts failures without dropping the batch.
    let report = AggregateReport::build(records, Some("wait"));
    let broken = report.candidate("broken:mode=panic").expect("summary");
    assert_eq!(broken.panics, 1);
    assert_eq!(broken.scored, 0);
    assert_eq!(report.candidate("broken:mode=hang").unwrap().timeouts, 1);
    assert_eq!(report.candidate("broken:mode=malformed").unwrap().invalids, 1);
    // And the healthy candidate is untouched.
    let healthy = report.candidate("loop:iterations=1:initial=greedy").unwrap();
    assert_eq!(healthy.timeouts + healthy.invalids + healthy.panics, 0);
}

// ---------------------------------------------------------------------------
// Pathological candidates (test-only)
// ---------------------------------------------------------------------------

/// Test-only factory: one `broken` algorithm with a `mode` parameter, so the
/// three failure modes are three *candidates*, not three algorithms.
struct BrokenFactory;

impl CandidateFactory for BrokenFactory {
    fn algorithm(&self) -> &'static str {
        "broken"
    }
    fn params(&self) -> &'static [ParamSpec] {
        &[ParamSpec { key: "mode", required: true, doc: "panic | hang | malformed" }]
    }
    fn describe(&self) -> &'static str {
        "Test-only candidate that fails on purpose. mode=panic|hang|malformed"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        Ok(Box::new(ModeSolver { mode: mode_of(spec)? }))
    }
}

fn mode_of(spec: &CandidateSpec) -> Result<&'static str, CandidateError> {
    match spec.param("mode") {
        Some("panic") => Ok("panic"),
        Some("hang") => Ok("hang"),
        Some("malformed") => Ok("malformed"),
        other => Err(CandidateError::InvalidParameter {
            key: "mode".to_owned(),
            value: other.unwrap_or_default().to_owned(),
            reason: "expected panic | hang | malformed".to_owned(),
        }),
    }
}

struct ModeSolver {
    mode: &'static str,
}

impl Solver for ModeSolver {
    fn name(&self) -> &'static str {
        "broken"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        match self.mode {
            "panic" => panic!("deliberate panic from a candidate"),
            "hang" => {
                // Exceeds the 400ms deadline used by the isolation test.
                std::thread::sleep(Duration::from_secs(10));
                AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
            }
            // Wrong length and wrong agent order: fails `is_well_shaped`.
            _ => AgentActions::new(vec![engine::model::Action::wait(9)]),
        }
    }
}
