//! Integration smoke tests for the `NearestSpotGreedy` variant
//! (issue #23 / PR #73).
//!
//! Coverage:
//! 1. Single agent / single series — closest spot in that series wins.
//! 2. Multi-agent / multi-series — round-robin series assignment gives
//!    each agent a different series, and each agent picks the nearest
//!    spot in its assigned series.
//! 3. Fuel-aware variant — agents skip spots whose hex distance exceeds
//!    their remaining fuel and stay when no in-budget spot exists in the
//!    assigned series.
//!
//! These tests are structural / acceptance gates, mirroring the role of
//! `smoke.rs` for the `GreedySolver` variant.

use solver_greedy::{
    Action, Agent, AgentId, HexCoord, NearestSpotGreedy, SolveContext, Solver, Spot, SpotId,
};

fn spot(id: u16, q: i32, r: i32, series: u16, stock: u32) -> Spot {
    Spot { id: SpotId(id), coord: HexCoord::new(q, r), series_id: series, stock }
}

fn agent(id: u8, q: i32, r: i32, fuel: u32) -> Agent {
    Agent { id: AgentId(id), coord: HexCoord::new(q, r), fuel }
}

/// 1) Single agent, single series.
///
/// a0 @ (0, 0). Spots are scattered around the agent — s1 is the closest
/// series-0 spot, the others are farther. Even though s4 happens to be
/// physically closer, it is in series 1 and the agent is implicitly
/// assigned to series 0 (the only series present). The solver must
/// therefore pick s1.
#[test]
fn single_agent_single_series_picks_nearest_in_series() {
    let agents = vec![agent(0, 0, 0, 10)];
    let spots = vec![
        spot(1, 1, 0, 0, 1), // s1 — closest series-0 spot, distance 1
        spot(2, 5, 0, 0, 1), // s2 — series 0, far
        spot(3, 3, 1, 0, 1), // s3 — series 0, medium
        spot(4, 0, 1, 1, 1), // s4 — series 1, distance 1 (different series)
    ];
    let ctx = SolveContext::for_test(0, agents.clone(), spots.clone());

    let actions = NearestSpotGreedy::new().decide(&ctx);
    assert_eq!(actions.len(), 1, "one action per agent");
    assert_eq!(actions[0].agent(), AgentId(0));
    assert_eq!(
        actions[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(1) },
        "expected nearest series-0 spot (s1)"
    );

    // Every chosen spot, if any, must be available.
    for action in &actions {
        if let Action::MoveToward { spot, .. } = action {
            let s = spots.iter().find(|s| s.id == *spot).expect("known spot");
            assert!(s.is_available(), "must not move toward an empty spot");
        }
    }
}

/// 2) Multi-agent / multi-series — round-robin assignment.
///
/// 3 agents, 3 series, each series has a near and a far spot for its
/// assigned agent. Round-robin assignment (sorted `series_ids` ascending)
/// gives:
///
/// - a0 -> series 0 (smallest `series_id`)
/// - a1 -> series 1
/// - a2 -> series 2
///
/// Each agent must pick the nearest available spot *in its assigned
/// series*, ignoring closer spots belonging to other series.
#[test]
fn multi_agent_multi_series_round_robin_assignment() {
    let agents = vec![
        agent(0, 0, 0, 10),   // a0 -> series 0
        agent(1, 10, 0, 10),  // a1 -> series 1
        agent(2, -10, 0, 10), // a2 -> series 2
    ];
    // For each agent, place one spot of *every* series very close to it
    // (a temptation) and one spot of the *assigned* series farther away.
    // The agent must ignore the temptation and pick the assigned-series
    // spot.
    let spots = vec![
        // a0 at (0, 0): tempt with series 1 / 2 spots close by.
        spot(10, 1, 0, 1, 1), // close to a0 but series 1
        spot(11, 0, 1, 2, 1), // close to a0 but series 2
        spot(0, 5, 0, 0, 1),  // series 0, far from a0 (must pick)
        // a1 at (10, 0): tempt with series 0 / 2 spots close by.
        spot(20, 9, 0, 0, 1),  // close to a1 but series 0
        spot(21, 10, 1, 2, 1), // close to a1 but series 2
        spot(1, 14, 0, 1, 1),  // series 1, far from a1 (must pick)
        // a2 at (-10, 0): tempt with series 0 / 1 spots close by.
        spot(30, -9, 0, 0, 1),  // close to a2 but series 0
        spot(31, -10, 1, 1, 1), // close to a2 but series 1
        spot(2, -14, 0, 2, 1),  // series 2, far from a2 (must pick)
    ];
    let ctx = SolveContext::for_test(0, agents.clone(), spots.clone());

    let actions = NearestSpotGreedy::new().decide(&ctx);
    assert_eq!(actions.len(), 3, "one action per agent");

    // Structural invariants: one action per agent, valid targets only.
    for (idx, action) in actions.iter().enumerate() {
        assert_eq!(action.agent(), agents[idx].id, "agent order preserved");
        if let Action::MoveToward { spot, .. } = action {
            let s = spots.iter().find(|s| s.id == *spot).expect("known spot");
            assert!(s.is_available(), "must not move toward an empty spot");
        }
    }

    // Each agent must pick the spot in its assigned series (the far one).
    assert_eq!(
        actions[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(0) },
        "a0 -> series 0 -> s0"
    );
    assert_eq!(
        actions[1],
        Action::MoveToward { agent: AgentId(1), spot: SpotId(1) },
        "a1 -> series 1 -> s1"
    );
    assert_eq!(
        actions[2],
        Action::MoveToward { agent: AgentId(2), spot: SpotId(2) },
        "a2 -> series 2 -> s2"
    );
}

