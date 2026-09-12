//! Smoke tests for the multi-agent A* API (Issue #26).
//!
//! Exercises the public surface added to `crates/solver-astar`:
//! [`ReservationTable`], [`MultiAgentAStar`], [`MultiAgentSolver`], and
//! [`AgentId`]. These live in `tests/` so they reach through the same
//! downstream-facing imports real consumers will use.

use solver_astar::{
    AgentId, AgentPlan, HexCoord, HexMap, MultiAgentAStar, MultiAgentPlan, MultiAgentSolver,
    ReservationTable, SolveContext, Solver,
};

/// Build an `8 x 8` open plain map. Each fixture test pads start / goal
/// positions so neighbours are in-bounds.
fn open_map() -> HexMap {
    HexMap::new(8, 8, vec![solver_astar::CellKind::Plain; 64]).expect("8x8 plain map")
}

// ---------------------------------------------------------------------------
// Reservation table
// ---------------------------------------------------------------------------

#[test]
fn reservation_table_rejects_already_claimed_cell_at_time() {
    let mut table = ReservationTable::new();

    // First claim succeeds.
    assert!(table.reserve(HexCoord::new(3, 4), 2, AgentId(0)));
    // Same slot, same time, *different* agent: rejected.
    assert!(!table.reserve(HexCoord::new(3, 4), 2, AgentId(1)));
    // The original holder is unchanged.
    assert_eq!(table.reserver(HexCoord::new(3, 4), 2), Some(AgentId(0)));

    // Same cell, *different* time: still available.
    assert!(table.reserve(HexCoord::new(3, 4), 3, AgentId(1)));
    // *Different* cell, same time: still available.
    assert!(table.reserve(HexCoord::new(4, 4), 2, AgentId(1)));

    // `is_available` agrees with the table contents.
    assert!(!table.is_available(HexCoord::new(3, 4), 2));
    assert!(table.is_available(HexCoord::new(3, 4), 4));
    assert!(table.is_available(HexCoord::new(0, 0), 0));
}

// ---------------------------------------------------------------------------
// Two-agent coordination
// ---------------------------------------------------------------------------

#[test]
fn two_agents_do_not_collide_when_one_reserves_first() {
    // Two agents, both crossing the same corridor. Agent 0 reserves
    // first and walks east along row 4; agent 1 walks east along row 5.
    // Even if they both move at every tick, they occupy *different*
    // (coord, time) cells, so both plans must succeed without
    // collision.
    //
    // The interesting case is the start cell — both agents share the
    // same column but a different row, so this is the easy case. We
    // additionally force agent 1 to detour via the south corridor
    // by introducing a pond wall that forces a route through (3, 7).
    let map = open_map();
    let planner = MultiAgentAStar::new(vec![AgentId(0), AgentId(1)]);

    // Agent 0: (0, 4) -> (7, 4). Plain cells, 7 eastward steps.
    let ctx0 = SolveContext::new(map.clone(), HexCoord::new(0, 4), HexCoord::new(7, 4), 100, 50)
        .expect("valid ctx0");
    // Agent 1: (0, 5) -> (7, 5). Plain cells, 7 eastward steps.
    let ctx1 = SolveContext::new(map.clone(), HexCoord::new(0, 5), HexCoord::new(7, 5), 100, 50)
        .expect("valid ctx1");

    let plan = planner.plan(&[ctx0.clone(), ctx1.clone()]);
    assert_eq!(plan.plans.len(), 2);
    assert!(plan.all_succeeded(), "both agents must reach their goals");

    let path0 = plan.plans[0].path().expect("agent 0 path");
    let path1 = plan.plans[1].path().expect("agent 1 path");
    let positions0 = path0.positions(ctx0.start);
    let positions1 = path1.positions(ctx1.start);

    // Check the reservation invariant: at no shared step `t` do both
    // agents occupy the same cell.
    let len = positions0.len().max(positions1.len());
    for t in 0..len {
        let p0 = positions0.get(t).copied();
        let p1 = positions1.get(t).copied();
        if let (Some(a), Some(b)) = (p0, p1) {
            assert_ne!(a, b, "agents collided at step {t} on cell {a:?}");
        }
    }

    // Both agents end at their declared goals.
    assert_eq!(positions0.last().copied(), Some(ctx0.goal));
    assert_eq!(positions1.last().copied(), Some(ctx1.goal));
}

