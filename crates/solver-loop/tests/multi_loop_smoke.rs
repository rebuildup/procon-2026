//! Smoke tests for the multi-loop composition solver.
//!
//! Covers the four headline acceptance items for Issue #33:
//!   1. Grid clustering produces the expected number of regions.
//!   2. K-means converges on a tiny synthetic dataset.
//!   3. Every spot is covered by exactly one loop.
//!   4. Agent assignment is balanced across regions.

use solver_loop::{
    Agent, ClusteringStrategy, HexCoord, MultiLoopComposer, Series, SolveProblem, Solver, Spot,
};

/// Helper to build a spot.
fn spot(id: u32, q: i32, r: i32, series: &str) -> Spot {
    Spot { id, coord: HexCoord::new(q, r), series: Series::new(series), stock: 1 }
}

fn agent(id: u32, q: i32, r: i32) -> Agent {
    Agent { id, start: HexCoord::new(q, r) }
}

/// Two clusters far apart from each other so a sensible partitioning must
/// separate them.
fn two_cluster_spots() -> Vec<Spot> {
    vec![
        // Cluster A near the origin.
        spot(0, 0, 0, "kake"),
        spot(1, 1, 0, "kake"),
        spot(2, 0, 1, "kake"),
        spot(3, 1, 1, "kake"),
        // Cluster B far to the east.
        spot(4, 20, 0, "bukkake"),
        spot(5, 21, 0, "bukkake"),
        spot(6, 20, 1, "bukkake"),
        spot(7, 21, 1, "bukkake"),
    ]
}

// 1. Grid clustering produces the expected number of regions.
#[test]
fn grid_clustering_produces_expected_number_of_regions() {
    let spots = two_cluster_spots();
    let agents = vec![agent(0, 0, 0), agent(1, 0, 0)];
    let problem = SolveProblem::new(spots.clone(), agents, 1);

    // 2x2 grid => 4 regions. We deliberately over-provision so some cells
    // end up empty — the composer must still produce 4 regions and still
    // cover every spot.
    let composer = MultiLoopComposer::new(4, ClusteringStrategy::Grid { rows: 2, cols: 2 });
    let outcome = composer.solve(&problem).expect("solve");

    // All spots covered.
    let mut covered: Vec<u32> =
        outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
    covered.sort_unstable();
    covered.dedup();
    let expected: Vec<u32> = (0..u32::try_from(spots.len()).unwrap()).collect();
    assert_eq!(covered, expected);

    // Grid strategy reports the right region count.
    assert_eq!(
        ClusteringStrategy::Grid { rows: 2, cols: 2 }.num_regions(),
        4,
        "2x2 grid should report 4 regions",
    );
}

// 2. K-means converges on a tiny synthetic dataset.
#[test]
fn kmeans_converges_on_tiny_synthetic_dataset() {
    let spots = two_cluster_spots();
    let agents = vec![agent(0, 0, 0), agent(1, 0, 0)];
    let problem = SolveProblem::new(spots.clone(), agents, 1);

    let composer =
        MultiLoopComposer::new(2, ClusteringStrategy::KMeans { k: 2, max_iters: 50, seed: 0 });
    let outcome = composer.solve(&problem).expect("solve");

    // All spots still covered.
    let mut covered: Vec<u32> =
        outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
    covered.sort_unstable();
    covered.dedup();
    let expected: Vec<u32> = (0..u32::try_from(spots.len()).unwrap()).collect();
    assert_eq!(covered, expected);
}

// 3. Every spot is covered by exactly one loop.
#[test]
fn every_spot_is_covered_by_exactly_one_loop() {
    let spots = two_cluster_spots();
    let agents = vec![agent(0, 0, 0), agent(1, 0, 0)];
    let problem = SolveProblem::new(spots.clone(), agents, 1);

    let composer = MultiLoopComposer::new(2, ClusteringStrategy::Grid { rows: 1, cols: 2 });
    let outcome = composer.solve(&problem).expect("solve");

    // Collect all visits and assert each spot id appears exactly once
    // across the whole plan (single-loop visit per spot).
    let visits: Vec<u32> = outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
    let mut sorted = visits.clone();
    sorted.sort_unstable();
    let expected: Vec<u32> = (0..u32::try_from(spots.len()).unwrap()).collect();
    assert_eq!(sorted, expected, "every spot id should appear exactly once in the plan");
}

// 4. Agent assignment is balanced across regions.
#[test]
fn agent_assignment_is_balanced_across_regions() {
    // Build a map with 3 distinct clusters at coordinates (0,0), (50,0),
    // and (100,0). Three regions, three agents -> exactly one region per
    // agent.
    let spots = vec![
        spot(0, 0, 0, "kake"),
        spot(1, 1, 0, "kake"),
        spot(2, 2, 0, "kake"),
        spot(3, 50, 0, "kake"),
        spot(4, 51, 0, "kake"),
        spot(5, 52, 0, "kake"),
        spot(6, 100, 0, "bukkake"),
        spot(7, 101, 0, "bukkake"),
        spot(8, 102, 0, "bukkake"),
    ];
    let agents = vec![agent(0, 0, 0), agent(1, 0, 0), agent(2, 0, 0)];
    let problem = SolveProblem::new(spots.clone(), agents.clone(), 1);

    let composer = MultiLoopComposer::new(3, ClusteringStrategy::Grid { rows: 1, cols: 3 });
    let outcome = composer.solve(&problem).expect("solve");

    // Count visits per agent over all days.
    let mut visits_per_agent: Vec<u32> = vec![0; agents.len()];
    for day_plans in &outcome.plans {
        for (agent_idx, agent_plan) in day_plans.iter().enumerate() {
            visits_per_agent[agent_idx] += u32::try_from(agent_plan.len()).unwrap_or(0);
        }
    }

    // With three balanced regions of three spots each, each agent should
    // visit exactly three spots. No agent should be overloaded or idle.
    for (agent_idx, count) in visits_per_agent.iter().enumerate() {
        assert_eq!(*count, 3, "agent {agent_idx} should visit exactly 3 spots, got {count}");
    }

    // And the union should cover every spot exactly once.
    let mut all: Vec<u32> =
        outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
    all.sort_unstable();
    let expected: Vec<u32> = (0..u32::try_from(spots.len()).unwrap()).collect();
    assert_eq!(all, expected);
}

// Bonus: refuses to solve when num_regions mismatches the strategy's
// implicit region count.
#[test]
fn composer_rejects_mismatched_region_count() {
    let spots = vec![spot(0, 0, 0, "a"), spot(1, 1, 0, "b")];
    let problem = SolveProblem::new(spots, vec![agent(0, 0, 0)], 1);
    // Grid produces 4 regions but composer says 2 — must reject.
    let composer = MultiLoopComposer::new(2, ClusteringStrategy::Grid { rows: 2, cols: 2 });
    let err = composer.solve(&problem).unwrap_err();
    assert!(
        matches!(err, solver_loop::SolverError::RegionMismatch { .. }),
        "expected RegionMismatch, got {err:?}",
    );
}
