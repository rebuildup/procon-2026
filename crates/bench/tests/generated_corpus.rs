//! End-to-end proof of the generated corpus (Issue #185).
//!
//! The unit tests in `crates/bench/src/generator.rs` prove that generation is
//! deterministic and validator-clean. These integration tests prove the things
//! that only show up once real candidates meet generated cases:
//!
//! 1. **E2E batch** — every bundled fixture *plus* generated cases evaluate
//!    in one batch, all scored, with `seed` / `generation_parameters` /
//!    generator version surviving into the durable records.
//! 2. **Shared case axis** — sibling candidates in that batch see exactly the
//!    same cases with exactly the same seeds and parameters.
//! 3. **Failure classes are distinct** — a case-generation failure is an `Err`
//!    at corpus build (naming archetype + seed + cause, zero records); a
//!    candidate failure is a `RunRecord` with a failure status. Neither can
//!    be produced by the other's path.
//! 4. **Baseline completability** — `wait` and a real `loop` baseline finish
//!    every archetype with zero fuel-short and zero illegal actions, i.e. a
//!    generated case is never invalid-by-construction.
//! 5. **Multi-day cases really run every day**, so tier 2
//!    (`daily_variety_sum`) and the per-day machinery are observable.

use bench::candidate::{CandidateFactory, CandidateSpec, ParamSpec};
use bench::case::{CaseSpec, Corpus};
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation, RunStatus};
use bench::generator::{self, Archetype, GENERATOR_VERSION, GenerationParams};
use bench::tournament::{TournamentConfig, TournamentRunner};
use bench::{CandidateRegistry, MatchFixtureLoader};
use domain::PlayerId;
use engine::model::{AgentActions, Solver, SolverContext};

fn team() -> PlayerId {
    PlayerId::new("player-a").expect("valid team id")
}

fn config() -> EvalConfig {
    EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        harness_deadline: HarnessDeadline::default(),
        isolation: Isolation::Thread,
        kinds: None,
        oracle_refill: false,
        jobs: 1,
    }
}

fn evaluator() -> Evaluator {
    Evaluator::new(config())
}

fn bundled_count() -> usize {
    MatchFixtureLoader::bundled_fixtures().len()
}

/// The mixed corpus the E2E batch runs: every bundled fixture first, then the
/// two explicitly seeded generated cases (the same shape the CLI builds for
/// `--cases all --generated 'open:seed=1,road-heavy:seed=1'`).
fn mixed_corpus() -> Corpus {
    let mut cases = Corpus::bundled().expect("bundled fixtures load").cases;
    let params =
        generator::parse_selection("open:seed=1,road-heavy:seed=1").expect("selection parses");
    let generated = generator::build_corpus(&params).expect("generated cases build");
    cases.extend(generated.cases);
    Corpus::new(cases).expect("fixture and generated ids never collide")
}

fn generated_case(archetype: Archetype, seed: u64) -> CaseSpec {
    generator::generate(&GenerationParams::new(archetype, seed))
        .unwrap_or_else(|e| panic!("{archetype} seed={seed} must generate: {e}"))
}

// ---------------------------------------------------------------------------
// 1 + 2. E2E: bundled fixtures + generated, one batch, shared case axis
// ---------------------------------------------------------------------------

