use bench::CandidateRegistry;
use domain::{MatchId, PlayerId};
use engine::MatchSim;
use engine::model::{
    Agent, Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState, SolverContext, Spot, Terrain,
};

fn setting(fuel: u32) -> MatchSetting {
    let cells = (0..8)
        .flat_map(|row| {
            (0..8).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = (row == 1 && (1..=4).contains(&col)).then_some(Spot {
                    series: if col < 3 { 1 } else { u32::try_from(col - 1).unwrap() },
                    pos,
                    max_stock: 3,
                    current_stock: 3,
                });
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new("coverage-test").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(8, 8).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 1), fuel),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(0, 2)),
        ],
        day_steps: 16,
        num_days: 2,
        day_steps_by_day: Vec::new(),
    }
}

#[test]
fn repeat_tour_replans_daily_and_harvests_after_covering_series() {
    let solver = CandidateRegistry::builtin()
        .build("repeat-tour:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let result = MatchSim::default().run(setting(32), solver.as_ref()).unwrap();
    assert_eq!((result.variety, result.daily_variety_sum, result.total_balls), (3, 6, 8));
    assert_eq!(result.illegal_action_count, 0);
}

#[test]
fn repeat_tour_uses_supply_contact_to_continue_beyond_initial_fuel() {
    let solver = CandidateRegistry::builtin()
        .build("repeat-tour:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let result = MatchSim::default().run(setting(2), solver.as_ref()).unwrap();
    assert_eq!((result.variety, result.daily_variety_sum, result.total_balls), (3, 6, 8));
    assert_eq!(result.fuel_short_count, 0);
}

#[test]
fn repeat_tour_does_not_depend_on_offline_only_series_history() {
    let solver = CandidateRegistry::builtin()
        .build("repeat-tour:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let mut state = MatchState::new(setting(32)).unwrap();
    let original = solver.decide(&SolverContext::from_state(&state));
    state.series_taken.extend([1, 2, 3]);
    assert_eq!(solver.decide(&SolverContext::from_state(&state)), original);
}

#[test]
fn repeat_tour_official_normalization_preserves_every_action() {
    use bench::adapters::walk::BudgetMode;
    use bench::case::CaseSpec;
    use bench::kinds::{parse_kinds, resolve_kinds};
    use engine::model::AgentActions;
    let case: CaseSpec = serde_json::from_str(include_str!("fixtures/arena-701.json")).unwrap();
    for pattern in ["P,S", "P,S,S,P,S"] {
        let kinds = resolve_kinds(Some(&parse_kinds(pattern).unwrap()), case.setting.agents.len());
        let setting = case
            .to_engine_setting_with_kinds(&PlayerId::new("player-a").unwrap(), Some(&kinds))
            .unwrap();
        let mut state = MatchState::new(setting).unwrap();
        let solver = CandidateRegistry::builtin()
            .build("repeat-tour:budget=official")
            .unwrap()
            .build_solver()
            .unwrap();
        let sim = MatchSim::new(case.sim_config().unwrap());
        while !state.is_finished() {
            let ctx = SolverContext::from_state(&state);
            let plan = solver.decide(&ctx);
            assert_eq!(
                BudgetMode::Official.apply(&ctx, plan.clone()),
                plan,
                "normalization changed day {} {pattern}",
                state.day
            );
            for chunk in plan.actions.chunks(kinds.len()) {
                sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
            }
        }
        assert_eq!(state.illegal_action_count, 0);
    }
}
