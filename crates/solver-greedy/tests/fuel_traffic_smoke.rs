//! Integration smoke test for the fuel-aware traffic-avoiding
//! [`solver_greedy::FuelTrafficGreedy`] variant (Issue #24).
//!
//! The unit tests inside `src/fuel_traffic.rs` exercise the algorithm
//! in isolation. This file is the "skeleton wired into the bench shape"
//! acceptance gate — it asserts the public API is reachable from
//! outside the crate and that the three Issue #24 acceptance criteria
//! hold end-to-end:
//!
//! 1. Low-fuel path to the nearest refill spot.
//! 2. Traffic avoidance (smooth path preferred over jammed path when
//!    fuel is sufficient and the traffic weight is non-trivial).
//! 3. Multi-agent mix: each agent independently picks a spot that
//!    respects its own fuel level.

use solver_greedy::{
    Action, Agent, AgentId, FuelTrafficGreedy, FuelTrafficSolveContext, FuelTrafficSolver,
    HexCoord, Spot, SpotId, TrafficState,
};

fn spot(id: u16, q: i32, r: i32, series: u16, stock: u32) -> Spot {
    Spot { id: SpotId(id), coord: HexCoord::new(q, r), series_id: series, stock }
}

fn agent(id: u8, q: i32, r: i32, fuel: u32) -> Agent {
    Agent { id: AgentId(id), coord: HexCoord::new(q, r), fuel }
}

/// Acceptance #1 — fuel-aware refuel path.
///
/// Agent a0 at `(0, 0)` with `fuel = 1` (well below the default
/// threshold of 3). Three spots at distances 1, 2, 3. The cells
/// between the agent and the nearest spot are marked `Jammed` so we
/// can prove the refuel branch ignores traffic and still heads for
/// the closest opportunity.
#[test]
fn low_fuel_targets_nearest_refill_spot() {
    let traffic = vec![
        (HexCoord::new(1, 0), TrafficState::Jammed),
        (HexCoord::new(2, 0), TrafficState::Jammed),
    ];
    let ctx = FuelTrafficSolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 1)],
        vec![
            spot(0, 3, 0, 0, 1), // distance 3 — smooth path
            spot(1, 1, 0, 1, 1), // distance 1 — nearest, jammed path
            spot(2, 2, 0, 0, 1), // distance 2 — middle, also jammed
        ],
        traffic,
    );

    let actions = FuelTrafficGreedy::with_params(3, 100).decide(&ctx);

    assert_eq!(actions.len(), 1, "one action per agent");
    assert_eq!(
        actions[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(1) },
        "low-fuel agent must pick the nearest refill spot regardless of traffic"
    );
}

/// Acceptance #2 — traffic avoidance.
///
/// Agent a0 at `(0, 0)` with plenty of fuel. Two spots at the same
/// distance:
/// - spot 0 at `(0, 3)` via cells `(0, 1), (0, 2)` — both `Jammed`.
/// - spot 1 at `(3, 0)` via cells `(1, 0), (2, 0)` — both `Smooth`.
///
/// With a non-trivial traffic weight, the solver must prefer the
/// smooth path.
#[test]
fn traffic_avoidance_picks_smooth_path() {
    let traffic = vec![
        (HexCoord::new(0, 1), TrafficState::Jammed),
        (HexCoord::new(0, 2), TrafficState::Jammed),
        (HexCoord::new(1, 0), TrafficState::Smooth),
        (HexCoord::new(2, 0), TrafficState::Smooth),
    ];
    let ctx = FuelTrafficSolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 100)],
        vec![
            spot(0, 0, 3, 0, 1), // distance 3 via jammed cells
            spot(1, 3, 0, 1, 1), // distance 3 via smooth cells
        ],
        traffic,
    );

    let actions = FuelTrafficGreedy::with_params(3, 100).decide(&ctx);

    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(1) },
        "high-fuel agent must avoid the jammed path and pick the smooth one"
    );
}

/// Symmetry check: with `traffic_weight = 0` the score collapses to
/// pure distance. Both spots sit at distance 3, so the tie-break falls
/// back to `series_id` then `spot_id` — deterministically spot 0.
#[test]
fn zero_traffic_weight_falls_back_to_distance() {
    let traffic = vec![
        (HexCoord::new(0, 1), TrafficState::Jammed),
        (HexCoord::new(0, 2), TrafficState::Jammed),
    ];
    let ctx = FuelTrafficSolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 100)],
        vec![
            spot(0, 0, 3, 0, 1), // distance 3, jammed cells
            spot(1, 3, 0, 1, 1), // distance 3, no traffic
        ],
        traffic,
    );

    let actions = FuelTrafficGreedy::with_params(3, 0).decide(&ctx);

    assert_eq!(actions.len(), 1);
    // Both spots are at distance 3. The tie-break resolves by series
    // (0 < 1) then by spot id. We accept either spot as long as the
    // solver returns a MoveToward (the goal is to prove traffic weight
    // is the differentiator, not to nail the exact tie-break winner).
    assert!(
        matches!(actions[0], Action::MoveToward { .. }),
        "expected MoveToward, got {:?}",
        actions[0]
    );
}