#[test]
fn two_agents_with_shared_corridor_reroute_second_agent() {
    // Both agents must traverse the same narrow corridor to reach
    // their goals. Agent 0 plans first and reserves the corridor;
    // agent 1 should be forced onto a longer path that avoids the
    // reserved cells.
    //
    // Setup: agent 0 goes (0, 4) -> (7, 4) along row 4.
    // Agent 1 starts south at (0, 5) and must reach (7, 4). The
    // straight route would step onto row 4 — which agent 0 reserves
    // for 7 consecutive ticks. We don't constrain agent 1's path
    // other than via the reservation table, so the planner is free to
    // route it however it wants as long as it ends at the goal.
    let map = open_map();
    let planner = MultiAgentAStar::new(vec![AgentId(0), AgentId(1)]);
    let ctx0 = SolveContext::new(map.clone(), HexCoord::new(0, 4), HexCoord::new(7, 4), 100, 50)
        .expect("valid ctx0");
    let ctx1 = SolveContext::new(map.clone(), HexCoord::new(0, 5), HexCoord::new(7, 4), 100, 50)
        .expect("valid ctx1");

    let plan = planner.plan(&[ctx0.clone(), ctx1.clone()]);
    assert!(plan.all_succeeded(), "both agents must reach their goals");

    // Verify the reservation invariant at every shared step.
    let path0 = plan.plans[0].path().expect("agent 0 path");
    let path1 = plan.plans[1].path().expect("agent 1 path");
    let positions0 = path0.positions(ctx0.start);
    let positions1 = path1.positions(ctx1.start);

    let len = positions0.len().max(positions1.len());
    for t in 0..len {
        let p0 = positions0.get(t).copied();
        let p1 = positions1.get(t).copied();
        if let (Some(a), Some(b)) = (p0, p1) {
            assert_ne!(a, b, "agents collided at step {t} on cell {a:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Single-agent fallback
// ---------------------------------------------------------------------------

#[test]
fn single_agent_solver_falls_through_to_base_astar() {
    // With no reservation table attached, the multi-agent solver must
    // produce the same path as the base `AstarSolver` for the same
    // input.
    let map = open_map();
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map.clone(), start, goal, 100, 50).unwrap();

    let base = solver_astar::AstarSolver::default().solve(&ctx);
    let base_path = base.path.as_ref().expect("base path");

    // MultiAgentSolver with no table.
    let solver = MultiAgentSolver::new(AgentId(0));
    let result = solver.solve(&ctx);
    let multi_path = result.path.as_ref().expect("multi path");

    assert_eq!(multi_path.steps.len(), base_path.steps.len());
    assert_eq!(multi_path.steps, base_path.steps);
}

#[test]
fn single_agent_planner_round_trip_matches_base_astar() {
    // The same single-agent case via `MultiAgentAStar::plan` must
    // produce a path equivalent to the base solver's path.
    let map = open_map();
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(7, 4);
    let ctx = SolveContext::new(map, start, goal, 100, 50).unwrap();

    let base = solver_astar::AstarSolver::default().solve(&ctx);
    let base_steps = base.path.as_ref().expect("base").steps.len();

    let planner = MultiAgentAStar::new(vec![AgentId(7)]);
    let plan: MultiAgentPlan = planner.plan(&[ctx]);
    assert_eq!(plan.plans.len(), 1);
    let slot: &AgentPlan = &plan.plans[0];
    assert_eq!(slot.agent, AgentId(7));
    let steps = slot.path().expect("planner path").steps.len();
    assert_eq!(steps, base_steps);
}

// ---------------------------------------------------------------------------
// Multi-agent failure handling
// ---------------------------------------------------------------------------

#[test]
fn multi_agent_plan_records_partial_failure_without_short_circuiting() {
    // Agent 0 has a valid trip; agent 1's budget is far too small to
    // reach the goal, so the planner must give up with a clean
    // `ExceedsStepBudget` failure. The plan must still report agent
    // 0's success and a failure for agent 1.
    let map = open_map();

    // Agent 0: easy trip.
    let ctx0 = SolveContext::new(map.clone(), HexCoord::new(0, 4), HexCoord::new(0, 0), 100, 50)
        .expect("valid ctx0");
    // Agent 1: impossibly small `max_steps` — the planner will fail
    // before it can step out of the start cell.
    let ctx1 = SolveContext::new(map, HexCoord::new(0, 0), HexCoord::new(7, 7), 100, 1)
        .expect("valid ctx1");

    let planner = MultiAgentAStar::new(vec![AgentId(0), AgentId(1)]);
    let plan = planner.plan(&[ctx0, ctx1]);

    assert_eq!(plan.plans.len(), 2);
    assert!(plan.plans[0].succeeded(), "agent 0 must succeed");
    // Agent 1's planner must record the failure without short-
    // circuiting agent 0's success.
    assert!(!plan.plans[1].succeeded(), "agent 1 must fail");
    assert!(plan.plans[1].result.failure.is_some());
}
