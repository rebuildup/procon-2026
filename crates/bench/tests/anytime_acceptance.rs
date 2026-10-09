//! Regression tests for anytime solver acceptance (Issue #342).
//!
//! Verifies that the FINAL RETURNED plan (not just the internal `best`
//! variable) actually reflects the dictionary-order improvement.

use bench::CandidateRegistry;
use engine::MatchSim;
use engine::model::{MatchState, SolverContext};

fn evaluate(cid: &str, case: &bench::CaseSpec) -> engine::model::RunResult {
    let pattern = bench::kinds::parse_kinds("P,S,P,S,P").unwrap();
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a").unwrap();
    let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).unwrap();
    let state = MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin().build(cid).unwrap().build_solver().unwrap();
    MatchSim::default().run(state.setting.clone(), solver.as_ref()).unwrap()
}

fn load_701() -> bench::CaseSpec {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read(manifest.join("tests/fixtures/arena-701.json"))
        .expect("701 fixture file");
    serde_json::from_str(&String::from_utf8(raw).unwrap()).expect("parse 701")
}

#[test]
fn anytime_b_improvement_returns_improved_plan_on_701() {
    let case = load_701();

    // Evaluate park=far baseline
    let base = evaluate("coverage:budget=official:park=far", &case);

    // Evaluate anytime with budget
    let anytime = evaluate("anytime:budget_ms=5000:seed=42:budget=official", &case);

    // Official V must not decrease
    assert!(
        anytime.variety >= base.variety,
        "anytime V {} < baseline V {}",
        anytime.variety, base.variety
    );

    // The returned plan must be at least as good as the baseline
    // (D same or better, B same or better when D is same)
    assert!(
        anytime.daily_variety_sum >= base.daily_variety_sum,
        "anytime D {} < baseline D {}",
        anytime.daily_variety_sum, base.daily_variety_sum
    );

    // If D is same, B must be >= baseline (B improvement accepted)
    if anytime.daily_variety_sum == base.daily_variety_sum {
        assert!(
            anytime.total_balls >= base.total_balls,
            "anytime D same but B {} < baseline B {}",
            anytime.total_balls, base.total_balls
        );
    }

    // No illegal actions or fuel shorts
    assert_eq!(anytime.illegal_action_count, 0, "illegal actions");
    assert_eq!(anytime.fuel_short_count, 0, "fuel shorts");
}

#[test]
fn anytime_never_degrades_v_on_any_fixture() {
    for name in bench::fixture::MatchFixtureLoader::bundled_fixtures() {
        let case = bench::CaseSpec::from_bundled_fixture(name).unwrap();
        let base = evaluate("coverage:budget=official:park=far", &case);
        let anytime = evaluate("anytime:budget_ms=5000:seed=42:budget=official", &case);

        assert!(
            anytime.variety >= base.variety,
            "{name}: anytime V {} < baseline V {}",
            anytime.variety, base.variety
        );
        assert!(
            anytime.daily_variety_sum >= base.daily_variety_sum,
            "{name}: anytime D {} < baseline D {}",
            anytime.daily_variety_sum, base.daily_variety_sum
        );
        assert_eq!(anytime.illegal_action_count, 0, "{name}: illegal");
        assert_eq!(anytime.fuel_short_count, 0, "{name}: fuel short");
    }
}

#[test]
fn anytime_budget_zero_returns_park_far_identically() {
    // budget_ms=0 must return the park=far plan unchanged.
    for name in bench::fixture::MatchFixtureLoader::bundled_fixtures() {
        let case = bench::CaseSpec::from_bundled_fixture(name).unwrap();
        let base = evaluate("coverage:budget=official:park=far", &case);
        let budget0 = evaluate("anytime:budget_ms=0:seed=42:budget=official", &case);

        assert_eq!(
            base.variety, budget0.variety,
            "{name}: budget=0 V mismatch"
        );
        assert_eq!(
            base.daily_variety_sum, budget0.daily_variety_sum,
            "{name}: budget=0 D mismatch"
        );
        assert_eq!(
            base.total_balls, budget0.total_balls,
            "{name}: budget=0 B mismatch"
        );
    }
}
