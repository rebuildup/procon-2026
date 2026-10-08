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

/// Issue #311: the wire normalizer judges plans the way the server does —
/// Q6/Q10 ticks (a move consumes its origin cell's step count), fuel charged
/// in the reflection phase, and Supply refuel whenever the two share a cell.
/// Its only liberty is **cutting**: the output keeps an action prefix of the
/// input and turns the rest into waits, and it is idempotent.
///
/// Full preservation is *not* a valid contract. On arena-701's day 8 a plan
/// costs 74 weighted ticks for a 53-tick day and contains moves the day's
/// fuel cannot pay under the official replay; the pre-tick normalizer passed
/// those through and the server discarded the whole answer with
/// `revision=-1` (live match 1762). What `budget=official` keeps is exactly
/// what live executes, which is the property that matters.
#[test]
fn coverage_normalization_only_cuts_and_pads() {
    use bench::adapters::walk::BudgetMode;
    use bench::case::CaseSpec;
    use bench::kinds::{parse_kinds, resolve_kinds};
    use engine::model::{ActionKind, AgentActions};
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
            let applied = BudgetMode::Official.apply(&ctx, plan.clone());
            let n = ctx.agent_ids.len();
            for slot in 0..n {
                let input: Vec<ActionKind> =
                    plan.actions.iter().skip(slot).step_by(n).map(|a| a.kind).collect();
                let output: Vec<ActionKind> =
                    applied.actions.iter().skip(slot).step_by(n).map(|a| a.kind).collect();
                let mut kept = 0;
                while kept < output.len() && kept < input.len() && output[kept] == input[kept] {
                    kept += 1;
                }
                assert!(
                    output[kept..].iter().all(|k| matches!(k, ActionKind::Wait)),
                    "day {} {pattern} slot {slot}: normalization may only replace a suffix \
                     with waits, got {:?} vs {:?}",
                    state.day,
                    &output[kept..],
                    &input[kept.min(input.len())..]
                );
            }
            // Idempotent: re-running the official judge changes nothing.
            assert_eq!(
                BudgetMode::Official.apply(&ctx, applied.clone()),
                applied,
                "day {} {pattern}: normalization must be idempotent",
                state.day
            );
            for chunk in plan.actions.chunks(kinds.len()) {
                sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
            }
        }
        assert_eq!(state.illegal_action_count, 0);
    }
}
