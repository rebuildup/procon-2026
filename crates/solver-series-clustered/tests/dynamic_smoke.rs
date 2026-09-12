//! Smoke / integration tests for `DynamicReplanner` (Issue #30).
//!
//! Acceptance criteria: the planner must
//! 1. reuse the cached plan when both inventory and traffic stay inside the
//!    configured thresholds,
//! 2. trigger a replan when the spot inventory drops beyond the inventory
//!    threshold,
//! 3. trigger a replan when traffic surges beyond the traffic threshold,
//! 4. honour `min_replan_interval_turns` even when the diffs are large.

use solver_series_clustered::{
    Agent, AgentId, DynamicReplanner, HexCoord, InventorySnapshot, PlanState, Series,
    SeriesClusteredSolver, SolveRequest, Solver, Spot, SpotId, TrafficSnapshot,
};

fn spot(id: &str, q: i32, r: i32, series: &str) -> Spot {
    Spot {
        id: SpotId::new(id),
        coord: HexCoord::new(q, r),
        series: Series::new(series),
        initial_stock: 5,
    }
}

fn agent(id: &str, q: i32, r: i32) -> Agent {
    Agent { id: AgentId::new(id), coord: HexCoord::new(q, r), patrol_radius: 8 }
}

fn two_agent_request() -> SolveRequest {
    SolveRequest {
        spots: vec![spot("a1", 0, 0, "udon"), spot("a2", 1, 0, "udon"), spot("b1", 10, 10, "soba")],
        agents: vec![agent("p1", 0, 0), agent("p2", 10, 10)],
    }
}

#[test]
fn no_replan_when_within_thresholds() {
    let replanner = DynamicReplanner::new(
        SeriesClusteredSolver::with_default_config(),
        0.5, // inventory threshold (50%)
        0.5, // traffic threshold (50%)
        1,
    );

    let initial_inventory = InventorySnapshot::from_pairs([
        (SpotId::new("a1"), 5),
        (SpotId::new("a2"), 5),
        (SpotId::new("b1"), 5),
    ]);
    let initial_traffic = TrafficSnapshot::from_pairs([
        ((HexCoord::new(0, 0), HexCoord::new(1, 0)), 2),
        ((HexCoord::new(10, 10), HexCoord::new(11, 10)), 2),
    ]);

    let req = two_agent_request();
    let first = replanner.inner().solve(req.clone());
    let mut state =
        PlanState::new(first.clone(), initial_inventory.clone(), initial_traffic.clone(), 1);

    // Tiny drift: 1 ball consumed out of 15 total = ~6.7%, well under 50%.
    let next_inventory = InventorySnapshot::from_pairs([
        (SpotId::new("a1"), 5),
        (SpotId::new("a2"), 4),
        (SpotId::new("b1"), 5),
    ]);
    let next_traffic = initial_traffic.clone();

    assert!(!replanner.should_replan(&state, 2, &next_inventory, &next_traffic));

    let returned = replanner.solve_with_state(&mut state, req, 2, &next_inventory, &next_traffic);
    // Plan is reused: returned value should equal `state.last_plan`.
    assert_eq!(returned, state.last_plan);
    // State must not have advanced (still turn 1).
    assert_eq!(state.last_replan_turn, 1);
}

#[test]
fn replan_when_inventory_drops_beyond_threshold() {
    let replanner = DynamicReplanner::new(
        SeriesClusteredSolver::with_default_config(),
        0.1, // inventory threshold (10%)
        1.0, // traffic threshold (impossibly large)
        1,
    );

    let initial_inventory = InventorySnapshot::from_pairs([
        (SpotId::new("a1"), 10),
        (SpotId::new("a2"), 10),
        (SpotId::new("b1"), 10),
    ]);
    let initial_traffic = TrafficSnapshot::default();
    let req = two_agent_request();

    let first = replanner.inner().solve(req.clone());
    let mut state = PlanState::new(first, initial_inventory, initial_traffic, 1);

    // Lose half the stock on `a1` → ~16.7% drift, above 10%.
    let next_inventory = InventorySnapshot::from_pairs([
        (SpotId::new("a1"), 5),
        (SpotId::new("a2"), 10),
        (SpotId::new("b1"), 10),
    ]);
    let next_traffic = TrafficSnapshot::default();

    assert!(replanner.should_replan(&state, 2, &next_inventory, &next_traffic));

    let pre_replan_turn = state.last_replan_turn;
    let _ = replanner.solve_with_state(&mut state, req, 2, &next_inventory, &next_traffic);
    // State must have advanced to turn 2.
    assert_eq!(state.last_replan_turn, 2);
    assert_ne!(state.last_replan_turn, pre_replan_turn);
    // And the new inventory snapshot is recorded.
    assert_eq!(state.last_inventory.stock_of(&SpotId::new("a1")), 5);
}