#[test]
fn bundled_fixtures_and_generated_cases_evaluate_in_one_batch() {
    let registry = CandidateRegistry::builtin();
    let ids: Vec<String> =
        ["loop:iterations=4:initial=greedy", "wait"].iter().map(|s| (*s).to_owned()).collect();
    let corpus = mixed_corpus();
    assert_eq!(corpus.len(), bundled_count() + 2, "bundled fixtures + 2 generated cases");
    assert_eq!(corpus.case_ids().iter().filter(|id| id.starts_with("generated:")).count(), 2);

    let config = TournamentConfig::new(ids.clone(), Some("wait".to_owned()), "player-a", None)
        .with_source_commit_sha("test-commit")
        .with_output_dir(std::env::temp_dir().join("procon-bench-e2e-generated-corpus"));
    let outcome = TournamentRunner::new(config)
        .run(&registry, &corpus)
        .unwrap_or_else(|e| panic!("batch must run: {e}"));

    // The full cross product: every candidate met every case.
    assert_eq!(outcome.records.len(), corpus.len() * ids.len());
    for case_id in corpus.case_ids() {
        for candidate in &ids {
            let hits = outcome
                .records
                .iter()
                .filter(|r| r.case_id == case_id && r.candidate_id == *candidate)
                .count();
            assert_eq!(hits, 1, "{candidate} on {case_id} must be evaluated exactly once");
        }
    }

    // A generated case that cannot be scored would mean generation produced
    // an invalid-by-construction case — the whole point of the fail-fast gate.
    for record in &outcome.records {
        assert!(
            record.is_scored(),
            "{} on {} must be scored, got {}: {:?}",
            record.candidate_id,
            record.case_id,
            record.status,
            record.status_detail
        );
    }

    // Provenance split: generated records carry the full generator identity,
    // fixture records carry none.
    for record in &outcome.records {
        if record.case_id.starts_with("generated:") {
            let seed = record.seed.expect("generated records carry a seed");
            let params = record.generation_parameters.as_ref().expect("carry parameters");
            assert_eq!(params["generator_version"], GENERATOR_VERSION);
            assert!(record.case_id.contains(&format!("seed={seed}")), "{}", record.case_id);
            assert!(record.case_id.starts_with(&format!("generated:v{GENERATOR_VERSION}:")));
        } else {
            assert_eq!(record.seed, None, "{} is a fixture", record.case_id);
            assert!(record.generation_parameters.is_none(), "{} is a fixture", record.case_id);
        }
    }

    // Siblings share one case axis: for a given case, every candidate's
    // record carries the identical seed and generation parameters.
    for case_id in corpus.case_ids() {
        let mut axes = outcome
            .records
            .iter()
            .filter(|r| r.case_id == case_id)
            .map(|r| (r.seed, r.generation_parameters.clone()));
        let first = axes.next().expect("at least one record per case");
        for axis in axes {
            assert_eq!(axis, first, "all candidates must share one case axis on {case_id}");
        }
    }

    // Durable: the records round-trip with their provenance intact.
    let paths = outcome.paths.clone().expect("output dir configured");
    let reread = bench::report::read_runs(&paths.runs).expect("runs.jsonl parses");
    assert_eq!(reread, outcome.records);
    assert!(
        reread.iter().any(|r| r.seed.is_some() && r.case_id.starts_with("generated:")),
        "a generated record must survive the JSONL round trip"
    );
}

// ---------------------------------------------------------------------------
// 3. The two failure classes cannot be confused
// ---------------------------------------------------------------------------

/// Test-only factory: a candidate that panics on demand, so this binary can
/// produce a *candidate* failure without depending on another test binary's
/// registry.
struct PanicFactory;

impl CandidateFactory for PanicFactory {
    fn algorithm(&self) -> &'static str {
        "panicky"
    }
    fn params(&self) -> &'static [ParamSpec] {
        &[]
    }
    fn describe(&self) -> &'static str {
        "Test-only candidate that panics, to represent a candidate failure."
    }
    fn build(&self, _spec: &CandidateSpec) -> Result<Box<dyn Solver>, bench::CandidateError> {
        Ok(Box::new(PanicSolver))
    }
}

struct PanicSolver;

impl Solver for PanicSolver {
    fn name(&self) -> &'static str {
        "panicky"
    }
    fn decide(&self, _ctx: &SolverContext<'_>) -> AgentActions {
        panic!("deliberate candidate panic");
    }
}

/// `RunStatus` has exactly these four variants, and every one of them
/// describes what a *candidate* did during a run. An exhaustive match is the
/// compile-level statement that no status can carry "the corpus could not be
/// built".
fn candidate_side_meaning(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Success => "candidate completed cleanly",
        RunStatus::Invalid => "candidate broke the contract / was judged late",
        RunStatus::Timeout => "candidate overran the harness deadline",
        RunStatus::Panic => "candidate panicked",
    }
}