/// Determinism guard for the multi-series case: two consecutive calls must
/// return byte-identical action vectors (replay + bench reproducibility).
#[test]
fn multi_series_decide_is_deterministic() {
    let ctx = SolveContext::for_test(
        12,
        vec![agent(0, 0, 0, 10), agent(1, 5, 0, 10), agent(2, -5, 0, 10)],
        vec![
            spot(0, 1, 0, 0, 1),
            spot(1, 6, 0, 1, 1),
            spot(2, -6, 0, 2, 1),
            spot(3, 2, 0, 0, 1),
            spot(4, 7, 0, 1, 1),
            spot(5, -7, 0, 2, 1),
        ],
    );
    let solver = NearestSpotGreedy::new();
    let a1 = solver.decide(&ctx);
    let a2 = solver.decide(&ctx);
    assert_eq!(a1, a2);
}

/// 3) Fuel-aware variant.
///
/// a0 has only 2 fuel. Its assigned series is 0. The closest series-0
/// spot is at distance 1 (s0) — within budget. Without the fuel-aware
/// filter the solver would happily pick s2 (distance 1, series 0) too,
/// but the variant under test must skip spots whose distance exceeds
/// the remaining fuel when no spot is in range.
///
/// Concretely: the *basic* variant would still pick s2 (distance 1) and
/// pass — so to actually exercise the fuel filter we set up a world where
/// the only series-0 spot is out of budget. a0's assigned series is 0
/// (only series present), and the only available series-0 spot is at
/// distance 5 (> fuel = 2). The fuel-aware solver must emit `Stay`.
#[test]
fn fuel_aware_skips_out_of_budget_spots_in_assigned_series() {
    // a0 has fuel = 2. Only series 0 is present. The single available
    // spot is at distance 5, well out of budget.
    let agents = vec![agent(0, 0, 0, 2)];
    let spots = vec![
        spot(0, 5, 0, 0, 1), // series 0, distance 5, available
        spot(1, 0, 1, 1, 1), // series 1 — ignored (not assigned series)
    ];
    let ctx = SolveContext::for_test(0, agents.clone(), spots.clone());

    // Basic variant ignores fuel — picks the (out-of-budget) spot.
    let basic = NearestSpotGreedy::new().decide(&ctx);
    assert_eq!(
        basic[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(0) },
        "basic variant ignores fuel"
    );

    // Fuel-aware variant must skip the out-of-budget spot and Stay.
    let aware = NearestSpotGreedy::fuel_aware().decide(&ctx);
    assert!(
        matches!(aware[0], Action::Stay { .. }),
        "fuel-aware variant must Stay when no in-budget spot in assigned series, got {:?}",
        aware[0]
    );
}

/// Fuel-aware variant: when at least one spot *is* in budget, the agent
/// must still pick the nearest *in-budget* spot in its assigned series.
#[test]
fn fuel_aware_picks_nearest_in_budget_spot() {
    // a0 has fuel = 3, only series 0. s1 (distance 1) and s2 (distance 5)
    // are both series 0. Without the filter s2 would not win anyway (it's
    // farther), so to actually exercise the filter we add an out-of-budget
    // *closer* alternative in a different series — the basic variant
    // would be tempted by series 1's close spot, but fuel-aware must
    // still pick the nearest *series-0 in-budget* spot.
    let agents = vec![agent(0, 0, 0, 3)];
    let spots = vec![
        spot(1, 0, 1, 0, 1), // series 0, distance 1 (assigned, in budget)
        spot(2, 4, 0, 0, 1), // series 0, distance 4 — assigned but out of budget
        spot(3, 1, 0, 1, 1), // series 1, distance 1 — different series
    ];
    let ctx = SolveContext::for_test(0, agents.clone(), spots.clone());

    let aware = NearestSpotGreedy::fuel_aware().decide(&ctx);
    assert_eq!(
        aware[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(1) },
        "fuel-aware must pick nearest in-budget spot in assigned series"
    );
}
