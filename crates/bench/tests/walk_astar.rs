//! Issue #215 — `walk=greedy | astar` on the tour-shaped candidates
//! (`loop`, `multiloop`), pinned at the *candidate* surface the evaluation
//! evidence is made of.
//!
//! The walk layer itself (ordered legs, no oscillation, arrival → next leg,
//! budget / fuel fallbacks) is pinned by the unit tests in
//! `adapters::astar_walk`. These integration tests cover what an evaluator
//! batch depends on:
//!
//! 1. **schema / catalog** — `walk` sits last in both factories' canonical
//!    parameter order, both values build, everything else is rejected, and
//!    a written-out-of-order id canonicalises onto `walk` at the end.
//! 2. **default equivalence** — `walk=greedy` spelled out is *action for
//!    action* identical to the omitted parameter on fixed bundled inputs
//!    (the AC's byte-for-byte pin; every pre-existing test also runs with
//!    the default).
//! 3. **multi-day param matrix** — `walk=astar` re-solves per day with the
//!    `operators=` / `refine=` combinations, scores every day, and never
//!    emits an illegal or unaffordable action.
//!
//! Deviation note: the design draft mentioned a `lookahead=none|crossday`
//! dimension for (d). `loop` / `multiloop` have no `lookahead` parameter on
//! `release-0-2-0` (Issue #193 has not landed), so the matrix below varies
//! the parameters that do exist instead.

use bench::case::{CaseSpec, Corpus};
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation, RunStatus};
use bench::generator;
use bench::{CandidateError, CandidateRegistry};
use domain::PlayerId;
use engine::model::{MatchState, Solver, SolverContext};

/// The baseline candidate and its explicit-default twin (Issue #215 AC).
const LOOP_BASE: &str = "loop:iterations=32:initial=greedy";
const LOOP_GREEDY: &str = "loop:iterations=32:initial=greedy:walk=greedy";
const LOOP_EXT_BASE: &str = "loop:iterations=32:initial=greedy:operators=extended";
const LOOP_EXT_GREEDY: &str = "loop:iterations=32:initial=greedy:operators=extended:walk=greedy";
const MULTILOP_BASE: &str = "multiloop:grid=2x2";
const MULTILOP_GREEDY: &str = "multiloop:grid=2x2:walk=greedy";
const MULTILOP_KMEANS_BASE: &str = "multiloop:kmeans=4:seed=42";
const MULTILOOP_KMEANS_GREEDY: &str = "multiloop:kmeans=4:seed=42:walk=greedy";

fn team() -> PlayerId {
    PlayerId::new("player-a").expect("valid team id")
}

fn evaluator() -> Evaluator {
    Evaluator::new(EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        harness_deadline: HarnessDeadline::default(),
        isolation: Isolation::Thread,
        kinds: None,
    })
}

/// A borrowed [`SolverContext`] over a bundled fixture's initial state.
///
/// The [`MatchState`] is leaked so the context can outlive this helper — a
/// test process exits right after, which is the same trade
/// `adapters::loop_solver`'s own tests make.
fn fixture_context(name: &str) -> SolverContext<'static> {
    let case = CaseSpec::from_bundled_fixture(name).expect("bundled fixture loads");
    let setting = case.to_engine_setting(&team()).expect("case converts");
    let state: &'static MatchState =
        Box::leak(Box::new(MatchState::new(setting).expect("validated state")));
    SolverContext::from_state(state)
}

fn build_solver(id: &str) -> Box<dyn Solver> {
    CandidateRegistry::builtin()
        .build(id)
        .unwrap_or_else(|e| panic!("{id}: {e}"))
        .build_solver()
        .unwrap_or_else(|e| panic!("{id}: {e}"))
}

// ---------------------------------------------------------------------------
// 1. schema / catalog
// ---------------------------------------------------------------------------

/// `walk` is appended last in **both** factories, so every pre-existing
/// canonical id keeps its bytes (Issue #215 design, AC 1).
#[test]
fn walk_is_the_last_canonical_parameter_of_both_factories() {
    let catalog = CandidateRegistry::builtin().catalog();
    for (algorithm, expected) in [
        ("loop", vec!["iterations", "initial", "seed", "operators", "lookahead", "walk", "assign"]),
        ("multiloop", vec!["grid", "kmeans", "seed", "refine", "walk", "assign"]),
    ] {
        let entry = catalog
            .algorithms
            .iter()
            .find(|a| a.algorithm == algorithm)
            .unwrap_or_else(|| panic!("{algorithm} must be in the catalog"));
        let keys: Vec<&str> = entry.params.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, expected, "{algorithm} canonical parameter order");
    }
}