#[test]
fn a_generation_failure_and_a_candidate_failure_are_distinct_classes() {
    // --- Class A: case generation failure ---------------------------------
    // Produced by `build_corpus`, *before* any candidate exists. It is an
    // `Err`, it names archetype + seed + cause, and because no `CaseSpec` was
    // built there is structurally nothing for the evaluator to turn into a
    // record.
    let mut poisoned = GenerationParams::new(Archetype::RoadHeavy, 7);
    poisoned.days = 0;
    let build = generator::build_corpus(&[poisoned]);
    let err = build.expect_err("poisoned parameters must fail the build");
    let msg = err.to_string();
    assert!(msg.starts_with("case generation failure"), "{msg}");
    assert!(msg.contains("archetype=road-heavy"), "{msg}");
    assert!(msg.contains("seed=7"), "{msg}");
    assert!(msg.contains("days"), "{msg}");

    // --- Class B: candidate failure ---------------------------------------
    // Produced by the evaluator *after* the corpus exists, as a record.
    let mut registry = CandidateRegistry::builtin();
    registry.register(std::sync::Arc::new(PanicFactory));
    let case = generated_case(Archetype::Open, 1);
    let candidate = registry.build("panicky").expect("test candidate builds");
    let record = evaluator().evaluate_one(&candidate, &case);
    assert_eq!(record.status, RunStatus::Panic);
    assert!(!record.is_scored());
    assert_eq!(candidate_side_meaning(record.status), "candidate panicked");

    // --- And the two cannot substitute for each other ----------------------
    // (a) A generation failure never reaches the evaluator: the build above
    //     returned `Err`, so the corpus (and therefore every record) does not
    //     exist. The healthy path proves the channel works when it should.
    let healthy = registry.build("wait").expect("wait is builtin");
    let ok = evaluator().evaluate_one(&healthy, &case);
    assert_eq!(ok.status, RunStatus::Success, "a valid generated case scores");
    assert!(ok.is_scored());
    // (b) An `invalid` record is a *judged run*, not a rejected case: it
    //     carries numbers and a candidate id, whereas GenerationError carries
    //     archetype + seed and no record at all.
    assert_eq!(ok.case_id, "generated:v2:open:seed=1");
    assert_ne!(msg, ok.status_detail.clone().unwrap_or_default());
}

// ---------------------------------------------------------------------------
// 4. Baseline completability — no generated case is invalid-by-construction
// ---------------------------------------------------------------------------

#[test]
fn baselines_complete_every_archetype_without_fuel_short_or_illegal_actions() {
    let registry = CandidateRegistry::builtin();
    let baselines = ["wait", "loop:iterations=32:initial=greedy"];

    for archetype in Archetype::ALL {
        for seed in [1_u64, 7] {
            let case = generated_case(archetype, seed);
            for id in baselines {
                let candidate = registry.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
                let record = evaluator().evaluate_one(&candidate, &case);
                assert_eq!(
                    record.status,
                    RunStatus::Success,
                    "{} on {} (seed={seed}) must complete: {:?}",
                    id,
                    case.case_id,
                    record.status_detail
                );
                assert_eq!(
                    record.fuel_short_count, 0,
                    "{} on {} ran the tank dry — the fuel invariant must make that impossible",
                    id, case.case_id
                );
                assert_eq!(
                    record.illegal_action_count, 0,
                    "{} on {} emitted illegal actions: {:?}",
                    id, case.case_id, record.status_detail
                );
                assert!(
                    record.day_budget_exceeded_days.is_empty(),
                    "{} on {} blew its per-day window",
                    id,
                    case.case_id
                );
                assert!(record.is_scored());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Multi-day cases exercise every day
// ---------------------------------------------------------------------------

#[test]
fn multi_day_generated_cases_run_every_day_and_split_tier_2() {
    let registry = CandidateRegistry::builtin();
    let candidate = registry.build("loop:iterations=4:initial=greedy").expect("loop builds");

    for archetype in [
        Archetype::RoadHeavy,
        Archetype::ManySeries,
        Archetype::PondObstacle,
        Archetype::DenseSpots,
    ] {
        let case = generated_case(archetype, 3);
        let days = case.setting.day_steps.len();
        assert!(days >= 2, "{archetype} must default to a multi-day case");

        let record = evaluator().evaluate_one(&candidate, &case);
        assert_eq!(record.status, RunStatus::Success, "{archetype}: {:?}", record.status_detail);
        assert_eq!(record.daily_varieties.len(), days, "{archetype}: one entry per day");
        assert_eq!(record.daily_response_us.len(), days, "{archetype}: one timing per day");
        assert_eq!(
            record.daily_varieties.iter().sum::<u32>(),
            record.daily_variety_sum,
            "{archetype}: tier 2 must be the sum of the per-day breakdown"
        );
        // Day 0 is the front-loaded long day; later days sit at the W+H floor.
        assert!(
            case.setting.day_steps[0] > case.setting.day_steps[1],
            "{archetype}: multi-day generated cases are front-loaded"
        );
    }
}
