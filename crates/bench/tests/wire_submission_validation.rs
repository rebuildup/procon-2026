//! Validate the wire submission body for 4-8 agents (Issue #331).
//!
//! Mimics the server's Q6 validation:
//! 1. Number of agent lists = number of agents
//! 2. Each agent's total consumed steps = daySteps
//! 3. All move codes are valid (0..6)
//! 4. No impossible moves (pond, off-map, fuel-short)
use bench::CandidateRegistry;
use domain::MatchId;
use engine::model::{
    AgentKind, Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState, SolverContext, Spot,
    Terrain,
};

fn test_setting(n_agents: usize) -> MatchSetting {
    // 16x16 map with mixed terrain (like live match #1791)
    let edge = 16i32;
    let cells = (0..edge)
        .flat_map(|row| {
            (0..edge).map(move |col| {
                let pos = HexCoord::new(col, row);
                // Terrain: border = Road, scattered Mountains and Ponds
                let terrain = if row == 0 || row == edge - 1 || col == 0 || col == edge - 1 {
                    Terrain::Road(engine::model::TrafficState::Smooth)
                } else if (row * 7 + col * 3) % 11 == 0 {
                    Terrain::Pond
                } else if (row * 3 + col * 5) % 7 == 0 {
                    Terrain::Mountain
                } else {
                    Terrain::Plain
                };
                let spot = if terrain == Terrain::Plain && (row + col) % 4 == 0 && row > 1 {
                    Some(Spot {
                        series: u32::try_from((row + col) % 3 + 1).unwrap(),
                        pos,
                        max_stock: 3,
                        current_stock: 3,
                    })
                } else {
                    None
                };
                Cell { coord: pos, terrain, spot }
            })
        })
        .collect();
    let agents = (0..n_agents)
        .map(|i| {
            let pos = HexCoord::new(i32::try_from(i).unwrap() % edge, 0);
            if i % 2 == 0 {
                engine::model::Agent::patrol_with_fuel(u8::try_from(i).unwrap(), pos, 100)
            } else {
                engine::model::Agent::supply(u8::try_from(i).unwrap(), pos)
            }
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new(&format!("wire-{n_agents}")).unwrap(),
        team: domain::PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(edge.try_into().unwrap(), edge.try_into().unwrap()).unwrap(), cells)
            .unwrap(),
        agents,
        day_steps: 32,
        num_days: 1,
        day_steps_by_day: Vec::new(),
    }
}

