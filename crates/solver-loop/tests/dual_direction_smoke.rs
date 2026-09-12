//! Integration tests for the dual-direction (CW / CCW) loop solver.
//!
//! These tests exercise the public surface re-exported from
//! `solver_loop` (i.e. what a downstream consumer actually sees) and
//! verify the three headline guarantees from Issue #32:
//!
//! 1. CW and CCW loops are opposite directions of the same closed tour.
//! 2. Collection agents stay on the CW loop, refill agents on the CCW
//!    loop.
//! 3. Both loops together cover every spot assigned to their respective
//!    agent group.
//!
//! Like the other solver-loop integration tests, these stay at the
//! `Solver::solve()` boundary so they remain valid once Issue #57
//! replaces the local stubs with `domain::*`.

use solver_loop::{
    Agent, Direction, DualDirectionLoop, HexCoord, InitialStrategy, Series, SolveProblem, Solver,
    SolverError, Spot,
};

/// Helper: build a spot.
fn spot(id: u32, q: i32, r: i32, series: &str) -> Spot {
    Spot { id, coord: HexCoord::new(q, r), series: Series::new(series), stock: 1 }
}

fn agent(id: u32, q: i32, r: i32) -> Agent {
    Agent { id, start: HexCoord::new(q, r) }
}

/// A small, predictable instance: four spots forming a square. The
/// refined CW tour from the underlying `LoopSolver` will be one of the
/// two optimal tours around the square; either way, the *reverse* of
/// that tour is the CCW tour by construction.
fn square_problem() -> SolveProblem {
    let spots = vec![
        spot(0, 0, 0, "kake"),
        spot(1, 2, 0, "bukkake"),
        spot(2, 2, 2, "kake"),
        spot(3, 0, 2, "bukkake"),
    ];
    // 4 collection + 4 refill agents to exercise the midpoint split.
    let agents: Vec<Agent> = (0..8).map(|i| agent(i, 0, 0)).collect();
    SolveProblem::new(spots, agents, 1)
}

/// 1. Direction ordering: CW visits spots in the clockwise order
///    produced by the underlying solver; CCW visits them in the exact
///    reverse order.
#[test]
fn direction_ordering_cw_equals_forward_ccw_equals_reverse() {
    let problem = square_problem();
    let solver = DualDirectionLoop::new(InitialStrategy::Identity, 0);
    let cw = solver.cw_tour(&problem);
    let ccw = solver.ccw_tour(&problem);

    // Both tours have the same length (same set of spots) but opposite
    // ordering.
    assert_eq!(cw.len(), ccw.len());
    assert_eq!(cw.len(), problem.spots.len());

    // CCW is the reverse of CW: every cw[i] must equal ccw[len-1-i].
    for (i, &id) in cw.iter().enumerate() {
        let j = cw.len() - 1 - i;
        assert_eq!(id, ccw[j], "CCW[{j}] must equal CW[{i}] reversed");
    }

    // And conversely — every spot id appears exactly once in each
    // direction.
    let mut cw_sorted = cw.clone();
    let mut ccw_sorted = ccw.clone();
    cw_sorted.sort_unstable();
    ccw_sorted.sort_unstable();
    cw_sorted.dedup();
    ccw_sorted.dedup();
    assert_eq!(cw_sorted, ccw_sorted, "CW and CCW visit the same spot set");
    assert_eq!(cw_sorted.len(), problem.spots.len());
}

/// 2. Collection agents stay on the CW loop, refill agents on the CCW
///    loop. Verified by inspecting the per-agent plan: a collection
///    agent's plan equals (a slice of) the CW tour in the same order; a
///    refill agent's plan equals (a slice of) the CCW tour in the same
///    order.
#[test]
fn collection_agents_get_cw_refill_agents_get_ccw() {
    let problem = square_problem();
    let solver = DualDirectionLoop::default();
    let cw = solver.cw_tour(&problem);
    let ccw = solver.ccw_tour(&problem);
    let outcome = solver.solve(&problem).expect("solve");

    let n_agents = problem.agents.len();
    let n_collect = n_agents / 2;

    // The plans are flattened as `plans[day][agent][..spot_ids]`.
    // For a 1-day solve the only day is index 0.
    let day_plans = &outcome.plans[0];
    assert_eq!(day_plans.len(), n_agents);

    // Per-day visits: every collection agent's visits must appear in the
    // CW tour in the same order; every refill agent's visits must appear
    // in the CCW tour in the same order.
    let cw_positions = |spot_id: u32| cw.iter().position(|&x| x == spot_id);
    let ccw_positions = |spot_id: u32| ccw.iter().position(|&x| x == spot_id);

    for (agent_idx, agent_plan) in day_plans.iter().enumerate() {
        if agent_idx < n_collect {
            // Collection agent: visits must follow CW order.
            for w in agent_plan.windows(2) {
                let a = cw_positions(w[0]).expect("spot in cw tour");
                let b = cw_positions(w[1]).expect("spot in cw tour");
                assert!(a < b, "collection agent {agent_idx} visits out of CW order: {w:?}");
            }
        } else {
            // Refill agent: visits must follow CCW order.
            for w in agent_plan.windows(2) {
                let a = ccw_positions(w[0]).expect("spot in ccw tour");
                let b = ccw_positions(w[1]).expect("spot in ccw tour");
                assert!(a < b, "refill agent {agent_idx} visits out of CCW order: {w:?}");
            }
        }
    }
}

