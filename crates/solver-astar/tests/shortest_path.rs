//! End-to-end tests of [`AstarSolver`] on small hand-built maps.
//!
//! Each scenario fixes the terrain, traffic, start, goal, and fuel budget,
//! then asserts on:
//! - path length (number of moves),
//! - that the path avoids ponds and stays inside the map,
//! - that the path obeys the cost policy (traffic / fuel cases).

use solver_astar::{
    Action, AstarSolver, CellKind, CostPolicy, Direction, HexCoord, HexMap, SolveContext, Solver,
    TrafficState,
};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Build a `width x height` map from a row-of-rows textual fixture.
///
/// Encoding:
/// - `.` plain
/// - `M` mountain
/// - `~` pond (impassable)
/// - `s` smooth road
/// - `b` busy road
/// - `j` jammed road
fn map_from_str(width: u32, height: u32, rows: &[&str]) -> HexMap {
    assert_eq!(u32::try_from(rows.len()), Ok(height), "fixture must declare {height} rows");
    let mut cells = Vec::with_capacity(width as usize * height as usize);
    for (row_idx, row) in rows.iter().enumerate() {
        assert_eq!(
            u32::try_from(row.chars().count()),
            Ok(width),
            "row {row_idx} must be exactly {width} chars wide"
        );
        for ch in row.chars() {
            let cell = match ch {
                '.' => CellKind::Plain,
                'M' => CellKind::Mountain,
                '~' => CellKind::Pond,
                's' => CellKind::Road { traffic: TrafficState::Smooth },
                'b' => CellKind::Road { traffic: TrafficState::Busy },
                'j' => CellKind::Road { traffic: TrafficState::Jammed },
                other => panic!("unknown fixture char {other:?}"),
            };
            cells.push(cell);
        }
    }
    HexMap::new(width, height, cells).expect("fixture map must validate")
}

// ---------------------------------------------------------------------------
// Case 1 — straight plain open map
// ---------------------------------------------------------------------------

#[test]
fn straight_open_map_finds_minimum_step_path() {
    let rows = &[
        "........", "........", "........", "........", "........", "........", "........",
        "........",
    ];
    let map = map_from_str(8, 8, rows);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map.clone(), start, goal, 100, 50).unwrap();

    let solver = AstarSolver::default();
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");

    // Step 1: every step costs 1, so any 7-step path is optimal; the
    // minimum hex distance from (0,4) to (7,4) is exactly 7.
    assert_eq!(path.step_total(), 7, "minimum hex distance is 7 eastward");
    assert_eq!(path.steps.len(), 7);

    // Step 2: every move must point east (the only way to advance 7 cells
    // in a row while staying inside the map is east).
    for step in &path.steps {
        assert_eq!(*step, Action::Move(Direction::East));
    }

    // Step 3: positions reach the goal.
    let positions = path.positions(start);
    assert_eq!(positions.last().copied(), Some(goal));
    assert_eq!(positions.len(), 8);
}

#[test]
fn straight_open_map_with_insufficient_steps_fails() {
    let map = map_from_str(8, 8, &["........"; 8]);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map, start, goal, 100, 6).unwrap();
    let solver = AstarSolver::default();
    let result = solver.solve(&ctx);
    assert!(result.path.is_none(), "6-step budget cannot reach 7 cells away");
}

// ---------------------------------------------------------------------------
// Case 2 — pond barrier forces a longer path
// ---------------------------------------------------------------------------