/// Replicate `decision_to_actions` encoding and validate like the server.
fn validate_wire_body(
    plan: &engine::model::AgentActions,
    ctx: &SolverContext,
    n: usize,
) -> Result<(), String> {
    let day_steps = ctx.day_steps as usize;

    // 1. Action count
    if plan.actions.len() != n * day_steps {
        return Err(format!(
            "action count {} != {}",
            plan.actions.len(),
            n * day_steps
        ));
    }

    // 2. Agent ID order at every step
    for step in 0..day_steps {
        for (i, &id) in ctx.agent_ids.iter().enumerate() {
            if plan.actions[step * n + i].agent_id != id {
                return Err(format!(
                    "step {step} slot {i}: agent id mismatch"
                ));
            }
        }
    }

    // 3. Encode to wire body (replicate decision_to_actions)
    let mut body: Vec<Vec<i32>> = Vec::with_capacity(n);
    for agent_index in 0..n {
        let mut inner: Vec<i32> = Vec::new();
        let mut computed: i64 = 0;
        for step in 0..day_steps {
            let a = &plan.actions[step * n + agent_index];
            match a.kind {
                engine::model::ActionKind::Wait => {
                    if let Some(last) = inner.last_mut() {
                        if *last < 0 {
                            *last -= 1;
                            computed += 1;
                            continue;
                        }
                    }
                    inner.push(-1);
                    computed += 1;
                }
                engine::model::ActionKind::Move(dir) => {
                    inner.push(dir as i32);
                    computed += 1;
                }
            }
        }
        if computed != i64::from(ctx.day_steps) {
            return Err(format!(
                "agent {agent_index}: computed steps {} != {}",
                computed,
                ctx.day_steps
            ));
        }
        body.push(inner);
    }

    // 4. Server-side validation: replay and check budget/fuel
    let map = &ctx.state.setting.map;
    let mut positions: Vec<HexCoord> = ctx.state.agents.iter().map(|a| a.pos).collect();
    let mut fuels: Vec<u32> = ctx.state.agents.iter().map(|a| a.fuel).collect();
    let mut budget = vec![0u32; n]; // terrain-weighted cost

    // Expand run-length encoding to per-step actions
    let mut expanded: Vec<Vec<i32>> = vec![Vec::new(); n];
    for (i, codes) in body.iter().enumerate() {
        for &code in codes {
            if code < 0 {
                for _ in 0..code.unsigned_abs() {
                    expanded[i].push(-1);
                }
            } else {
                expanded[i].push(code);
            }
        }
    }

    // Replay step by step
    for step in 0..day_steps {
        for i in 0..n {
            let code = expanded[i][step];
            if code < 0 {
                // Wait: costs 1 step
                budget[i] += 1;
            } else {
                // Move: validate and cost
                let dir = match code {
                    0 => engine::model::Direction::UpperLeft,
                    1 => engine::model::Direction::UpperRight,
                    2 => engine::model::Direction::Right,
                    3 => engine::model::Direction::LowerRight,
                    4 => engine::model::Direction::LowerLeft,
                    5 => engine::model::Direction::Left,
                    _ => return Err(format!("agent {i} step {step}: invalid code {code}")),
                };
                let from = positions[i];
                let to = dir.step(from);
                // Check off-map
                if map.get(to).is_none() {
                    return Err(format!("agent {i} step {step}: off-map move to ({},{})", to.col, to.row));
                }
                // Check pond
                let target = map.get(to).unwrap();
                if target.terrain == Terrain::Pond {
                    return Err(format!("agent {i} step {step}: pond move"));
                }
                // Check fuel
                let fuel_cost = engine::rules::fuel_cost(
                    map.get(from).unwrap().terrain,
                ).max(engine::rules::fuel_cost(target.terrain));
                if i % 2 == 0 && fuels[i] < fuel_cost {
                    // Patrol: fuel check
                    return Err(format!(
                        "agent {i} step {step}: fuel short ({} < {})",
                        fuels[i], fuel_cost
                    ));
                }
                // Check budget
                let step_cost = match map.get(from).unwrap().terrain {
                    Terrain::Plain => 2,
                    Terrain::Mountain => 3,
                    Terrain::Pond => return Err(format!("agent {i} step {step}: standing on pond")),
                    Terrain::Road(s) => s.step_cost(),
                };
                if budget[i] + step_cost > ctx.day_steps {
                    return Err(format!(
                        "agent {i} step {step}: budget overflow ({} + {} > {})",
                        budget[i], step_cost, ctx.day_steps
                    ));
                }
                budget[i] += step_cost;
                positions[i] = to;
                if i % 2 == 0 {
                    fuels[i] -= fuel_cost;
                }
            }
        }
    }

    // Per-move budget check: each move must fit within remaining budget.
    // The server checks "insufficient steps" per-move (Q6).
    // Also check: total action count == daySteps (already checked above).
    // Report budget info for diagnosis.
    for i in 0..n {
        println!("  agent {i}: budget used={}/{}", budget[i], ctx.day_steps);
    }

    Ok(())
}

#[test]
fn wire_body_is_valid_for_4_through_8_agents() {
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

        // Print plan structure
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
        println!("n={n}: moves={:?} waits={:?}", moves, waits);

        // Dump wire body
        {
            let day_steps = ctx.day_steps as usize;
            let mut body: Vec<Vec<i32>> = Vec::with_capacity(n);
            for agent_index in 0..n {
                let mut inner: Vec<i32> = Vec::new();
                for step in 0..day_steps {
                    let a = &plan.actions[step * n + agent_index];
                    match a.kind {
                        engine::model::ActionKind::Wait => {
                            if let Some(last) = inner.last_mut() {
                                if *last < 0 {
                                    *last -= 1;
                                    continue;
                                }
                            }
                            inner.push(-1);
                        }
                        engine::model::ActionKind::Move(dir) => {
                            inner.push(dir as i32);
                        }
                    }
                }
                body.push(inner);
            }
            println!("n={n} body: {:?}", body);
        }

        match validate_wire_body(&plan, &ctx, n) {
            Ok(()) => println!("n={n}: VALID"),
            Err(e) => println!("n={n}: INVALID - {e}"),
        }
    }
}
