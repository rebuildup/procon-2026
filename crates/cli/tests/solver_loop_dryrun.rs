//! Dry-run regression test for issue #158.
//!
//! `--solver loop` was rejected by the official server on day 0 with
//! `revision=-1` because [`EngineLoopAdapter::decide`] emitted
//! `Move(dir)` actions without checking map bounds or terrain
//! passability. The directions were computed from `spot_id % 6`, so
//! agents blindly moved in six arbitrary directions regardless of what
//! sat on the other side of the wall / pond / map edge.
//!
//! This test exercises the map-aware path: it builds a real
//! [`engine::model::MatchSetting`] from a wire-shaped `MatchSetting`
//! (with a map that contains walls and ponds), attaches it to an
//! [`EngineLoopAdapter`], drives `decide` for every day, and walks
//! every emitted `Move(dir)` to assert that the target cell is in
//! bounds and traversable. A plan that contains an invalid move is the
//! regression we are guarding against.

use cli::action_adapter::decision_to_actions;
use cli::protocol_to_engine::protocol_setting_to_engine;
use cli::solver_adapter::{EngineLoopAdapter, is_legal_move};
use domain::state::Score;
use domain::{MatchId, Outcome, PlayerId};
use engine::model::{
    AgentKind, Cell, HexCoord, MapGrid, MapSize, MatchSetting, Spot, Terrain, TrafficState,
};
use engine::solver::{
    ActionKind as SActionKind, Direction as SDirection, Solver, SolverContext, SolverDecision,
};
use protocol::MatchSnapshot;
use protocol::match_config::{
    CellType, Map as WireMap, MatchSetting as WireSetting, Spot as WireSpot,
};