#[test]
fn replan_when_traffic_surges_beyond_threshold() {
    let replanner = DynamicReplanner::new(
        SeriesClusteredSolver::with_default_config(),
        1.0, // inventory threshold (impossibly large)
        0.1, // traffic threshold (10%)
        1,
    );

    let initial_traffic = TrafficSnapshot::from_pairs([
        ((HexCoord::new(0, 0), HexCoord::new(1, 0)), 2),
        ((HexCoord::new(1, 0), HexCoord::new(2, 0)), 2),
        ((HexCoord::new(10, 10), HexCoord::new(11, 10)), 2),
        ((HexCoord::new(11, 10), HexCoord::new(12, 10)), 2),
    ]);
    let initial_inventory = InventorySnapshot::default();
    let req = two_agent_request();

    let first = replanner.inner().solve(req.clone());
    let mut state = PlanState::new(first, initial_inventory, initial_traffic, 5);

    // All four edges jump from 2 → 4 (50% drift across the surface).
    let next_traffic = TrafficSnapshot::from_pairs([
        ((HexCoord::new(0, 0), HexCoord::new(1, 0)), 4),
        ((HexCoord::new(1, 0), HexCoord::new(2, 0)), 4),
        ((HexCoord::new(10, 10), HexCoord::new(11, 10)), 4),
        ((HexCoord::new(11, 10), HexCoord::new(12, 10)), 4),
    ]);
    let next_inventory = InventorySnapshot::default();

    assert!(replanner.should_replan(&state, 6, &next_inventory, &next_traffic));

    let _ = replanner.solve_with_state(&mut state, req, 6, &next_inventory, &next_traffic);
    assert_eq!(state.last_replan_turn, 6);
    // And the new traffic snapshot is recorded.
    assert_eq!(state.last_traffic.congestion_of(&(HexCoord::new(0, 0), HexCoord::new(1, 0))), 4);
}

#[test]
fn min_replan_interval_is_respected() {
    let replanner = DynamicReplanner::new(
        SeriesClusteredSolver::with_default_config(),
        0.05, // very tight inventory threshold
        0.05, // very tight traffic threshold
        3,    // but only replan every 3 turns
    );

    let initial_inventory = InventorySnapshot::from_pairs([
        (SpotId::new("a1"), 100),
        (SpotId::new("a2"), 100),
        (SpotId::new("b1"), 100),
    ]);
    let initial_traffic =
        TrafficSnapshot::from_pairs([((HexCoord::new(0, 0), HexCoord::new(1, 0)), 10)]);
    let req = two_agent_request();

    let first = replanner.inner().solve(req.clone());
    let mut state = PlanState::new(first, initial_inventory, initial_traffic, 10);

    // Same turn: even with a 50% inventory cliff, the interval gates the call.
    let next_inventory = InventorySnapshot::from_pairs([
        (SpotId::new("a1"), 50),
        (SpotId::new("a2"), 100),
        (SpotId::new("b1"), 100),
    ]);
    let next_traffic = state.last_traffic.clone();
    assert!(!replanner.should_replan(&state, 11, &next_inventory, &next_traffic));

    // Turn 12 (interval = 2, still under the 3-turn cap).
    assert!(!replanner.should_replan(&state, 12, &next_inventory, &next_traffic));

    // Turn 13 (interval = 3, exactly on the cap) — replan is now permitted.
    assert!(replanner.should_replan(&state, 13, &next_inventory, &next_traffic));

    // Apply the replan and confirm `last_replan_turn` advances.
    let _ = replanner.solve_with_state(&mut state, req, 13, &next_inventory, &next_traffic);
    assert_eq!(state.last_replan_turn, 13);
}

#[test]
fn default_config_replanner_is_constructible() {
    // Smoke check: the convenience constructor wires sensible defaults.
    let replanner = DynamicReplanner::with_default_config();
    let _ = replanner.inner();
}

#[test]
fn solver_trait_impl_delegates_to_inner() {
    // The blanket `Solver` impl must work so callers can pass the
    // replanner wherever `&dyn Solver` is expected.
    let replanner = DynamicReplanner::with_default_config();
    let req = two_agent_request();
    let direct = SeriesClusteredSolver::with_default_config().solve(req.clone());
    let via_replanner = <DynamicReplanner as Solver>::solve(&replanner, req);
    assert_eq!(direct, via_replanner);
}
