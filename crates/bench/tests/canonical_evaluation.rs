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
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation, RunRecord, RunStatus};
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

/// The `procon-eval` binary, for [`Isolation::Process`]. Cargo builds package
/// binaries before running that package's integration tests.
fn worker_exe() -> &'static str {
    env!("CARGO_BIN_EXE_procon-eval")
}

/// Every record in this suite carries the same commit, so this cannot fail.
fn agg(runs: Vec<RunRecord>, baseline: Option<&str>) -> AggregateReport {
    AggregateReport::build(runs, baseline).expect("all records share one commit")
}

/// Config for this suite. `isolation` defaults to a killed child process —
/// the same thing a real batch uses — because a timeout must *stop* the run,
/// not just stop waiting for it.
fn config(harness_deadline: HarnessDeadline, isolation: Isolation) -> EvalConfig {
    EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        harness_deadline,
        isolation,
        kinds: None,
        oracle_refill: false,
    }
}

fn evaluator() -> Evaluator {
    Evaluator::new(
        config(HarnessDeadline::default(), Isolation::Thread).with_process_isolation(worker_exe()),
    )
}

fn evaluator_with_timeout(isolation: Isolation, harness_deadline: HarnessDeadline) -> Evaluator {
    Evaluator::new(config(harness_deadline, isolation))
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
    let config = TournamentConfig::new(ids, Some(TWO_VARIANTS[0].to_owned()), "player-a", None)
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
        assert_ne!(r.candidate_id.len(), 0);
        assert_ne!(r.case_id.len(), 0);
        assert_eq!(r.source_commit_sha, "test-commit");
        assert!(r.is_scored());
    }

    // Aggregation is regenerable from the records alone.
    let regenerated = agg(reread, Some(TWO_VARIANTS[0]));
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
// 1b. Canonical candidate identity
// ---------------------------------------------------------------------------

/// Issue #186: a resolved candidate is identified by its canonical
/// (schema-ordered) id, and that is the id the durable record and the
/// aggregate carry. Writing the same configuration in another parameter
/// order must therefore not create a second identity for it.
#[test]
fn a_shuffled_candidate_id_is_recorded_and_aggregated_canonically() {
    let registry = CandidateRegistry::builtin();
    // Deliberately not in schema order — validity never depended on order,
    // and canonicalisation is what makes the identity unique.
    let candidate = registry
        .build("loop:initial=greedy:iterations=1")
        .expect("parameter order is not part of validity");
    assert_eq!(candidate.id(), "loop:iterations=1:initial=greedy");

    let case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
    let record = Evaluator::new(config(HarnessDeadline::default(), Isolation::Thread))
        .evaluate_one(&candidate, &case);
    assert_eq!(record.status, RunStatus::Success);
    assert_eq!(
        record.candidate_id, "loop:iterations=1:initial=greedy",
        "RunRecord.candidate_id must be the canonical id"
    );

    // The aggregate keys on the same canonical id as the record.
    let report = agg(vec![record], Some("loop:iterations=1:initial=greedy"));
    assert!(report.candidate("loop:iterations=1:initial=greedy").is_some());
    assert!(
        report.candidate("loop:initial=greedy:iterations=1").is_none(),
        "the non-canonical spelling must not be a separate aggregate entry"
    );
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
    let r1 = agg(first, Some(TWO_VARIANTS[0]));
    let r2 = agg(second, Some(TWO_VARIANTS[0]));
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

    // These three candidates are registered only inside this test binary, so
    // they cannot be built by the `procon-eval` worker — thread isolation is
    // the only way to exercise them. Real batches use process isolation, and
    // `process_isolation_kills_the_worker_on_timeout` below proves that path.
    let evaluator = evaluator_with_timeout(
        Isolation::Thread,
        HarnessDeadline::Fixed(Duration::from_millis(400)),
    );

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

    // The malformed run still produced numbers (MatchSim degrades the plan to
    // waits rather than aborting), but a submission the server would refuse
    // must not be rankable — so it is recorded with `scored = false`.
    let malformed = by_id("broken:mode=malformed");
    assert_eq!(malformed.status, RunStatus::Invalid);
    assert!(!malformed.is_scored(), "an unsound submission must not be rankable");
    assert!(
        malformed.status_detail.as_deref().unwrap_or_default().contains("malformed day plan"),
        "detail must name the contract violation: {:?}",
        malformed.status_detail
    );
    // The numbers are still kept as evidence — `MatchSim` degraded the plan
    // to waits and finished the match — they just do not win anything.
    assert!(malformed.status_detail.is_some());

    // Aggregation counts failures without dropping the batch.
    let report = agg(records, Some("wait"));
    let broken = report.candidate("broken:mode=panic").expect("summary");
    assert_eq!(broken.panics, 1);
    assert_eq!(broken.scored, 0);
    assert_eq!(report.candidate("broken:mode=hang").unwrap().timeouts, 1);
    assert_eq!(report.candidate("broken:mode=malformed").unwrap().invalids, 1);
    // And the healthy candidate is untouched.
    let healthy = report.candidate("loop:iterations=1:initial=greedy").unwrap();
    assert_eq!(healthy.timeouts + healthy.invalids + healthy.panics, 0);
}

