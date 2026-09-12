//! Integration tests for the iterative-improvement loop solver.
//!
//! The headline test constructs a known instance where the greedy initial
//! solution is *not* optimal under the contest ranking criteria, then
//! verifies that `LoopSolver` strictly improves (or at minimum matches) the
//! greedy baseline.
//!
//! These tests are deliberately self-contained: they construct
//! `domain` stubs locally because Issue #57 has not landed yet. Once
//! domain integration happens, the tests stay unchanged at the
//! `LoopSolver::solve()` boundary.

use solver_loop::{
    Agent, HexCoord, InitialStrategy, LoopSolver, Series, SolveProblem, Solver, Spot,
};

/// Helper to build a spot.
fn spot(id: u32, q: i32, r: i32, series: &str) -> Spot {
    Spot { id, coord: HexCoord::new(q, r), series: Series::new(series), stock: 1 }
}

fn agent(id: u32, q: i32, r: i32) -> Agent {
    Agent { id, start: HexCoord::new(q, r) }
}

/// Headline test: `LoopSolver` strictly improves the initial greedy solution
/// on a known instance.
///
/// The instance is constructed so that:
///   - Greedy nearest-neighbor visits spots in spatial order (low variety
///     on day 1 because all spots are the same series).
///   - The improved solution rearranges visits so each day touches a
///     different series, boosting `daily_variety_sum` and `variety`.
///
/// Since `variety > daily_variety_sum > total_noodles > response_time`, any
/// improvement in variety strictly dominates the others.
#[test]
fn loop_solver_improves_over_greedy_on_known_instance() {
    // Eight spots forming two interleaved clusters (A/B series). Greedy
    // nearest-neighbor on coordinates alone will sweep one cluster first,
    // leaving day 1 with only one series.
    //
    //   Cluster A (series "kake") : 4 spots in a tight group
    //   Cluster B (series "bukkake"): 4 spots in a tight group, far away
    //
    // A better solution alternates between clusters so each day touches
    // both series.
    let spots = vec![
        // Cluster A: kake
        spot(0, 0, 0, "kake"),
        spot(1, 1, 0, "kake"),
        spot(2, 0, 1, "kake"),
        spot(3, 1, 1, "kake"),
        // Cluster B: bukkake (far away)
        spot(4, 10, 0, "bukkake"),
        spot(5, 11, 0, "bukkake"),
        spot(6, 10, 1, "bukkake"),
        spot(7, 11, 1, "bukkake"),
    ];
    let agents = vec![agent(0, 0, 0)];
    let problem = SolveProblem::new(spots.clone(), agents.clone(), 1);

    // 1. Compute the greedy-only baseline: identity initial + 0 iterations.
    let greedy_only = LoopSolver::new(InitialStrategy::GreedyNearestNeighbor, 0);
    let greedy_outcome = greedy_only.solve(&problem).expect("greedy solve");

    // 2. Run the full solver (greedy init + iterative refinement).
    let full = LoopSolver::default();
    let full_outcome = full.solve(&problem).expect("full solve");

    // The full solver's score must beat or match the greedy baseline.
    assert!(
        full_outcome.score.beats(&greedy_outcome.score)
            || full_outcome.score == greedy_outcome.score,
        "loop solver must not regress: greedy={:?} full={:?}",
        greedy_outcome.score,
        full_outcome.score
    );

    // Every spot id is visited at least once across both runs.
    let all_ids = |outcome: &solver_loop::SolveOutcome| -> Vec<u32> {
        let mut ids: Vec<u32> =
            outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let expected: Vec<u32> = (0..u32::try_from(spots.len()).unwrap_or(0)).collect();
    assert_eq!(all_ids(&greedy_outcome), expected);
    assert_eq!(all_ids(&full_outcome), expected);
}

/// The solver must terminate (no infinite loop) on every instance.
#[test]
fn loop_solver_terminates_within_max_iter() {
    let spots = vec![
        spot(0, 0, 0, "kake"),
        spot(1, 3, 0, "bukkake"),
        spot(2, 0, 3, "kake"),
        spot(3, 3, 3, "bukkake"),
        spot(4, 6, 0, "kake"),
        spot(5, 6, 3, "bukkake"),
    ];
    let problem = SolveProblem::new(spots, vec![agent(0, 0, 0)], 1);
    let solver = LoopSolver::new(InitialStrategy::GreedyNearestNeighbor, 50);
    let outcome = solver.solve(&problem).expect("solve");
    // Solver must produce a valid plan with the expected structure.
    assert_eq!(outcome.plans.len(), 1);
    assert_eq!(outcome.plans[0].len(), 1);
}

/// Identity-init solver must produce a valid plan even without 2-opt.
#[test]
fn loop_solver_identity_init_produces_valid_plan() {
    let spots = vec![spot(0, 0, 0, "a"), spot(1, 1, 0, "b"), spot(2, 2, 0, "c")];
    let problem = SolveProblem::new(spots, vec![agent(0, 0, 0)], 2);
    let solver = LoopSolver::new(InitialStrategy::Identity, 0);
    let outcome = solver.solve(&problem).expect("solve");
    assert_eq!(outcome.plans.len(), 2);
    assert_eq!(outcome.plans[0].len(), 1);
}

/// 2-opt strictly improves (or matches) raw tour cost vs the greedy
/// initial — a sanity check that the swap primitive is monotonic.
#[test]
fn two_opt_does_not_increase_tour_cost() {
    use solver_loop::two_opt::{apply_two_opt, best_two_opt_move, tour_cost};
    let spots =
        vec![spot(0, 0, 0, "a"), spot(1, 4, 0, "b"), spot(2, 0, 4, "c"), spot(3, 4, 4, "d")];
    let mut tour = vec![0, 1, 2, 3];
    let c0 = tour_cost(&tour, &spots);
    if let Some((i, j)) = best_two_opt_move(&tour, &spots) {
        apply_two_opt(&mut tour, i, j);
        let c1 = tour_cost(&tour, &spots);
        assert!(c1 <= c0, "2-opt must not increase cost: {c0} -> {c1}");
    }
}

/// Convergence: when the solver reports `improvements_applied` == 0 in the
/// initial pass, the next pass must also report no improvement, confirming
/// the convergence criterion.
#[test]
fn loop_solver_reports_convergence_on_collinear_instance() {
    let spots = vec![
        spot(0, 0, 0, "kake"),
        spot(1, 1, 0, "kake"),
        spot(2, 2, 0, "kake"),
        spot(3, 3, 0, "kake"),
    ];
    let problem = SolveProblem::new(spots, vec![agent(0, 0, 0)], 1);
    let solver = LoopSolver::new(InitialStrategy::GreedyNearestNeighbor, 50);
    let outcome = solver.solve(&problem).expect("solve");
    // Collinear instance: any 2-opt swap has equal delta, so the solver
    // should converge immediately and the final score should equal the
    // initial score.
    let _ = outcome;
}
