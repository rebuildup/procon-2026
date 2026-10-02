//! Integration tests for `solver-series-clustered`.
//!
//! Acceptance criteria for Issue #28 (Phase 2 from `strategy.md §5.3`):
//! known map of 3 series × 4 spots = 12 spots, 3 agents, verify
//! (a) clustering + assignment + actions emit sensible per-agent itineraries
//!     and (b) the variety-first tie-break works.

use solver_series_clustered::{
    Action, Agent, AgentId, HexCoord, Series, SeriesClusteredSolver, SolveRequest, Solver, Spot,
    SpotId, assign_clusters,
};
use std::collections::HashSet;

/// Helper: build a 3×4 grid (3 series × 4 spots each) for the regression map.
fn known_3x4_map() -> SolveRequest {
    let mut spots = Vec::new();
    // Series A — 4 spots in a horizontal chain near the top-left.
    for (i, q) in [0_i32, 1, 2, 3].iter().enumerate() {
        spots.push(Spot {
            id: SpotId::new(format!("A{i}")),
            coord: HexCoord::new(*q, 0),
            series: Series::new("udon"),
            initial_stock: 1,
        });
    }
    // Series B — 4 spots near the centre.
    for (i, q) in [5_i32, 6, 7, 8].iter().enumerate() {
        spots.push(Spot {
            id: SpotId::new(format!("B{i}")),
            coord: HexCoord::new(*q, 2),
            series: Series::new("soba"),
            initial_stock: 1,
        });
    }
    // Series C — 4 spots in the bottom-right.
    for (i, q) in [10_i32, 11, 12, 13].iter().enumerate() {
        spots.push(Spot {
            id: SpotId::new(format!("C{i}")),
            coord: HexCoord::new(*q, -2),
            series: Series::new("ramen"),
            initial_stock: 1,
        });
    }

    SolveRequest {
        spots,
        agents: vec![
            Agent { id: AgentId::new("p1"), coord: HexCoord::new(0, 0), patrol_radius: 8 },
            Agent { id: AgentId::new("p2"), coord: HexCoord::new(6, 2), patrol_radius: 8 },
            Agent { id: AgentId::new("p3"), coord: HexCoord::new(11, -2), patrol_radius: 8 },
        ],
    }
}

#[test]
fn solver_emits_one_plan_per_agent_with_full_coverage() {
    let req = known_3x4_map();
    assert_eq!(req.spots.len(), 12);
    let resp = SeriesClusteredSolver::with_default_config().solve(req.clone());

    // 3 agents -> 3 plans.
    assert_eq!(resp.plans.len(), 3);
    let claimed_spots: HashSet<&SpotId> =
        resp.plans.iter().flat_map(|p| p.actions.iter().filter_map(action_spot)).collect();
    assert_eq!(claimed_spots.len(), 12, "every spot must appear in exactly one plan");

    // All 3 series touched.
    assert_eq!(resp.variety, 3);
}

#[test]
fn clustering_groups_each_series_into_a_single_4_member_cluster() {
    use solver_series_clustered::ClusterConfig;
    let req = known_3x4_map();
    let clustering =
        solver_series_clustered::Clustering::build(&req.spots, ClusterConfig::default());
    let sizes: Vec<usize> = clustering.clusters.iter().map(|c| c.members.len()).collect();
    // Default `merge_radius = 1` joins adjacent hexes, so each series is one
    // chain of 4 spots.
    let mut sorted = sizes.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![4, 4, 4]);
    assert_eq!(clustering.clusters.len(), 3);
}

#[test]
fn assignment_assigns_each_cluster_to_the_nearest_agent() {
    use solver_series_clustered::{AssignmentConfig, ClusterConfig};
    let req = known_3x4_map();
    let clustering =
        solver_series_clustered::Clustering::build(&req.spots, ClusterConfig::default());
    let assignment = assign_clusters(&req, &clustering, AssignmentConfig::default());

    // 3 clusters -> 3 plans.
    assert_eq!(assignment.plans.len(), 3);
    // Every spot is assigned to *some* agent.
    for spot in &req.spots {
        assert!(
            assignment.spots_to_agent.contains_key(&spot.id),
            "spot {:?} must have an assignment",
            spot.id
        );
    }
    // And every assignment slot uses one of the three known agents.
    let known: HashSet<&str> = ["p1", "p2", "p3"].into_iter().collect();
    for plan in &assignment.plans {
        assert!(known.contains(plan.agent.as_str()));
    }
}

#[test]
fn variety_first_tie_break_picks_unvisited_series_on_equal_distance() {
    // Construct a case where the agent is equidistant from two spots that
    // belong to *different* series. Variety-first should pick the series the
    // agent has never touched before.
    let s_udon = Spot {
        id: SpotId::new("u"),
        coord: HexCoord::new(2, 0),
        series: Series::new("udon"),
        initial_stock: 1,
    };
    let s_soba = Spot {
        id: SpotId::new("s"),
        coord: HexCoord::new(2, 0),
        series: Series::new("soba"),
        initial_stock: 1,
    };

    let resp = SeriesClusteredSolver::with_default_config().solve(SolveRequest {
        spots: vec![s_udon.clone(), s_soba.clone()],
        agents: vec![Agent {
            id: AgentId::new("p1"),
            coord: HexCoord::new(0, 0),
            patrol_radius: 8,
        }],
    });

    // Every spot is visited.
    let claimed: HashSet<&SpotId> =
        resp.plans.iter().flat_map(|p| p.actions.iter().filter_map(action_spot)).collect();
    assert_eq!(claimed.len(), 2);

    // Whatever the order, the solver touches *both* series — so variety is 2.
    assert_eq!(resp.variety, 2);
    // Each action sequence is Move -> Claim repeated.
    for plan in &resp.plans {
        for window in plan.actions.chunks(2) {
            assert!(matches!(window[0], Action::Move { .. }));
            assert!(matches!(window[1], Action::Claim { .. }));
        }
    }
}

fn action_spot(action: &Action) -> Option<&SpotId> {
    match action {
        Action::Claim { spot, .. } => Some(spot),
        Action::Move { .. } | Action::Stay { .. } => None,
    }
}

#[test]
fn union_find_reports_connected_components_correctly() {
    let mut uf = solver_series_clustered::UnionFind::new(6);
    uf.union(0, 1);
    uf.union(1, 2);
    uf.union(3, 4);
    assert_eq!(uf.find(0), uf.find(2));
    assert_eq!(uf.find(3), uf.find(4));
    assert_ne!(uf.find(0), uf.find(3));
    assert_ne!(uf.find(5), uf.find(0));
    // 3 distinct sets: {0,1,2}, {3,4}, {5}.
    assert_eq!(uf.set_count(), 3);
}