#[test]
fn pond_barrier_forces_five_step_detour() {
    // Pond wall blocks cols 3..=7 on rows 2..=4. Start (1, 4) and goal
    // (5, 7) live on opposite sides.
    // Direct hex distance = 5 (NE x5 from start? No — start is at (1,4).
    // Going NE from (1,4) [even row] -> (1,3). Repeating: (1,2), (1,1),
    // (1,0). That's only 4 NE moves and ends at (1,0), not the goal.
    // The solver will find a detour through the south plain region.
    let rows = &[
        "........", "........", "...~~~~~", "...~~~~~", "...~~~~~", "........", "........",
        "........",
    ];
    let map = map_from_str(8, 8, rows);
    let start = HexCoord::new(1, 4);
    let goal = HexCoord::new(5, 7);
    let ctx = SolveContext::new(map.clone(), start, goal, 200, 50).unwrap();

    let solver = AstarSolver::default();
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");

    // Detour constraint: the path must never step on a pond cell.
    let positions = path.positions(start);
    for pos in &positions {
        let cell = map.get(*pos).expect("positions must be in-bounds");
        assert!(cell.is_passable(), "path stepped on impassable cell at {pos:?}");
    }

    // Goal reached, and the path is non-trivial (must go around the
    // 3-row pond wall).
    assert_eq!(positions.last().copied(), Some(goal));
    assert!(path.step_total() >= 5, "expected at least 5-step detour, got {}", path.step_total());
}

#[test]
fn pond_cell_is_never_a_neighbour_in_path() {
    let rows = &[
        "........", "..~.....", "..~.....", "........", "........", "........", "........",
        "........",
    ];
    let map = map_from_str(8, 8, rows);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map.clone(), start, goal, 200, 50).unwrap();

    let solver = AstarSolver::default();
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");
    let positions = path.positions(start);
    for pos in &positions {
        assert_ne!(*pos, HexCoord::new(2, 1));
        assert_ne!(*pos, HexCoord::new(2, 2));
        // Goal reachable since row 4 has no ponds.
        let cell = map.get(*pos).unwrap();
        assert!(cell.is_passable(), "path stepped on pond at {pos:?}");
    }
}

// ---------------------------------------------------------------------------
// Case 3 — fuel-aware: solver respects strict / conservative policies
// ---------------------------------------------------------------------------

#[test]
fn fuel_constraint_blocks_when_initial_fuel_is_below_distance() {
    // Goal is 7 plain cells east of start; each cell consumes 1 fuel.
    let map = map_from_str(8, 8, &["........"; 8]);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map, start, goal, 6, 50).unwrap();
    let solver = AstarSolver::with_policy(CostPolicy::strict(6));
    let result = solver.solve(&ctx);
    assert!(
        result.path.is_none(),
        "expected no path with strict 6-fuel budget over a 7-step plain trip"
    );
}

#[test]
fn fuel_constraint_allows_goal_when_budget_covers_it() {
    let map = map_from_str(8, 8, &["........"; 8]);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    // 7 plain cells * 1 fuel = 7 fuel. The strict policy with threshold=1
    // and `max_fuel == initial_fuel + 1` keeps an extra unit of fuel in
    // reserve, so 8 initial fuel covers the trip.
    let ctx = SolveContext::new(map, start, goal, 8, 50).unwrap();
    let solver = AstarSolver::with_policy(CostPolicy::strict(8));
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");
    assert_eq!(path.step_total(), 7);
}

#[test]
fn conservative_policy_still_finds_a_path() {
    // The straight run along row 4 crosses four mountains. Leaving the
    // mountain strip for row 3 costs 8 plain moves, and in official steps
    // that route departs one mountain (3) and seven plains (2 each), for a
    // total of 17 — so a 17-step budget fits exactly that route and nothing
    // cheaper. (This was written as an 8-step budget against the old
    // plain-1/mountain-2 table; the route is the same, only the unit moved.
    // `official_step_budget.rs` pins the table contract.)
    //
    // The budget is deliberately that tight: `CostPolicy::conservative`
    // inflates `g`/`f` as fuel drops, and pruning on the scaled value used
    // to reject the cheapest route (its weighted `g` passes 17 well before
    // the goal) and fall back to the greedy walk. The heap still orders on
    // `f`; only the budget check uses official steps.
    let rows = &[
        "........", "........", "........", "........", "MMMM....", "........", "........",
        "........",
    ];
    let map = map_from_str(8, 8, rows);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    // `SolveContext::new` takes (initial_fuel, max_steps).
    let ctx = SolveContext::new(map, start, goal, 24, 17).unwrap();
    // Cap 30 with only 24 in the tank: scaling starts immediately, so the
    // weighted `g` of the cheapest route passes the 17-step budget.
    let solver = AstarSolver::with_policy(CostPolicy::conservative(30));
    let result = solver.solve(&ctx);
    assert!(
        result.path.is_some(),
        "a path that fits the day budget must survive fuel-scaled ordering, got {:?}",
        result.failure
    );
}