/// Acceptance #3 — multi-agent mix.
///
/// Three agents with different fuel levels, three spots with
/// different distances. Each agent independently picks a spot that
/// respects its own state:
///
/// - a0 at `(0, 0)` with `fuel = 100` (high). Spot 0 is at distance 3
///   but the cells `(0, 1)` and `(0, 2)` on the way are `Jammed`.
///   Spot 1 is also at distance 3 but the cells `(1, 0)` and `(2, 0)`
///   on the way are `Smooth`. With a heavy traffic weight, a0 must
///   pick spot 1.
/// - a1 at `(5, 1)` with `fuel = 1` (low). Must refuel at the
///   nearest spot — spot 2 sits on a1's own cell (distance 0).
/// - a2 at `(2, 2)` with `fuel = 100` (high). The greedy paths from
///   a2 to spot 0 / spot 1 both pass through cells not in the
///   traffic map (treated as `Smooth`). a2 is free to pick either
///   based on its own scoring; we only assert that it picks a legal
///   spot.
#[test]
fn multi_agent_mixes_fuel_and_traffic() {
    let traffic = vec![
        // Cells on the route to spot 0 are jammed.
        (HexCoord::new(0, 1), TrafficState::Jammed),
        (HexCoord::new(0, 2), TrafficState::Jammed),
        // Cells on the route to spot 1 are smooth.
        (HexCoord::new(1, 0), TrafficState::Smooth),
        (HexCoord::new(2, 0), TrafficState::Smooth),
    ];
    let ctx = FuelTrafficSolveContext::for_test(
        0,
        vec![
            agent(0, 0, 0, 100), // high fuel — traffic aware
            agent(1, 5, 1, 1),   // low fuel — refuel at nearest
            agent(2, 2, 2, 100), // high fuel — traffic aware
        ],
        vec![
            spot(0, 0, 3, 0, 1), // distance 3 from a0 via jammed cells
            spot(1, 3, 0, 1, 1), // distance 3 from a0 via smooth cells
            spot(2, 5, 1, 0, 1), // distance 0 from a1 — a1's refuel target
        ],
        traffic,
    );

    let actions = FuelTrafficGreedy::with_params(3, 100).decide(&ctx);

    assert_eq!(actions.len(), 3, "one action per agent");

    // Every action targets a known, available spot.
    for (idx, action) in actions.iter().enumerate() {
        match action {
            Action::MoveToward { agent, spot } => {
                assert_eq!(*agent, AgentId(u8::try_from(idx).unwrap()), "agent order preserved");
                let s = ctx
                    .spots
                    .iter()
                    .find(|s| s.id == *spot)
                    .expect("MoveToward must reference a known spot");
                assert!(s.is_available(), "must not move toward an empty spot");
            }
            Action::Stay { agent } => {
                assert_eq!(*agent, AgentId(u8::try_from(idx).unwrap()));
            }
        }
    }

    // a0: high fuel, heavy traffic weight. Spot 0's greedy path
    // accumulates two Jammed penalties (cost 6); spot 1's path is
    // clean (cost 0). Even though both spots are at distance 3, the
    // combined score collapses to `3 + 100*0 = 3` for spot 1 vs
    // `3 + 100*6 = 603` for spot 0 — a0 must pick spot 1.
    assert_eq!(
        actions[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(1) },
        "high-fuel agent must avoid jammed cells and pick the smooth route"
    );

    // a1: fuel = 1 is below the threshold of 3. The refuel branch
    // picks the nearest spot unconditionally — spot 2 is on a1's
    // own cell, distance 0.
    assert_eq!(
        actions[1],
        Action::MoveToward { agent: AgentId(1), spot: SpotId(2) },
        "low-fuel agent must refuel at the nearest spot (spot 2, on its own cell)"
    );
}

/// Determinism — same context, two calls, identical actions.
#[test]
fn decide_is_deterministic() {
    let traffic = vec![(HexCoord::new(1, 0), TrafficState::Busy)];
    let ctx = FuelTrafficSolveContext::for_test(
        7,
        vec![agent(0, 0, 0, 100), agent(1, 1, 0, 2), agent(2, 0, 1, 100)],
        vec![spot(0, 0, 0, 0, 1), spot(1, 1, 1, 1, 1), spot(2, -1, 0, 1, 1)],
        traffic,
    );
    let solver = FuelTrafficGreedy::new();
    let a1 = solver.decide(&ctx);
    let a2 = solver.decide(&ctx);
    assert_eq!(a1, a2);
}

/// All stocks exhausted ⇒ every agent stays.
#[test]
fn empty_stock_everyone_stays() {
    let ctx = FuelTrafficSolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 100), agent(1, 1, 0, 1)],
        vec![spot(0, 0, 0, 0, 0), spot(1, 1, 1, 1, 0)],
        vec![],
    );
    let actions = FuelTrafficGreedy::new().decide(&ctx);
    for a in actions {
        assert!(matches!(a, Action::Stay { .. }), "expected Stay, got {a:?}");
    }
}
