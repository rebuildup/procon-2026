//! Issue #204 — the fuel-stress axis, proven where candidates meet cases.
//!
//! `generator::FuelMode::Tight` pins `fuelLimits` to the Q&A Q14 floor
//! (`daySteps[0]`, `docs/contest/api.md:199`) instead of the v1 completability
//! floor `>= 2 x sum(daySteps)`. The generator's own unit tests prove that
//! number is inside the official envelope; only real candidates can prove the
//! two properties the axis exists for:
//!
//! 1. **Completability** — for every archetype (multiple seeds, default size
//!    / days), the landed fuel-aware walker (`walk=astar`, Issue #215) which
//!    plans legs against remaining fuel and re-checks affordability before
//!    emitting, completes each tight case with `fuel_short_count = 0`,
//!    `illegal_action_count = 0` and `scored = true`. A tight case that not
//!    even the fuel-aware walker could finish would be a *generator* bug —
//!    caught here rather than shipped.
//! 2. **Axis observability** — on at least some of those same tight cases the
//!    fuel-blind walk (`loop:iterations=32:initial=greedy`, default walk)
//!    incurs `fuel_short_count > 0` (recorded as an *explained* candidate
//!    `invalid`, Issue #185's failure semantics) while `walk=astar` still
//!    completes the very same case cleanly. Without that gap the mode would
//!    be a label, not an axis. The comfortable twins of the same
//!    (archetype, seed) pairs stay `fuel_short_count = 0` for the same
//!    fuel-blind candidate — the v1 invariant did not move.

use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation, RunStatus};
use bench::generator::{self, Archetype, FuelMode, GenerationParams};
use bench::{CandidateRegistry, MatchFixtureLoader};
use domain::PlayerId;

/// The standard screening seed set (same two seeds the Stage B batches use).
const SEEDS: [u64; 2] = [1, 2];

/// The fuel-blind control: the default walk never looks at the tank.
const FUEL_BLIND: &str = "loop:iterations=32:initial=greedy";

/// The landed fuel-aware walkers (Issue #215) — the completability proof.
const FUEL_AWARE: [&str; 2] =
    ["loop:iterations=32:initial=greedy:walk=astar", "multiloop:kmeans=4:seed=42:walk=astar"];

fn team() -> PlayerId {
    PlayerId::new("player-a").expect("valid team id")
}

fn evaluator() -> Evaluator {
    Evaluator::new(EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        harness_deadline: HarnessDeadline::default(),
        isolation: Isolation::Thread,
    })
}

/// Every `(archetype, seed)` pair of the tight axis, default size / days.
fn tight_pairs() -> Vec<(Archetype, u64)> {
    Archetype::ALL.iter().flat_map(|a| SEEDS.iter().map(move |s| (*a, *s))).collect()
}

fn generated_case(archetype: Archetype, seed: u64, fuel: FuelMode) -> bench::CaseSpec {
    let mut params = GenerationParams::new(archetype, seed);
    params.fuel = fuel;
    generator::generate(&params)
        .unwrap_or_else(|e| panic!("{archetype} seed={seed} fuel={fuel} must generate: {e}"))
}

/// A run that completed the axis contract: judged, legal, tank intact.
fn assert_clean(record: &bench::evaluate::RunRecord, id: &str, expected_fuel_short: u32) {
    assert_eq!(
        record.status,
        RunStatus::Success,
        "{id} on {} must complete: {:?}",
        record.case_id,
        record.status_detail
    );
    assert!(record.is_scored(), "{id} on {} must be scored", record.case_id);
    assert_eq!(
        record.fuel_short_count, expected_fuel_short,
        "{id} on {}: fuel_short_count",
        record.case_id
    );
    assert_eq!(
        record.illegal_action_count, 0,
        "{id} on {}: illegal_action_count: {:?}",
        record.case_id, record.status_detail
    );
    assert!(
        record.day_budget_exceeded_days.is_empty(),
        "{id} on {} blew its per-day window",
        record.case_id
    );
}

// ---------------------------------------------------------------------------
// 1. Completability — every tight case is finishable by the fuel-aware walk
// ---------------------------------------------------------------------------

/// Issue #204 AC: `fuel=tight` never produces an unsolvable case. Both landed
/// fuel-aware walkers must finish every archetype × seed with zero fuel-short
/// and zero illegal actions, i.e. `status: success` + `scored`.
#[test]
fn tight_cases_are_completable_by_the_fuel_aware_walkers() {
    let registry = CandidateRegistry::builtin();
    let evaluator = evaluator();
    let mut checked = 0_usize;

    for (archetype, seed) in tight_pairs() {
        let case = generated_case(archetype, seed, FuelMode::Tight);
        assert_eq!(
            case.setting.fuel_limits, case.setting.day_steps[0],
            "{}: tight must pin fuelLimits to daySteps[0]",
            case.case_id
        );
        for id in FUEL_AWARE {
            let candidate = registry.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let record = evaluator.evaluate_one(&candidate, &case);
            assert_clean(&record, id, 0);
            checked += 1;
        }
    }
    assert_eq!(checked, Archetype::ALL.len() * SEEDS.len() * FUEL_AWARE.len());
}