/// Both values build on both algorithms (including inside full canonical
/// ids), an unknown *value* is `InvalidParameter`, an unknown *key* is
/// `UnknownParameter`, and a written-out-of-order id canonicalises with
/// `walk` at the end.
#[test]
fn walk_values_are_validated_and_canonicalised_on_both_factories() {
    let reg = CandidateRegistry::builtin();
    for id in [
        "loop:walk=greedy",
        "loop:walk=astar",
        "loop:iterations=32:initial=greedy:walk=astar",
        "multiloop:grid=2x2:walk=greedy",
        "multiloop:kmeans=4:seed=42:walk=astar",
    ] {
        assert!(reg.build(id).is_ok(), "{id} must build");
    }
    for id in ["loop:walk=step", "multiloop:grid=2x2:walk=nope"] {
        assert!(
            matches!(reg.build(id), Err(CandidateError::InvalidParameter { .. })),
            "{id} must be rejected as an invalid walk value"
        );
    }
    assert!(matches!(
        reg.build("loop:walkmode=astar"),
        Err(CandidateError::UnknownParameter { .. })
    ));

    // Canonicalisation puts `walk` last whatever order it was written in.
    assert_eq!(
        reg.build("loop:walk=astar:iterations=32:initial=greedy").expect("builds").id(),
        "loop:iterations=32:initial=greedy:walk=astar"
    );
    assert_eq!(
        reg.build("multiloop:walk=astar:grid=2x2").expect("builds").id(),
        "multiloop:grid=2x2:walk=astar"
    );
    // …and an already-canonical id round-trips byte for byte.
    assert_eq!(reg.build(LOOP_GREEDY).expect("builds").id(), LOOP_GREEDY);
}

// ---------------------------------------------------------------------------
// 2. default equivalence (AC 1)
// ---------------------------------------------------------------------------

