//! Smoke tests for the time-expanded A* search (Issue #27).
//!
//! Verifies three invariants:
//!
//! 1. Search finds the shortest path in time on a plain open map.
//! 2. The fuel-aware variant rejects paths whose fuel cost exceeds
//!    `initial_fuel`.
//! 3. Search terminates within `max_steps` even when the goal is far.

use solver_astar::{
    CellKind, HexCoord, HexMap, SolveContext, Solver, TimeExpandedAStar, TimeExpandedNode,
    TimeExpandedSolver,
};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Build a `width x height` plain-cell map. The fixture is simple enough
/// that the only interesting decisions are start / goal / budgets.
fn plain_map(width: u32, height: u32) -> HexMap {
    let cells = vec![CellKind::Plain; (width as usize) * (height as usize)];
    HexMap::new(width, height, cells).expect("plain map validates")
}

// ---------------------------------------------------------------------------
// Case 1 — shortest path in time
// ---------------------------------------------------------------------------

#[test]
fn finds_shortest_path_in_time_on_open_map() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map.clone(), start, goal, 100, 50).unwrap();

    let solver = TimeExpandedSolver::new(50, false);
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");

    // Minimum hex distance from (0,4) to (7,4) is 7 eastward moves.
    assert_eq!(path.steps.len(), 7, "expected 7-step east-only path");
    assert_eq!(path.step_total(), 7);
    let positions = path.positions(start);
    assert_eq!(positions.first().copied(), Some(start));
    assert_eq!(positions.last().copied(), Some(goal));

    // Path must stay inside the map and on passable cells.
    for pos in &positions {
        assert!(map.get(*pos).is_some(), "{pos:?} escaped the map");
        assert!(map.get(*pos).unwrap().is_passable(), "{pos:?} is impassable");
    }
}

#[test]
fn raw_search_returns_optimal_node_chain() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map, start, goal, 100, 50).unwrap();

    let config = TimeExpandedAStar::new(50, false);
    let result = config.search(&ctx);
    let chain = result.reconstruct_path().expect("chain");

    // Root + 7 steps = 8 nodes.
    assert_eq!(chain.len(), 8, "expected 8 nodes (root + 7 steps)");
    assert_eq!(chain.first().copied(), Some((start, 0)));
    let goal_step = u32::try_from(chain.len() - 1).unwrap();
    assert_eq!(chain.last().copied(), Some((goal, goal_step)));
}

#[test]
fn node_helpers_match_basic_invariants() {
    let root = TimeExpandedNode::root(HexCoord::new(2, 3), 5.0);
    assert_eq!(root.step, 0);
    assert!(root.g_cost.abs() < f64::EPSILON);
    assert!(root.parent.is_none());
    assert!((root.f_cost() - 5.0).abs() < f64::EPSILON);

    let child = TimeExpandedNode {
        position: HexCoord::new(3, 3),
        step: 1,
        g_cost: 1.0,
        h_cost: 4.0,
        parent: Some((HexCoord::new(2, 3), 0)),
    };
    assert_eq!(child.parent, Some((HexCoord::new(2, 3), 0)));
    assert!((child.f_cost() - 5.0).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// Case 2 — fuel-aware variant rejects over-budget paths
// ---------------------------------------------------------------------------

#[test]
fn fuel_aware_variant_rejects_paths_exceeding_initial_fuel() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    // 6 plain cells * 1 fuel = 6 fuel. Budget is 5 — unreachable.
    let ctx = SolveContext::new(map, start, goal, 5, 50).unwrap();

    let solver = TimeExpandedSolver::new(50, true);
    let result = solver.solve(&ctx);
    assert!(
        result.path.is_none(),
        "fuel-aware solver should reject a 7-step trip with only 5 fuel"
    );
    assert!(result.failure.is_some(), "failure reason should be reported");
}

#[test]
fn fuel_aware_variant_accepts_within_budget() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    // 7 plain cells * 1 fuel = 7 fuel. Budget is exactly 7.
    let ctx = SolveContext::new(map.clone(), start, goal, 7, 50).unwrap();

    let solver = TimeExpandedSolver::new(50, true);
    let result = solver.solve(&ctx);
    let path = result.path.expect("path");

    // Same 7-step east path, but every move had to fit inside the fuel
    // counter, so the solver must produce an equivalently long route.
    assert_eq!(path.steps.len(), 7);
    let positions = path.positions(start);
    assert_eq!(positions.last().copied(), Some(goal));
}

// ---------------------------------------------------------------------------
// Case 3 — terminates within max_steps
// ---------------------------------------------------------------------------

#[test]
fn search_terminates_when_max_steps_below_min_distance() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    // Distance is 7 steps; allow only 6.
    let ctx = SolveContext::new(map, start, goal, 100, 6).unwrap();

    let solver = TimeExpandedSolver::new(6, false);
    let result = solver.solve(&ctx);
    assert!(result.path.is_none(), "max_steps=6 cannot cover a 7-step trip");

    // The result must be deterministic: every entry on the open set at
    // termination has `step <= max_steps`.
    let config = TimeExpandedAStar::new(6, false);
    let raw = config.search(&ctx);
    assert!(raw.goal_state.is_none());
    // expanded is bounded by the (step × cell) frontier, which is at most
    // (max_steps + 1) * cells = 7 * 64 = 448. Anything wildly outside that
    // would signal a runaway loop.
    assert!(raw.expanded <= 448, "expansion runaway: {}", raw.expanded);
}

#[test]
fn search_terminates_within_max_steps_on_completed_run() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map, start, goal, 100, 7).unwrap();

    // max_steps is exactly the minimum distance.
    let config = TimeExpandedAStar::new(7, false);
    let raw = config.search(&ctx);
    let chain = raw.reconstruct_path().expect("chain");
    assert_eq!(chain.last().copied(), Some((goal, 7)));
    // Every step in the chain is monotonically non-decreasing.
    for window in chain.windows(2) {
        assert!(window[1].1 >= window[0].1, "step regressed at {window:?}");
    }
}
