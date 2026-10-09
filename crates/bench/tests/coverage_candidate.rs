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
fn coverage_replans_daily_and_harvests_after_covering_series() {
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let result = MatchSim::default().run(setting(32), solver.as_ref()).unwrap();
    assert_eq!((result.variety, result.daily_variety_sum, result.total_balls), (3, 6, 8));
    assert_eq!(result.illegal_action_count, 0);
}

#[test]
fn coverage_uses_supply_contact_to_continue_beyond_initial_fuel() {
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let result = MatchSim::default().run(setting(2), solver.as_ref()).unwrap();
    assert_eq!((result.variety, result.daily_variety_sum, result.total_balls), (3, 6, 8));
    assert_eq!(result.fuel_short_count, 0);
}

#[test]
fn coverage_does_not_depend_on_offline_only_series_history() {
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let mut state = MatchState::new(setting(32)).unwrap();
    let original = solver.decide(&SolverContext::from_state(&state));
    state.series_taken.extend([1, 2, 3]);
    assert_eq!(solver.decide(&SolverContext::from_state(&state)), original);
}

#[test]
fn coverage_official_normalization_preserves_every_action() {
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
            .build("coverage:budget=official")
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

/// Long-day board used to exercise the park-spreading gate (Issue #331).
/// Two series sit in opposite corners so a single patrol must choose a
/// target order; `day_claimed` spreading flips that order on large maps.
fn long_day_setting(edge: u8) -> MatchSetting {
    let e = i32::from(edge);
    let cells = (0..e)
        .flat_map(|row| {
            (0..e).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = if pos == HexCoord::new(e - 1, 0) {
                    Some(Spot { series: 1, pos, max_stock: 3, current_stock: 3 })
                } else if pos == HexCoord::new(0, e - 1) {
                    Some(Spot { series: 2, pos, max_stock: 3, current_stock: 3 })
                } else {
                    None
                };
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new("coverage-gate-test").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(edge, edge).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol(0, HexCoord::new(0, 0), 80, 3),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(0, 2)),
        ],
        day_steps: 80,
        num_days: 2,
        day_steps_by_day: vec![80, 80],
    }
}

fn first_day_plan(setting: MatchSetting, candidate: &str) -> engine::model::AgentActions {
    let state = MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin().build(candidate).unwrap().build_solver().unwrap();
    solver.decide(&SolverContext::from_state(&state))
}

/// Park spreading is gated to maps whose edge reaches 24: on small boards
/// nearest-first already covers every series, so `park=far` must not
/// distort positioning (Issue #331).
#[test]
fn park_far_matches_park_none_on_small_maps() {
    let none = first_day_plan(long_day_setting(8), "coverage:budget=official:park=none");
    let far = first_day_plan(long_day_setting(8), "coverage:budget=official:park=far");
    assert_eq!(far, none, "park=far must stay byte-identical to park=none when width/height < 24");
}

/// On maps whose edge reaches 24 the gate opens: `park=far` must actually
/// spread across unclaimed series, so its first-day plan differs from
/// `park=none`. If this fails, the map-size condition is blocking the
/// short-day coverage fix that Issue #331 needs.
#[test]
fn park_far_spreads_on_maps_of_edge_24_or_more() {
    let none = first_day_plan(long_day_setting(24), "coverage:budget=official:park=none");
    let far = first_day_plan(long_day_setting(24), "coverage:budget=official:park=far");
    assert_ne!(
        far, none,
        "park=far must diverge from park=none when width/height >= 24 and days are long"
    );
}