// ---------------------------------------------------------------------------
// Case 4 — traffic-aware: busy / jammed cells carry higher step cost
// ---------------------------------------------------------------------------

#[test]
fn traffic_costs_make_solver_avoid_jammed_road_when_plain_detour_is_cheaper() {
    // Layout forces a choice between going through a 1-cell jammed patch or
    // a 2-step plain detour. The jammed cell sits at (2, 1).
    //
    //   row 0: ......  (plain)
    //   row 1: ..j...  (jammed at col 2)
    //   row 2: ......  (plain)
    //
    // Start (1, 2), goal (3, 0).
    // Direct path: NE -> (2,1) JAMMED cost 4, then NE -> (3,0) plain cost 1.
    //   Total: 5.
    // Detour via (1,1) plain: NE -> (1,1) plain, NE -> (2,0) plain, E -> (3,0)
    //   plain. Total: 3. But (1,1) is plain (row 1 col 1 is "."), so OK.
    //   Wait, we need to check the layout: "..j..." means only col 2 is
    //   jammed. So (1,1) is plain.
    let rows = &[
        "........", "..j.....", "........", "........", "........", "........", "........",
        "........",
    ];
    let map = map_from_str(8, 8, rows);
    let start = HexCoord::new(1, 2);
    let goal = HexCoord::new(3, 0);
    let ctx = SolveContext::new(map.clone(), start, goal, 200, 50).unwrap();

    let solver = AstarSolver::default();
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");
    let positions = path.positions(start);

    // Path must avoid the jammed cell at (2, 1).
    for pos in &positions {
        assert_ne!(*pos, HexCoord::new(2, 1), "touched jammed cell at {pos:?}");
    }
    assert_eq!(positions.last().copied(), Some(goal));
    // Detour cost is 3 (plain x3), jammed path cost is 5. The solver should
    // have picked the detour.
    assert!(path.step_total() <= 3, "expected detour of <= 3 steps, got {}", path.step_total());
}

#[test]
fn solver_treats_smooth_road_as_plain_for_step_count() {
    // Smooth road cells have step cost 1 — cheaper than plain (2). The
    // solver should produce a path that may include smooth-road cells
    // without inflating the step count.
    let rows = &[
        "........", "........", "........", "........", "ssssssss", "........", "........",
        "........",
    ];
    let map = map_from_str(8, 8, rows);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map.clone(), start, goal, 100, 50).unwrap();
    let solver = AstarSolver::default();
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");

    // 7 cells * step cost 1 = 7 step weight, 7 moves.
    assert_eq!(path.step_total(), 7);
    let positions = path.positions(start);
    for pos in &positions {
        let cell = map.get(*pos).unwrap();
        assert!(cell.is_passable(), "{pos:?} should be passable");
    }
}

// ---------------------------------------------------------------------------
// Sanity: solver returns a path that is internally consistent
// ---------------------------------------------------------------------------

#[test]
fn reconstructed_path_stays_in_bounds_and_avoids_ponds() {
    let rows = &[
        "........", "..~.....", "........", "...~....", "........", "........", "........",
        "........",
    ];
    let map = map_from_str(8, 8, rows);
    let start = HexCoord::new(0, 0);
    let goal = HexCoord::new(7, 7);
    let ctx = SolveContext::new(map.clone(), start, goal, 200, 100).unwrap();
    let solver = AstarSolver::default();
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");
    let positions = path.positions(start);
    assert_eq!(positions.first().copied(), Some(start));
    assert_eq!(positions.last().copied(), Some(goal));
    for pos in &positions {
        assert!(map.get(*pos).is_some(), "path escaped the map at {pos:?}");
        assert!(
            map.get(*pos).unwrap().is_passable(),
            "path stepped on an impassable cell at {pos:?}"
        );
    }
}