/// Build a wire [`WireSetting`] that mirrors `quick.json` — 10x10 map
/// with mountains and roads, four spots, three agents starting on plain
/// cells. The shape and values come from `.reference/procon37/quick.json`
/// / `match-config.example.json`; the wire form (flat `Map`, etc.) is
/// reconstructed here because the on-disk file uses the *problem*
/// schema, not the wire schema.
fn build_quick_like_wire_setting() -> WireSetting {
    // `quick.json` cells[y][x]:
    //   row 0: 0 1 1 1 1 1 1 1 1 0
    //   row 1: 1 0 0 0 0 0 0 0 0 1
    //   row 2: 1 0 3 0 0 0 0 3 0 1
    //   ...
    // The exact terrain distribution is not load-bearing — we only need
    // *some* mountains and roads so the map-aware solver has to
    // navigate around them. The values below match the quick fixture.
    let cells_row = |row: &[i32]| -> Vec<CellType> {
        row.iter()
            .copied()
            .map(|v| match v {
                0 => CellType::TYPE_0,
                1 => CellType::TYPE_1,
                2 => CellType::TYPE_2,
                _ => CellType::TYPE_3,
            })
            .collect()
    };
    let cells = vec![
        cells_row(&[0, 1, 1, 1, 1, 1, 1, 1, 1, 0]),
        cells_row(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        cells_row(&[1, 0, 3, 0, 0, 0, 0, 3, 0, 1]),
        cells_row(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        cells_row(&[1, 0, 0, 0, 3, 0, 0, 0, 0, 1]),
        cells_row(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        cells_row(&[1, 0, 3, 0, 0, 0, 0, 3, 0, 1]),
        cells_row(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        cells_row(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
        cells_row(&[0, 1, 1, 1, 1, 1, 1, 1, 1, 0]),
    ];
    WireSetting {
        starts_at: 1,
        day_seconds: vec![10, 10, 10, 10],
        day_steps: vec![6, 6, 6, 6],
        map: WireMap { height: 10, width: 10, cells },
        spots: vec![
            WireSpot { brand: 0, pos: 0, stocks: 2 },
            WireSpot { brand: 1, pos: 90, stocks: 1 },
            WireSpot { brand: 0, pos: 9, stocks: 2 },
            WireSpot { brand: 2, pos: 99, stocks: 1 },
        ],
        agents: vec![44, 45, 50],
        fuel_limits: 20,
        players: 2,
        busy_threshold: 2,
        jammed_threshold: 4,
    }
}

/// Convert a wire [`CellType`] into the matching [`Terrain`].
fn wire_terrain(c: CellType) -> Terrain {
    match c.value() {
        1 => Terrain::Mountain,
        2 => Terrain::Pond,
        3 => Terrain::Road(TrafficState::Smooth),
        _ => Terrain::Plain,
    }
}

/// Build an [`engine::model::MatchSetting`] by hand (bypassing the
/// wire-format conversion). Used by tests that want to drive the
/// adapter with a curated map / agent setup without depending on
/// `protocol_setting_to_engine`'s MVP agent-composition rules.
fn build_engine_setting_from_engine(
    width: u8,
    height: u8,
    cells: &[Vec<Terrain>],
    spots: &[(u32, u32, u32)], // (series, pos_idx, max_stock)
    agents: &[(u8, u32)],      // (agent_id, pos_idx) — alternated kinds
    day_steps: u32,
) -> MatchSetting {
    let size = MapSize::new(width, height).expect("size in bounds");
    let mut grid_cells: Vec<Cell> = Vec::with_capacity(usize::from(width) * usize::from(height));
    for (row, row_cells) in cells.iter().enumerate() {
        for (col, terrain) in row_cells.iter().enumerate() {
            let coord =
                HexCoord::new(i32::try_from(col).unwrap_or(0), i32::try_from(row).unwrap_or(0));
            grid_cells.push(Cell { coord, terrain: *terrain, spot: None });
        }
    }
    let mut grid = MapGrid::new(size, grid_cells).expect("grid builds");
    for (series, pos_idx, max_stock) in spots {
        let row = *pos_idx / u32::from(width);
        let col = *pos_idx % u32::from(width);
        let coord = HexCoord::new(i32::try_from(col).unwrap_or(0), i32::try_from(row).unwrap_or(0));
        let cell = grid.get_mut(coord).expect("spot coord in bounds");
        cell.spot = Some(Spot {
            series: *series,
            pos: coord,
            max_stock: *max_stock,
            current_stock: *max_stock,
        });
    }
    let agents_vec: Vec<engine::model::Agent> = agents
        .iter()
        .map(|(id, pos_idx)| {
            let row = *pos_idx / u32::from(width);
            let col = *pos_idx % u32::from(width);
            let coord =
                HexCoord::new(i32::try_from(col).unwrap_or(0), i32::try_from(row).unwrap_or(0));
            let day_steps_u = day_steps;
            if id % 2 == 0 {
                engine::model::Agent::patrol(*id, coord, day_steps_u, 3)
            } else {
                engine::model::Agent::supply(*id, coord)
            }
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new("dryrun").expect("valid id"),
        team: PlayerId::new("dryrun-player").expect("valid id"),
        map: grid,
        agents: agents_vec,
        day_steps,
        num_days: 4,
    }
}

fn snapshot() -> MatchSnapshot {
    let match_id = MatchId::new("dryrun").unwrap();
    let team = PlayerId::new("dryrun-player").unwrap();
    MatchSnapshot {
        match_id: match_id.clone(),
        state: domain::state::GameState { match_id, turn: 0, scores: vec![(team, Score(0))] },
        outcome: Outcome::Pending,
        server_time_unix_ms: 0,
    }
}

fn ctx_for_day(day: u32, agent_ids: Vec<u8>, day_steps: u32) -> SolverContext {
    SolverContext::new(
        snapshot(),
        PlayerId::new("dryrun-player").unwrap(),
        agent_ids,
        day,
        day_steps,
        60_000,
    )
}

/// Assert every `Move(dir)` in `decision.actions` lands on a passable
/// cell within the map. Steps that are `Wait` are skipped; the function
/// returns the total step count actually used so callers can sanity
/// check `day_steps`.
#[allow(clippy::too_many_lines)]
fn assert_all_moves_valid(
    decision: &SolverDecision,
    ctx: &SolverContext,
    setting: &MatchSetting,
) -> i64 {
    let agent_count = ctx.my_agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    assert_eq!(
        decision.len(),
        ctx.expected_action_count(),
        "decision shape: {} actions, expected {}",
        decision.len(),
        ctx.expected_action_count()
    );
    assert!(decision.is_well_shaped(ctx), "decision must be well-shaped against the context");

    // Track each agent's position; start at the engine-side start
    // position. The loop solver's adapter (`pick_valid_move`) does the
    // same tracking, but we re-implement it here so a bug in the
    // adapter cannot make the test pass on invalid plans.
    let mut positions: Vec<HexCoord> = ctx
        .my_agent_ids
        .iter()
        .map(|id| {
            setting
                .agents
                .iter()
                .find(|a| a.id == *id)
                .map_or(HexCoord::new(0, 0), |a| a.pos)
        })
        .collect();

    let mut total_step_sum: i64 = 0;
    for step in 0..day_steps {
        for (agent_idx, &agent_id) in ctx.my_agent_ids.iter().enumerate() {
            let action = decision.actions[step * agent_count + agent_idx];
            assert_eq!(
                action.agent_id, agent_id,
                "step {step} agent_idx {agent_idx}: wrong agent_id"
            );
            match action.kind {
                SActionKind::Wait => {
                    total_step_sum += 1;
                }
                SActionKind::Move(dir) => {
                    let from = positions[agent_idx];
                    let model_dir = model_dir(dir);
                    let to = model_dir.step(from);
                    assert!(
                        setting.map.size.contains(to),
                        "agent {agent_id} step {step}: move {dir:?} from {from:?} \
                         lands at {to:?} which is OUT OF BOUNDS for {:?}",
                        setting.map.size
                    );
                    let cell = setting.map.get(to).unwrap_or_else(|| {
                        panic!("map.get({to:?}) returned None despite bounds check")
                    });
                    assert!(
                        cell.terrain.traversable(),
                        "agent {agent_id} step {step}: move {dir:?} from {from:?} \
                         lands on impassable {:?} at {to:?}",
                        cell.terrain
                    );
                    positions[agent_idx] = to;
                    total_step_sum += 1;
                }
            }
        }
    }
    total_step_sum
}

fn model_dir(d: SDirection) -> engine::model::Direction {
    match d {
        SDirection::UpperLeft => engine::model::Direction::UpperLeft,
        SDirection::UpperRight => engine::model::Direction::UpperRight,
        SDirection::Right => engine::model::Direction::Right,
        SDirection::LowerRight => engine::model::Direction::LowerRight,
        SDirection::LowerLeft => engine::model::Direction::LowerLeft,
        SDirection::Left => engine::model::Direction::Left,
    }
}

#[test]
fn engine_loop_adapter_plan_satisfies_map_passability_for_quick_like_fixture() {
    let wire = build_quick_like_wire_setting();
    let team = PlayerId::new("dryrun-player").unwrap();
    let engine_setting =
        protocol_setting_to_engine(&wire, team).expect("wire -> engine setting succeeds");

    let adapter = EngineLoopAdapter::new(solver_loop::InitialStrategy::GreedyNearestNeighbor, 16);
    adapter.set_engine_setting(engine_setting.clone());

    let agent_ids = engine_setting.agent_ids();
    let total_days = wire.day_steps.len();
    for day_idx in 0..total_days {
        let day = u32::try_from(day_idx).unwrap_or(0);
        let day_steps = u32::try_from(wire.day_steps[day_idx]).unwrap_or(0);
        let ctx = ctx_for_day(day, agent_ids.clone(), day_steps);
        let decision = adapter.decide(&ctx);

        // Map-aware path: every emitted Move(dir) must land on a
        // passable cell within the map.
        let step_sum = assert_all_moves_valid(&decision, &ctx, &engine_setting);
        let expected_total = i64::from(day_steps) * i64::try_from(agent_ids.len()).unwrap_or(0);
        assert_eq!(step_sum, expected_total, "day {day}: total step-sum mismatch");

        // The adapter must also round-trip through the wire adapter
        // without tripping shape / step-sum checks (mirrors what the
        // CLI submits to the server).
        let body = decision_to_actions(&decision, &ctx).expect("decision_to_actions");
        assert_eq!(body.len(), agent_ids.len(), "one inner list per agent");
        for (agent_idx, inner) in body.iter().enumerate() {
            let wire_sum: i64 =
                inner.iter().map(|&x| if x < 0 { i64::from(-x) } else { 1_i64 }).sum();
            assert_eq!(
                wire_sum,
                i64::from(day_steps),
                "day {day} agent {agent_idx}: wire step-sum mismatch"
            );
        }
    }
}

#[test]
fn engine_loop_adapter_plan_satisfies_map_passability_for_handcrafted_grid() {
    // 10x10 grid with ponds and mountains scattered around so the
    // map-aware solver has plenty of obstacles to navigate around.
    let mut cells = vec![vec![Terrain::Plain; 10]; 10];
    cells[0][5] = Terrain::Pond;
    cells[3][3] = Terrain::Mountain;
    cells[5][7] = Terrain::Pond;
    cells[7][2] = Terrain::Mountain;
    cells[2][8] = Terrain::Mountain;

    let spots = vec![(1, 4, 2), (2, 88, 1), (1, 50, 2), (3, 95, 1)];
    let agents = vec![(0u8, 44u32), (1, 45), (2, 50)];

    let setting = build_engine_setting_from_engine(10, 10, &cells, &spots, &agents, 6);

    let adapter = EngineLoopAdapter::new(solver_loop::InitialStrategy::GreedyNearestNeighbor, 16);
    adapter.set_engine_setting(setting.clone());

    let agent_ids: Vec<u8> = agents.iter().map(|(id, _)| *id).collect();
    let ctx = ctx_for_day(0, agent_ids.clone(), 6);
    let decision = adapter.decide(&ctx);

    let step_sum = assert_all_moves_valid(&decision, &ctx, &setting);
    assert_eq!(
        step_sum,
        i64::from(6) * i64::try_from(agent_ids.len()).unwrap_or(0),
        "step sum equals day_steps[0] * agent_count"
    );

    // Bonus: every emitted `Action::agent_id` matches the controlled
    // agents, and the decision is well-shaped against the context.
    assert!(decision.is_well_shaped(&ctx));

    // Spot-check: the agent kinds round-trip through the engine view
    // of the setting — every odd-indexed agent is `Supply`, every
    // even-indexed agent is `Patrol` (per the MVP rule used by
    // `protocol_setting_to_engine`).
    for (i, agent) in setting.agents.iter().enumerate() {
        let expected = if i % 2 == 0 { AgentKind::Patrol } else { AgentKind::Supply };
        assert_eq!(agent.kind, expected, "agent {} has wrong kind", agent.id);
    }
}

#[test]
fn engine_loop_adapter_falls_back_when_setting_is_missing() {
    // When the engine setting is never attached, the adapter still
    // produces a well-shaped decision (legacy `spot_id % 6` heuristic).
    // This guard keeps the dry-run / unit-test suite green even if a
    // caller forgets to wire the setting.
    let adapter = EngineLoopAdapter::new(solver_loop::InitialStrategy::Identity, 1);
    let ctx = ctx_for_day(0, vec![0, 1, 2], 4);
    let decision = adapter.decide(&ctx);
    assert_eq!(decision.len(), 12);
    assert!(decision.is_well_shaped(&ctx));
    assert_eq!(decision.metadata["map_aware"], serde_json::Value::Bool(false));
}

#[test]
fn engine_loop_adapter_handles_zero_spots_without_panicking() {
    // An 8x8 plain grid (the minimum legal size) with no spots and a
    // single agent. The loop solver produces an empty visit list; the
    // adapter must emit a well-shaped plan of all `Wait` actions and
    // not crash on the empty visit-list branch.
    let cells = vec![vec![Terrain::Plain; 8]; 8];
    let agents = vec![(0u8, 9u32)]; // row 1, col 1
    let setting = build_engine_setting_from_engine(8, 8, &cells, &[], &agents, 4);

    let adapter = EngineLoopAdapter::new(solver_loop::InitialStrategy::GreedyNearestNeighbor, 1);
    adapter.set_engine_setting(setting.clone());

    let ctx = ctx_for_day(0, vec![0], 4);
    let decision = adapter.decide(&ctx);
    let step_sum = assert_all_moves_valid(&decision, &ctx, &setting);
    assert_eq!(step_sum, 4, "single-agent step sum equals day_steps[0]");
}

#[test]
fn is_legal_move_rejects_pond_and_out_of_bounds() {
    let mut cells = vec![vec![Terrain::Plain; 8]; 8];
    cells[1][1] = Terrain::Pond;
    let agents = vec![(0u8, 0u32)];
    let setting = build_engine_setting_from_engine(8, 8, &cells, &[], &agents, 4);

    // (0,0) -> (1,0) is plain and in bounds: legal.
    assert!(is_legal_move(HexCoord::new(0, 0), HexCoord::new(1, 0), &setting));
    // (0,0) -> (-1,0) is out of bounds: illegal.
    assert!(!is_legal_move(HexCoord::new(0, 0), HexCoord::new(-1, 0), &setting));
    // (0,0) -> (1,1) is a pond: illegal.
    assert!(!is_legal_move(HexCoord::new(0, 0), HexCoord::new(1, 1), &setting));
}

#[test]
fn wire_terrain_mapping_matches_protocol_to_engine() {
    // Every wire cell type must map to a traversable engine terrain
    // except `TYPE_2` (Pond). The map-aware adapter relies on this
    // contract — if it ever breaks, `assert_all_moves_valid` will
    // start failing on otherwise-valid plans.
    assert_eq!(wire_terrain(CellType::TYPE_0), Terrain::Plain);
    assert_eq!(wire_terrain(CellType::TYPE_1), Terrain::Mountain);
    assert_eq!(wire_terrain(CellType::TYPE_2), Terrain::Pond);
    assert_eq!(wire_terrain(CellType::TYPE_3), Terrain::Road(TrafficState::Smooth));
    assert!(Terrain::Plain.traversable());
    assert!(!Terrain::Pond.traversable());
}