#[test]
fn process_isolation_kills_the_worker_on_timeout() {
    // Uses a case no other test touches, so the process check below cannot
    // race against a concurrent run.
    let registry = CandidateRegistry::builtin();
    // The deadline has to be one the child cannot meet, and everything about
    // that bound was machine-dependent before: sizing it *inside* the
    // computation needs the workload slower than the deadline (but
    // "iterations" is an upper bound — refinement stops as soon as no move
    // improves — so a faster refine sails under a fixed 3 ms), while sizing
    // it just above process start-up races the child's own 0 ms timeout
    // report. A zero deadline removes the guesswork: the child can never
    // finish before the parent checks, so the kill/reap path this test is
    // actually about always runs.
    let candidate = registry.build("loop:iterations=128:initial=greedy").unwrap();
    let case = CaseSpec::from_bundled_fixture("large-32x32").unwrap();

    let evaluator = Evaluator::new(
        config(HarnessDeadline::Fixed(Duration::ZERO), Isolation::Thread)
            .with_process_isolation(worker_exe()),
    );

    let record = evaluator.evaluate_one(&candidate, &case);

    assert_eq!(record.status, RunStatus::Timeout, "detail: {:?}", record.status_detail);
    assert!(record.timeout);
    assert!(!record.is_scored());
    assert!(
        record.status_detail.as_deref().unwrap_or_default().contains("worker process was killed"),
        "the record must say the child was terminated: {:?}",
        record.status_detail
    );

    // The guarantee itself: no worker for this run is left behind.
    //
    // The pattern is built at runtime from the real executable path, so it
    // matches `<abs>/procon-eval --worker ...` and nothing else — a shell or
    // an editor with `--worker` somewhere in its own command line cannot
    // produce a false positive.
    let pattern = format!("{} --worker", worker_exe());
    let output = match std::process::Command::new("pgrep").args(["-f", &pattern]).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        // pgrep exits 1 with no output when nothing matched; anything else
        // means pgrep itself is unavailable, in which case the assertions
        // above are still the primary evidence.
        Ok(_) => String::new(),
        Err(_) => return,
    };
    // Every worker in this binary shares that command line — `evaluator()`,
    // `kinds_survive_process_isolation` and friends all spawn `--worker`
    // children concurrently — so keep only the ones running *this* case,
    // which travels in `--case-json`.
    let stray: Vec<String> =
        output.lines().filter(|line| line.contains("large-32x32")).map(ToOwned::to_owned).collect();
    assert!(stray.is_empty(), "a killed worker must not still be running (pids {stray:?})");
}

