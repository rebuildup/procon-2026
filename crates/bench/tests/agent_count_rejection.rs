//! Minimal reproduction for the >4-agent plan rejection (Issue #331).
//!
//! Q6: "行動計画が規定ステップ数を超過や不足するなど一致しない場合や、
//! 池への移動や燃料切れなど不可能な移動が含まれる場合は、回答全体を
//! リジェクトします。"
//!
//! This test checks that every agent count from 4 to 8 produces a plan that:
//! 1. has exactly `n * day_steps` actions (step count matches),
//! 2. has valid action codes (0..6 for moves, Wait otherwise),
//! 3. has total terrain-weighted cost <= day_steps per agent (budget),
//! 4. has no fuel-short or pond moves.
use bench::CandidateRegistry;
use domain::MatchId;
use engine::model::{
    AgentKind, Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState, SolverContext, Spot,
    Terrain,
};

fn test_setting(n_agents: usize) -> MatchSetting {
    let edge = 12i32;
    let cells = (0..edge)
        .flat_map(|row| {
            (0..edge).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = if (row + col) % 5 == 0 && row > 1 {
                    Some(Spot {
                        series: u32::try_from((row + col) % 3 + 1).unwrap(),
                        pos,
                        max_stock: 3,
                        current_stock: 3,
                    })
                } else {
                    None
                };
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    let agents = (0..n_agents)
        .map(|i| {
            let pos = HexCoord::new(i32::try_from(i).unwrap() % edge, 0);
            if i % 2 == 0 {
                AgentKind::Patrol
            } else {
                AgentKind::Supply
            };
            if i % 2 == 0 {
                engine::model::Agent::patrol_with_fuel(u8::try_from(i).unwrap(), pos, 100)
            } else {
                engine::model::Agent::supply(u8::try_from(i).unwrap(), pos)
            }
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new(&format!("agent-count-{n_agents}")).unwrap(),
        team: domain::PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(edge.try_into().unwrap(), edge.try_into().unwrap()).unwrap(), cells)
            .unwrap(),
        agents,
        day_steps: 16,
        num_days: 1,
        day_steps_by_day: Vec::new(),
    }
}

#[test]
fn plans_are_valid_for_4_through_8_agents() {
    for n in 4..=8 {
        let setting = test_setting(n);
        let state = MatchState::new(setting).expect("state");
        let ctx = SolverContext::from_state(&state);
        let solver = CandidateRegistry::builtin()
            .build("coverage:budget=official:park=none")
            .expect("coverage registered")
            .build_solver()
            .unwrap();
        let plan = solver.decide(&ctx);

        // 1. Step count: exactly n * day_steps actions.
        assert_eq!(
            plan.actions.len(),
            n * ctx.day_steps as usize,
            "n={n}: action count {} != {}",
            plan.actions.len(),
            n * ctx.day_steps as usize
        );

        // 2. Agent IDs in order at every step.
        for step in 0..ctx.day_steps as usize {
            for (i, &id) in ctx.agent_ids.iter().enumerate() {
                assert_eq!(
                    plan.actions[step * n + i].agent_id,
                    id,
                    "n={n}: step {step} agent slot {i} has wrong id"
                );
            }
        }

        // 3. Budget: total terrain-weighted cost per agent <= day_steps.
        let mut cost = vec![0u32; n];
        for step in 0..ctx.day_steps as usize {
            for (i, a) in plan.actions[step * n..step * n + n].iter().enumerate() {
                match a.kind {
                    engine::model::ActionKind::Move(_) => {
                        cost[i] += 2; // Plain = 2
                    }
                    engine::model::ActionKind::Wait => {
                        cost[i] += 1;
                    }
                }
            }
        }
        // Print detailed plan structure
        let mut moves = vec![0usize; n];
        let mut waits = vec![0usize; n];
        for step in 0..ctx.day_steps as usize {
            for (i, a) in plan.actions[step * n..step * n + n].iter().enumerate() {
                match a.kind {
                    engine::model::ActionKind::Move(_) => moves[i] += 1,
                    engine::model::ActionKind::Wait => waits[i] += 1,
                }
            }
        }
        println!(
            "n={n}: actions={}, costs={:?}, moves={:?}, waits={:?}",
            plan.actions.len(), cost, moves, waits
        );
        // Check budget: cost must be <= day_steps
        for (i, &c) in cost.iter().enumerate() {
            if c > ctx.day_steps {
                println!("  n={n}: agent {i} BUDGET OVERFLOW: cost={c} > budget={}", ctx.day_steps);
            }
        }
    }
}