// ---------------------------------------------------------------------------
// 2. Axis observability — fuel-blind fails where fuel-aware still finishes
// ---------------------------------------------------------------------------

/// Issue #204's whole point: on the tight axis the default (fuel-blind) walk
/// runs the tank dry on *some* tight cases — an explained candidate `invalid`,
/// never a silent one — while `walk=astar` completes those same cases.
///
/// If this assertion ever fails because *no* tight case discriminates any
/// more, the tight range has drifted loose and the axis is unobservable
/// again: tighten the range (Issue #204) rather than weaken this test.
#[test]
fn the_tight_axis_discriminates_fuel_blind_from_fuel_aware() {
    let registry = CandidateRegistry::builtin();
    let evaluator = evaluator();
    let blind = registry.build(FUEL_BLIND).unwrap_or_else(|e| panic!("{FUEL_BLIND}: {e}"));
    let aware = registry.build(FUEL_AWARE[0]).unwrap_or_else(|e| panic!("{}: {e}", FUEL_AWARE[0]));

    let mut discriminating = Vec::new();
    for (archetype, seed) in tight_pairs() {
        let case = generated_case(archetype, seed, FuelMode::Tight);
        let blind_record = evaluator.evaluate_one(&blind, &case);
        let aware_record = evaluator.evaluate_one(&aware, &case);

        // The fuel-aware half of the claim, on the same case.
        assert_clean(&aware_record, FUEL_AWARE[0], 0);

        if blind_record.fuel_short_count > 0 {
            // An *explained* candidate failure: judged, numbers recorded,
            // explicitly invalid because of the tank — Issue #185's
            // candidate-vs-case failure split holds on the tight axis too.
            assert_eq!(
                blind_record.status,
                RunStatus::Invalid,
                "{} on {}: a fuel-short run must be Invalid, not silently scored",
                FUEL_BLIND,
                case.case_id
            );
            assert!(!blind_record.is_scored(), "{} must not rank a fuel-short run", case.case_id);
            assert!(
                blind_record
                    .status_detail
                    .as_deref()
                    .is_some_and(|d| d.contains("fuel exhaustion")),
                "{} on {}: the invalid status must name fuel exhaustion, got {:?}",
                FUEL_BLIND,
                case.case_id,
                blind_record.status_detail
            );
            discriminating
                .push(format!("{}: fuel_short={}", case.case_id, blind_record.fuel_short_count));
        }
    }

    // At least one tight case must bind the fuel-blind walk; report the set so
    // a drift to zero is visible in the failure message.
    assert!(
        !discriminating.is_empty(),
        "no tight case discriminated: the fuel-blind walk never ran the tank dry, so \
         `fuel=tight` no longer makes fuel observable — tighten the range (Issue #204)"
    );
    println!("discriminating tight cases ({}): {discriminating:#?}", discriminating.len());
}

// ---------------------------------------------------------------------------
// 3. Control — the comfortable axis did not move (v1 regression)
// ---------------------------------------------------------------------------

/// The default `fuel=comfortable` corpus must keep its exact v1 guarantee:
/// the same fuel-blind candidate that goes dry on tight cases can never go
/// dry on comfortable ones. This is the "existing seed / `case_id` semantics
/// did not change" AC, checked behaviourally on top of the id assertions in
/// `generator::tests`.
#[test]
fn comfortable_cases_keep_the_v1_no_fuel_short_invariant() {
    let registry = CandidateRegistry::builtin();
    let evaluator = evaluator();
    let blind = registry.build(FUEL_BLIND).unwrap_or_else(|e| panic!("{FUEL_BLIND}: {e}"));

    for (archetype, seed) in tight_pairs() {
        let case = generated_case(archetype, seed, FuelMode::Comfortable);
        assert!(
            !case.case_id.contains(":fuel="),
            "the default mode must not add a fuel segment: {}",
            case.case_id
        );
        let record = evaluator.evaluate_one(&blind, &case);
        assert_clean(&record, FUEL_BLIND, 0);
    }
}

/// Both modes are present in the bundled evaluation surface at once, with
/// distinct ids and one shared map: a mixed corpus builds (the seed-set
/// evidence can quote either mode in one batch).
#[test]
fn comfortable_and_tight_twins_coexist_in_one_corpus() {
    assert!(MatchFixtureLoader::bundled_fixtures().len() >= 9, "the fixture corpus is intact");

    let selection =
        generator::parse_selection("road-heavy:seed=1:fuel=tight,open:seed=1:fuel=tight")
            .expect("tight selection parses");
    let tight = generator::build_corpus(&selection).expect("tight cases build");
    let default_selection =
        generator::parse_selection("road-heavy:seed=1,open:seed=1").expect("selection parses");
    let default = generator::build_corpus(&default_selection).expect("comfortable cases build");

    let mut all = default.cases;
    all.extend(tight.cases);
    let merged = bench::case::Corpus::new(all).expect("twin ids never collide");
    assert_eq!(merged.len(), 4);
    assert!(merged.case_ids().contains(&"generated:v2:road-heavy:seed=1"));
    assert!(merged.case_ids().contains(&"generated:v2:road-heavy:seed=1:fuel=tight"));
}
