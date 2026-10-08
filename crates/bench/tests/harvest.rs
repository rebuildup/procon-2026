//! Canonical regression tests for the independent all-spot harvest candidate.
use bench::CandidateRegistry;
use domain::{MatchId, PlayerId};
use engine::{
    MatchSim,
    model::{
        Agent, AgentActions, Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState,
        SolverContext, Spot, Terrain,
    },
};

fn state() -> MatchState {
    let cells = (0..8)
        .flat_map(|row| {
            (0..8).map(move |col| {
                let coord = HexCoord::new(col, row);
                Cell {
                    coord,
                    terrain: Terrain::Plain,
                    spot: (row == 0 && (1..=6).contains(&col)).then_some(Spot {
                        series: col as u32,
                        pos: coord,
                        max_stock: 8,
                        current_stock: 8,
                    }),
                }
            })
        })
        .collect();
    MatchState::new(MatchSetting {
        match_id: MatchId::new("harvest-test").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(8, 8).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 3),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(1, 1)),
        ],
        day_steps: 16,
        num_days: 2,
        day_steps_by_day: vec![],
    })
    .unwrap()
}

fn run_day(st: &mut MatchState) {
    let solver = CandidateRegistry::builtin()
        .build("harvest:budget=official")
        .expect("harvest registered")
        .build_solver()
        .unwrap();
    let ctx = SolverContext::from_state(st);
    let plan = solver.decide(&ctx);
    assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
    assert_eq!(plan, solver.decide(&ctx), "deterministic plan");
    for chunk in plan.actions.chunks(3) {
        MatchSim::default().step(st, &AgentActions::new(chunk.to_vec()));
    }
    assert_eq!(st.illegal_action_count, 0);
    assert_eq!(st.fuel_short_count, 0);
}

#[test]
fn harvest_uses_actual_supply_refills_to_exceed_initial_tank_range() {
    let mut st = state();
    run_day(&mut st);
    assert_eq!(st.agents[0].inventory.values().sum::<u32>(), 6);
    assert_eq!(st.series_taken.len(), 6);
}

#[test]
fn harvest_collects_fresh_start_spot_on_next_day() {
    let mut st = state();
    run_day(&mut st);
    run_day(&mut st);
    assert_eq!(st.agents[0].inventory.values().sum::<u32>(), 12);
}