#[test]
fn worker_reports_the_team_it_was_given() {
    // Asserts on the record the *child* produced, before
    // `Evaluator::stamp_identity` normalises `team` from the parent's config.
    // The previous version of this test asserted after that stamping, so it
    // passed even when the worker ignored `--team` entirely — which is
    // exactly the regression it was meant to catch.
    let case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
    let out = std::process::Command::new(worker_exe())
        .arg("--worker")
        .arg("--candidate-id")
        .arg("wait")
        .arg("--case-json")
        .arg(serde_json::to_string(&case).expect("case serialises"))
        .arg("--team")
        .arg("raw-team")
        .arg("--source-commit-sha")
        .arg("test-commit")
        .arg("--harness-timeout-ms")
        .arg("60000")
        .output()
        .expect("worker starts");
    assert!(out.status.success(), "worker failed: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let line = stdout.lines().next_back().expect("worker printed a record");
    let raw: RunRecord = serde_json::from_str(line).expect("record parses");
    assert_eq!(raw.team, "raw-team", "the worker must read `--team`, not a flag nothing sends");
}

/// Issue #235 — the kind composition has to survive **process isolation**:
/// the parent expands nothing itself, it forwards `--kinds` and the child
/// reports back what it fielded. A batch run with process isolation (the
/// default for a real `procon-eval`) would otherwise silently fall back to
/// the historical alternation while the operator believed they had changed it.
#[test]
fn kinds_survive_process_isolation() {
    use engine::model::AgentKind;

    let evaluator = Evaluator::new(
        config(HarnessDeadline::default(), Isolation::Thread)
            .with_process_isolation(worker_exe())
            .with_kinds(Some(bench::kinds::parse_kinds("S").expect("pattern parses"))),
    );
    let case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
    let registry = CandidateRegistry::builtin();
    let candidate = registry.build("wait").expect("wait is builtin");

    let record = evaluator.evaluate_one(&candidate, &case);
    assert_eq!(record.status, RunStatus::Success, "{}", record.status_detail.unwrap_or_default());
    assert_eq!(
        record.agent_kinds,
        Some(vec![AgentKind::Supply; 3]),
        "small-8x8 fields 3 agents; `--kinds S` must reach the child and come back all-supply"
    );
}

/// Issue #243 — `oracle_refill` must survive **process isolation**: the
/// counterfactual lives in the child's `SimConfig`, so a parent that forgets
/// to forward `--oracle-refill` would silently run the official refuel rule
/// while the operator believed they had enabled the counterfactual. The flag
/// is read back from the record the child itself produced (before any parent
/// stamping) so the assertion cannot pass by accident.
#[test]
fn oracle_refill_survives_process_isolation() {
    let evaluator = Evaluator::new(
        config(HarnessDeadline::default(), Isolation::Thread)
            .with_process_isolation(worker_exe())
            .with_oracle_refill(true),
    );
    let case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
    let registry = CandidateRegistry::builtin();
    let candidate = registry.build("wait").expect("wait is builtin");

    let record = evaluator.evaluate_one(&candidate, &case);
    assert_eq!(record.status, RunStatus::Success, "{}", record.status_detail.unwrap_or_default());
    assert!(
        record.oracle_refill,
        "the child must have received `--oracle-refill` and recorded the counterfactual"
    );

    // And the default direction: with the knob off, the child records false.
    let default_eval = Evaluator::new(
        config(HarnessDeadline::default(), Isolation::Thread).with_process_isolation(worker_exe()),
    );
    let off = default_eval.evaluate_one(&candidate, &case);
    assert!(!off.oracle_refill, "an unflagged batch must stay on the official refuel rule");
}

#[test]
fn harness_deadline_is_derived_from_the_cases_official_window() {
    // `daySeconds = [60,60,60]` is 180 s of legal answering time. The flat
    // 60 s default that used to ship would have killed a run that is inside
    // its official budget on every single day.
    let mut case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
    case.setting.day_steps = vec![16, 16, 16];
    case.setting.day_seconds = vec![60, 60, 60];

    let derived = HarnessDeadline::CaseWindow { margin: Duration::from_secs(5) }.resolve(&case);
    assert!(
        derived > Duration::from_secs(60),
        "a flat 60 s cap would kill a legal 180 s run; derived deadline was {derived:?}"
    );
    assert!(derived >= Duration::from_secs(185), "expected window + margin, got {derived:?}");

    // An explicit override still means what it says.
    let fixed = HarnessDeadline::Fixed(Duration::from_millis(600)).resolve(&case);
    assert_eq!(fixed, Duration::from_millis(600));
}

#[test]
fn a_flat_deadline_kills_a_legal_run_but_the_case_window_does_not() {
    // Behavioural version of the test above, at a scale a test can afford:
    // 1.1 s of work inside a 2 s per-day budget — late for nobody.
    let registry = registry_with_failures();
    let candidate = registry.build("broken:mode=sleep").unwrap();

    let mut case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
    case.setting.day_seconds = vec![2];
    assert_eq!(case.setting.day_steps.len(), 1, "daySeconds must match daySteps");

    // The failure mode: one flat deadline that is shorter than the run.
    let flat = evaluator_with_timeout(
        Isolation::Thread,
        HarnessDeadline::Fixed(Duration::from_millis(600)),
    );
    let killed = flat.evaluate_one(&candidate, &case);
    assert_eq!(killed.status, RunStatus::Timeout, "the flat cap kills a legal run");

    // The fix: the deadline comes from the case's own window.
    let derived = evaluator_with_timeout(
        Isolation::Thread,
        HarnessDeadline::CaseWindow { margin: Duration::from_secs(1) },
    );
    let survived = derived.evaluate_one(&candidate, &case);
    assert_eq!(
        survived.status,
        RunStatus::Success,
        "2 s window + 1 s margin must outlive a 1.1 s answer: {:?}",
        survived.status_detail
    );
    assert!(survived.is_scored());
    assert_eq!(survived.day_budget_exceeded_days.len(), 0);
}

#[test]
fn exceeding_the_per_day_answer_window_makes_a_run_unrankable() {
    // Official per-day validity (Q&A Q3): an answer arriving after
    // `daySeconds[i]` is not accepted. MatchSim has no clock and took the
    // plan at face value, so without this check the scores would be
    // optimistic and the run would rank.
    // `broken` is registered only in this binary, so thread isolation again.
    let registry = registry_with_failures();
    let candidate = registry.build("broken:mode=sleep").unwrap();

    let mut case = CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
    // One second of budget, one second and ten milliseconds of work.
    case.setting.day_seconds = vec![1];
    assert_eq!(case.day_budget_us().unwrap(), vec![1_000_000]);

    // Thread isolation: this candidate exists only in this binary. The
    // harness deadline is generous — the run is *late*, not *hung*.
    let evaluator =
        evaluator_with_timeout(Isolation::Thread, HarnessDeadline::Fixed(Duration::from_secs(10)));
    let record = evaluator.evaluate_one(&candidate, &case);

    assert_eq!(record.status, RunStatus::Invalid, "detail: {:?}", record.status_detail);
    assert!(!record.is_scored(), "a late answer must not be rankable");
    assert_eq!(record.day_budget_exceeded_days, vec![0]);
    let detail = record.status_detail.as_deref().unwrap_or_default();
    assert!(detail.contains("daySeconds"), "detail must name the window: {detail}");
    // Measured latency really was over the one-second budget.
    assert!(record.daily_response_us[0] > 1_000_000);
}

#[test]
fn a_run_that_fits_its_daily_window_is_not_flagged() {
    // The everyday case: every bundled fixture gives 60 s a day and real
    // solvers answer in milliseconds.
    let registry = CandidateRegistry::builtin();
    let candidate = registry.build("loop:iterations=2:initial=greedy").unwrap();
    let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();

    let record = evaluator().evaluate_one(&candidate, &case);

    assert!(record.day_budget_exceeded_days.is_empty(), "60s window, not late");
    assert_eq!(record.status, RunStatus::Success);
    assert!(record.is_scored());
}

#[test]
fn aggregation_refuses_to_mix_commits() {
    // Evidence from two commits must not be ranked together: one commit's
    // scores would otherwise be attributed to the other, and the report would
    // name a single SHA it never checked.
    let mut a = record_fixture("loop:iterations=1:initial=greedy");
    a.source_commit_sha = "aaaa111".to_owned();
    let mut b = record_fixture("greedy");
    b.source_commit_sha = "bbbb222".to_owned();

    match AggregateReport::build(vec![a.clone(), b.clone()], None) {
        Err(e) => {
            let msg = e.to_string();
            assert!(msg.contains("aaaa111"), "error must name the commits: {msg}");
            assert!(msg.contains("bbbb222"), "error must name the commits: {msg}");
        }
        Ok(_) => panic!("mixing commits must be refused"),
    }

    // A homogeneous set still aggregates, and reports exactly one commit.
    b.source_commit_sha = "aaaa111".to_owned();
    let ok = agg(vec![a, b], None);
    assert_eq!(ok.single_commit(), Some("aaaa111"));
    assert_eq!(ok.source_commit_shas, vec!["aaaa111".to_owned()]);
}

/// A minimal successful record against the smallest fixture.
fn record_fixture(candidate_id: &str) -> RunRecord {
    RunRecord {
        candidate_id: candidate_id.to_owned(),
        case_id: "fixture:small-8x8".to_owned(),
        seed: None,
        generation_parameters: None,
        source_commit_sha: "aaaa111".to_owned(),
        team: "player-a".to_owned(),
        variety: 1,
        daily_variety_sum: 1,
        total_balls: 1,
        response_time_us: 100,
        elapsed_us: 200,
        timeout: false,
        status: RunStatus::Success,
        scored: true,
        status_detail: None,
        last_acquisition_at: None,
        illegal_action_count: 0,
        fuel_short_count: 0,
        day_budget_exceeded_days: Vec::new(),
        daily_response_us: vec![100],
        daily_varieties: vec![1],
        agent_kinds: None,
        agent_ball_counts: None,
        oracle_refill: false,
    }
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
        "Test-only candidate that fails on purpose. mode=panic|hang|malformed|sleep"
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
        Some("sleep") => Ok("sleep"),
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
            // Deliberately answers later than the case's per-day window so
            // the budget judgement can be observed end to end.
            "sleep" => {
                std::thread::sleep(Duration::from_millis(1_100));
                AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
            }
            // Wrong length and wrong agent order: fails `is_well_shaped`.
            _ => AgentActions::new(vec![engine::model::Action::wait(9)]),
        }
    }
}