/// `walk=greedy` is byte-for-byte the pre-#215 walk: same actions as the
/// omitted parameter, step for step, on three bundled fixtures — for `loop`
/// (plain and `extended`) and `multiloop` (grid and k-means).
#[test]
fn walk_greedy_is_action_identical_to_the_omitted_parameter() {
    for (base, explicit) in [
        (LOOP_BASE, LOOP_GREEDY),
        (LOOP_EXT_BASE, LOOP_EXT_GREEDY),
        (MULTILOP_BASE, MULTILOP_GREEDY),
        (MULTILOP_KMEANS_BASE, MULTILOOP_KMEANS_GREEDY),
    ] {
        let base_solver = build_solver(base);
        let explicit_solver = build_solver(explicit);
        for fixture in ["small-8x8", "medium-16x16", "road-heavy-v2"] {
            let ctx = fixture_context(fixture);
            let expected = base_solver.decide(&ctx);
            assert!(expected.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
            assert_eq!(
                explicit_solver.decide(&ctx),
                expected,
                "{explicit} must be byte-identical to {base} on {fixture}"
            );
        }
    }
}

/// The same equivalence, but through the evaluator: identical official
/// tuples and identical legality verdicts on a bundled fixture.
#[test]
fn walk_greedy_scores_identically_to_the_omitted_parameter() {
    let reg = CandidateRegistry::builtin();
    let evaluator = evaluator();
    let corpus = Corpus::bundled().expect("bundled fixtures load");
    let case = corpus
        .cases
        .iter()
        .find(|c| c.case_id == "fixture:road-heavy-v2")
        .expect("road-heavy-v2 is bundled");

    for (base, explicit) in [(LOOP_BASE, LOOP_GREEDY), (MULTILOP_BASE, MULTILOP_GREEDY)] {
        let records = [base, explicit].map(|id| {
            let candidate = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            evaluator.evaluate_one(&candidate, case)
        });
        let (base_run, explicit_run) = (&records[0], &records[1]);
        assert_eq!(base_run.status, RunStatus::Success, "{base} must score");
        assert_eq!(explicit_run.status, base_run.status, "{explicit} status");
        assert_eq!(
            (explicit_run.variety, explicit_run.daily_variety_sum, explicit_run.total_balls),
            (base_run.variety, base_run.daily_variety_sum, base_run.total_balls),
            "{explicit} must reproduce {base}'s official tuple"
        );
        assert_eq!(explicit_run.illegal_action_count, base_run.illegal_action_count);
        assert_eq!(explicit_run.fuel_short_count, base_run.fuel_short_count);
    }
}

// ---------------------------------------------------------------------------
// 3. multi-day param matrix smoke (AC 2 / (d))
// ---------------------------------------------------------------------------

/// `walk=astar` runs the whole day contract through `decide` once per day
/// across a multi-day generated corpus and the parameter matrix that exists
/// on this branch — every day solved and judged, `fuel_short` / `illegal`
/// zero, no per-day budget overrun.
///
/// Road-heavy's generated cases default to three days, so a single-day
/// implementation would show up here as a truncated `daily_varieties`.
#[test]
fn walk_astar_re_solves_every_day_across_the_param_matrix() {
    let reg = CandidateRegistry::builtin();
    let evaluator = evaluator();
    let params = generator::parse_selection("road-heavy:seed=1,few-series:seed=1")
        .expect("selection parses");
    let corpus = generator::build_corpus(&params).expect("generated cases build");
    assert_eq!(corpus.cases.len(), 2, "both selected generated cases must build");
    let longest = corpus
        .cases
        .iter()
        .map(|c| c.to_engine_setting(&team()).expect("case converts").num_days)
        .max()
        .unwrap_or(0);
    assert!(longest >= 2, "at least one case must exercise multi-day re-solving, got {longest}");

    for id in [
        LOOP_GREEDY, // control: the default walk through the same path
        "loop:iterations=32:initial=greedy:walk=astar",
        "loop:iterations=32:initial=greedy:operators=extended:walk=astar",
        "multiloop:kmeans=4:seed=42:walk=astar",
        "multiloop:grid=2x2:refine=false:walk=astar",
    ] {
        let candidate = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        for case in &corpus.cases {
            let expected_days =
                case.to_engine_setting(&team()).expect("case converts").num_days as usize;
            assert!(expected_days >= 1, "the case must have at least one day");

            let record = evaluator.evaluate_one(&candidate, case);
            assert_eq!(
                record.status,
                RunStatus::Success,
                "{id} on {}: {:?}",
                case.case_id,
                record.status_detail
            );
            assert!(record.is_scored(), "{id} on {} must be scored", case.case_id);
            assert_eq!(
                record.fuel_short_count, 0,
                "{id} on {} must never emit an unaffordable move",
                case.case_id
            );
            assert_eq!(
                record.illegal_action_count, 0,
                "{id} on {} must never emit an illegal action",
                case.case_id
            );
            assert!(
                record.day_budget_exceeded_days.is_empty(),
                "{id} on {} must answer within every day's window",
                case.case_id
            );
            assert_eq!(
                record.daily_varieties.len(),
                expected_days,
                "{id} on {} must solve and judge every one of its days",
                case.case_id
            );
        }
    }
}

/// The Issue #213 baseline-zero case (`generated:v1:few-series:seed=1` at the
/// time; the same archetype + seed builds as `generated:v2:…` since #226 —
/// same terrain, re-coded cells) has
/// to stay runnable under `walk=astar` — success, scored, zero illegal and
/// zero fuel-short. Whether the score actually leaves zero is evaluation
/// evidence (Issue #215 comment), not something this gate asserts.
#[test]
fn walk_astar_runs_the_213_baseline_zero_case_cleanly() {
    let reg = CandidateRegistry::builtin();
    let evaluator = evaluator();
    let params = generator::parse_selection("few-series:seed=1").expect("selection parses");
    let corpus = generator::build_corpus(&params).expect("generated case builds");
    let case = corpus.cases.first().expect("one generated case");

    for id in [
        LOOP_BASE,
        LOOP_GREEDY,
        "loop:iterations=32:initial=greedy:walk=astar",
        "multiloop:kmeans=4:seed=42:walk=astar",
    ] {
        let candidate = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        let record = evaluator.evaluate_one(&candidate, case);
        assert_eq!(
            record.status,
            RunStatus::Success,
            "{id} on {} must complete: {:?}",
            case.case_id,
            record.status_detail
        );
        assert!(record.is_scored(), "{id} on {} must be scored", case.case_id);
        assert_eq!(record.fuel_short_count, 0, "{id}: no fuel shortage");
        assert_eq!(record.illegal_action_count, 0, "{id}: no illegal actions");
    }
}