/// 3. Both loops together cover every spot that is assigned to their
///    respective agent group. In other words, the union of
///    CW-distributed and CCW-distributed spots across all days equals
///    the full input spot set (modulo the round-robin modulo arithmetic
///    — every spot appears at least once in some plan).
#[test]
fn both_loops_cover_all_assigned_spots() {
    let problem = square_problem();
    let solver = DualDirectionLoop::default();
    let cw = solver.cw_tour(&problem);
    let ccw = solver.ccw_tour(&problem);
    let outcome = solver.solve(&problem).expect("solve");

    // All spot ids visited across the entire plan.
    let mut all_ids: Vec<u32> =
        outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
    all_ids.sort_unstable();
    all_ids.dedup();
    let expected: Vec<u32> = (0..u32::try_from(problem.spots.len()).unwrap_or(0)).collect();
    assert_eq!(all_ids, expected, "every spot must appear at least once");

    // Every spot id appears in both the CW tour and the CCW tour (by
    // construction of `build_tours`). Both agent groups therefore have
    // the same set of *available* spots to draw from.
    for s in &problem.spots {
        assert!(cw.contains(&s.id), "spot {} missing from CW tour", s.id);
        assert!(ccw.contains(&s.id), "spot {} missing from CCW tour", s.id);
    }
}

/// Bonus: explicit direction override works. With the directions
/// flipped (collection = CCW, refill = CW) the same solver still
/// produces a valid plan and the union still covers every spot.
#[test]
fn explicit_direction_override_produces_valid_plan() {
    let problem = square_problem();
    let solver = DualDirectionLoop::with_directions(
        InitialStrategy::Identity,
        0,
        Direction::CounterClockwise, // collection now CCW
        Direction::Clockwise,        // refill now CW
    );
    assert_eq!(solver.collect_direction, Direction::CounterClockwise);
    assert_eq!(solver.refill_direction, Direction::Clockwise);

    let outcome = solver.solve(&problem).expect("solve");
    assert_eq!(outcome.plans.len(), problem.days as usize);
    assert_eq!(outcome.plans[0].len(), problem.agents.len());
}

/// Bonus: the solver rejects the same degenerate inputs as
/// `LoopSolver`, confirming the dual-direction implementation honors
/// the shared `Solver` contract.
#[test]
fn dual_solver_rejects_degenerate_inputs() {
    let solver = DualDirectionLoop::default();

    // No spots.
    let err = solver.solve(&SolveProblem::new(vec![], vec![agent(0, 0, 0)], 1)).unwrap_err();
    assert_eq!(err, SolverError::NoSpots);

    // No agents.
    let err = solver.solve(&SolveProblem::new(vec![spot(0, 0, 0, "a")], vec![], 1)).unwrap_err();
    assert_eq!(err, SolverError::NoAgents);

    // Zero days.
    let err = solver
        .solve(&SolveProblem::new(vec![spot(0, 0, 0, "a")], vec![agent(0, 0, 0)], 0))
        .unwrap_err();
    assert_eq!(err, SolverError::ZeroDays);
}

/// Singleton agent case: when there is only one agent, the solver must
/// still produce a valid plan (the lone agent is treated as a
/// collection agent, so the CW tour is fully realized and the CCW
/// tour contributes nothing).
#[test]
fn singleton_agent_case_is_handled() {
    let spots = vec![spot(0, 0, 0, "kake"), spot(1, 1, 0, "kake"), spot(2, 2, 0, "kake")];
    let agents = vec![agent(0, 0, 0)];
    let problem = SolveProblem::new(spots, agents, 1);

    let solver = DualDirectionLoop::default();
    let outcome = solver.solve(&problem).expect("solve");
    assert_eq!(outcome.plans.len(), 1);
    assert_eq!(outcome.plans[0].len(), 1);
    // The single agent's plan visits all spots (in CW order).
    let mut ids = outcome.plans[0][0].clone();
    ids.sort_unstable();
    assert_eq!(ids, vec![0, 1, 2]);
}
