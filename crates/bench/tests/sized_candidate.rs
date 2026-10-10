//! `sized` (Issue #428) only chooses who plans the day: its answer must be
//! the chosen candidate's answer, byte for byte.
use bench::CandidateRegistry;
use bench::case::CaseSpec;
use bench::kinds::{parse_kinds, resolve_kinds};
use domain::PlayerId;
use engine::model::{MatchState, Solver, SolverContext};

fn solver(id: &str) -> Box<dyn Solver> {
    CandidateRegistry::builtin().build(id).and_then(|c| c.build_solver()).expect("id builds")
}

fn state(case: &CaseSpec) -> MatchState {
    let kinds = resolve_kinds(Some(&parse_kinds("S,P").unwrap()), case.setting.agents.len());
    let kinds: Vec<_> = (0..kinds.len()).map(|i| kinds[usize::from(i != 0)]).collect();
    let setting = case
        .to_engine_setting_with_kinds(&PlayerId::new("player-a").unwrap(), Some(&kinds))
        .unwrap();
    MatchState::new(setting).unwrap()
}

/// A board wider than 16 goes to `fleet` with the day-end fuel term.
#[test]
fn sized_hands_a_large_board_to_fleet_with_the_fuel_term() {
    let case: CaseSpec = serde_json::from_str(include_str!("fixtures/arena-701.json")).unwrap();
    assert!(case.setting.map.width > 16);
    let state = state(&case);
    let ctx = SolverContext::from_state(&state);
    let fleet = "fleet:budget=official:seed=7:iters=60000:chains=4:fuel_need=50:fuel_mix=on";
    assert_eq!(solver("sized:budget=official:seed=7").decide(&ctx), solver(fleet).decide(&ctx));
    let mid =
        "fleet:budget=official:seed=7:iters=60000:chains=4:fuel_need=50:fuel_mix=on:midleg=on";
    assert_eq!(
        solver("sized:budget=official:seed=7:midleg=on").decide(&ctx),
        solver(mid).decide(&ctx)
    );
    let plain = "fleet:budget=official:seed=7:iters=60000:chains=4";
    assert_eq!(
        solver("sized:budget=official:seed=7:fuel_need=0").decide(&ctx),
        solver(plain).decide(&ctx)
    );
}

/// A board up to 16x16 goes to `convoy`, without the fuel term.
#[test]
fn sized_hands_a_small_board_to_convoy() {
    // The round-one board (16x16).
    let case: CaseSpec =
        serde_json::from_str(include_str!("../../../data/round1/derived/round1-match3.case.json"))
            .unwrap();
    assert!(case.setting.map.width <= 16 && case.setting.map.height <= 16);
    let state = state(&case);
    let ctx = SolverContext::from_state(&state);
    assert_eq!(
        solver("sized:budget=official:seed=7").decide(&ctx),
        solver("convoy:budget=official:seed=7").decide(&ctx)
    );
}

#[test]
fn sized_requires_the_official_budget_and_known_params() {
    let registry = CandidateRegistry::builtin();
    assert!(registry.build("sized").and_then(|c| c.build_solver()).is_err());
    assert!(
        registry.build("sized:budget=official:width=3").and_then(|c| c.build_solver()).is_err()
    );
}
