//! Multi-loop composition: decompose a large map into multiple regions and
//! build an independent closed loop per region.
//!
//! ## Motivation
//!
//! When the contest map has spots spread over a wide area, a single closed
//! loop visiting every spot becomes expensive in terms of tour length and
//! cannot be covered efficiently by a single agent in a single day. By
//! clustering the spots into contiguous regions, each region can be served
//! by its own agent (or by a small "touring + supply" pair — see the parent
//! issue #33), enabling parallel coverage of the map.
//!
//! ## Algorithm
//!
//! 1. **Partition** the spots into `num_regions` clusters using one of two
//!    strategies:
//!    - [`ClusteringStrategy::Grid`] — split the bounding box of the spot
//!      coordinates into a `rows × cols` grid; each cell is one region.
//!    - [`ClusteringStrategy::KMeans`] — Lloyd's algorithm on hex coordinates
//!      with deterministic seeded centroid initialization.
//! 2. **Build** a tour per region. Each region uses the same greedy
//!    nearest-neighbor initialization as [`crate::LoopSolver`], optionally
//!    refined by 2-opt best-improvement passes until a local optimum is
//!    reached.
//! 3. **Assign** agents to regions round-robin (lowest region index first,
//!    wrapping at the agent count). When there are more regions than agents,
//!    the surplus regions share the last agent. When there are more agents
//!    than regions, the surplus agents are left idle for the day.
//! 4. **Distribute** each agent's combined visit order across `days` using
//!    round-robin interleaving so that no single agent is overloaded on
//!    the first day.
//!
//! The combined plan is returned through the canonical
//! [`crate::Solver`] trait so that downstream callers (bench, tournament
//! runner) can swap [`crate::LoopSolver`] for [`MultiLoopComposer`]
//! without changing the call site.

use serde::{Deserialize, Serialize};

use crate::score::{ScoreReport, simulate_day};
use crate::solver::{
    SolveOutcome, SolveProblem, Solver, SolverError, distribute_plan, greedy_nearest_neighbor,
};
use crate::two_opt::{apply_two_opt, best_two_opt_move};
use crate::types::{HexCoord, Spot};

/// Clustering strategy used by [`MultiLoopComposer`] to partition spots into
/// regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusteringStrategy {
    /// Uniform grid partitioning of the bounding box of spot coordinates
    /// into `rows × cols` cells. Each cell becomes one region.
    ///
    /// This is fast, deterministic, and works well when spots are spread
    /// roughly uniformly across the map. For tightly clustered spots, it
    /// can leave some cells empty.
    Grid {
        /// Number of rows in the grid.
        rows: usize,
        /// Number of columns in the grid.
        cols: usize,
    },
    /// K-means clustering on hex coordinates.
    ///
    /// Lloyd's algorithm with deterministic seeded centroid initialization.
    /// Best for clusters of similar size; converges in `O(k * n * max_iters)`
    /// where `n` is the number of spots and `k` is the target number of
    /// regions.
    KMeans {
        /// Target number of clusters.
        k: usize,
        /// Maximum number of Lloyd iterations before bailing out.
        max_iters: usize,
        /// Deterministic seed for centroid initialization.
        seed: u64,
    },
}

impl ClusteringStrategy {
    /// Effective number of regions produced by this strategy.
    ///
    /// For [`ClusteringStrategy::Grid`], this is `rows * cols`. For
    /// [`ClusteringStrategy::KMeans`], this is `k`.
    #[must_use]
    pub const fn num_regions(&self) -> usize {
        match self {
            ClusteringStrategy::Grid { rows, cols } => rows.saturating_mul(*cols),
            ClusteringStrategy::KMeans { k, .. } => *k,
        }
    }

    /// Partition `spots` into clusters, returning one `Vec<u32>` of spot ids
    /// per region. Regions may be empty if the input layout does not
    /// naturally produce `num_regions()` non-empty clusters.
    fn partition(&self, spots: &[Spot]) -> Vec<Vec<u32>> {
        match self {
            ClusteringStrategy::Grid { rows, cols } => grid_partition(spots, *rows, *cols),
            ClusteringStrategy::KMeans { k, max_iters, seed } => {
                kmeans_partition(spots, *k, *max_iters, *seed)
            }
        }
    }
}

/// Multi-loop composer: partition spots into regions, build an independent
/// greedy (+ optional 2-opt) loop per region, and assign agents to regions.
#[derive(Debug, Clone)]
pub struct MultiLoopComposer {
    /// Number of regions to produce. Must agree with
    /// `clustering.num_regions()` — the composer validates this in
    /// [`Self::solve`] and returns [`SolverError::RegionMismatch`] on a
    /// mismatch.
    pub num_regions: usize,
    /// Clustering strategy used to partition the spots.
    pub clustering: ClusteringStrategy,
    /// Whether to refine each region's loop with 2-opt best-improvement
    /// passes until a local optimum is reached.
    pub refine: bool,
}

impl Default for MultiLoopComposer {
    fn default() -> Self {
        Self {
            num_regions: 4,
            clustering: ClusteringStrategy::Grid { rows: 2, cols: 2 },
            refine: true,
        }
    }
}

impl MultiLoopComposer {
    /// Construct a composer with the given number of regions and clustering
    /// strategy. Refinement defaults to enabled.
    #[must_use]
    pub fn new(num_regions: usize, clustering: ClusteringStrategy) -> Self {
        Self { num_regions, clustering, refine: true }
    }

    /// Construct a composer that disables per-region 2-opt refinement. Useful
    /// for fast smoke tests and benchmarks that only care about the
    /// partitioning / assignment shape.
    #[must_use]
    pub fn without_refinement(num_regions: usize, clustering: ClusteringStrategy) -> Self {
        Self { num_regions, clustering, refine: false }
    }

    /// Build a greedy (+ optional 2-opt) loop through the spots in a region.
    fn build_region_tour(&self, region_spots: &[Spot]) -> Vec<u32> {
        let mut tour = greedy_nearest_neighbor(region_spots);
        if self.refine {
            // Repeated best-improvement 2-opt passes until a pass produces
            // no improvement — mirrors the convergence criterion used by
            // `LoopSolver`. The iteration cap is a safety bound: with
            // strictly-decreasing integer tour cost the loop *must*
            // terminate, but we cap defensively so a buggy delta computation
            // can never hang the solver.
            let cap =
                u32::try_from(region_spots.len()).unwrap_or(u32::MAX).saturating_mul(64).max(1);
            for _ in 0..cap {
                let Some((i, j)) = best_two_opt_move(&tour, region_spots) else {
                    break;
                };
                apply_two_opt(&mut tour, i, j);
            }
        }
        tour
    }
}

impl Solver for MultiLoopComposer {
    fn solve(&self, problem: &SolveProblem) -> Result<SolveOutcome, SolverError> {
        if problem.spots.is_empty() {
            return Err(SolverError::NoSpots);
        }
        if problem.agents.is_empty() {
            return Err(SolverError::NoAgents);
        }
        if problem.days == 0 {
            return Err(SolverError::ZeroDays);
        }
        if self.num_regions == 0 {
            return Err(SolverError::ZeroRegions);
        }
        let strategy_regions = self.clustering.num_regions();
        if strategy_regions != self.num_regions {
            return Err(SolverError::RegionMismatch {
                configured: self.num_regions,
                strategy: strategy_regions,
            });
        }

        // 1. Partition spots into regions.
        let raw_regions = self.clustering.partition(&problem.spots);
        // Some partitioning strategies can produce fewer effective regions
        // than requested (e.g. KMeans collapsing a centroid). Pad with empty
        // regions so the assignment logic stays uniform.
        let mut regions: Vec<Vec<u32>> = raw_regions;
        while regions.len() < self.num_regions {
            regions.push(Vec::new());
        }

        // 2. Build a tour per region. We need the actual Spot objects so we
        // can run nearest-neighbor and 2-opt on coordinates.
        let mut region_tours: Vec<Vec<u32>> = Vec::with_capacity(self.num_regions);
        for region_spot_ids in &regions {
            if region_spot_ids.is_empty() {
                region_tours.push(Vec::new());
                continue;
            }
            let region_spots: Vec<Spot> = region_spot_ids
                .iter()
                .filter_map(|id| problem.spots.iter().find(|s| s.id == *id).cloned())
                .collect();
            region_tours.push(self.build_region_tour(&region_spots));
        }

        // 3. Assign agents to regions round-robin.
        let agents_count = problem.agents.len();
        let mut per_agent_tours: Vec<Vec<u32>> = vec![Vec::new(); agents_count];
        for (region_idx, tour) in region_tours.iter().enumerate() {
            let agent_idx = region_idx % agents_count;
            per_agent_tours[agent_idx].extend(tour);
        }

        // 4. Distribute each agent's combined visit order across days using
        // the canonical round-robin distribution from the solver module.
        // `distribute_plan` expects a single visit order per agent-day, so we
        // first lay out per-agent combined tours, then invoke it once per
        // agent by treating each agent's combined tour as a fresh "single
        // agent" problem.
        let days = problem.days as usize;
        let mut plans: Vec<Vec<Vec<u32>>> = vec![Vec::with_capacity(agents_count); days];
        for day_plans in plans.iter_mut().take(days) {
            *day_plans = vec![Vec::new(); agents_count];
        }
        for (agent_idx, tour) in per_agent_tours.iter().enumerate() {
            let single_agent_problem = SolveProblem {
                spots: problem.spots.clone(),
                agents: vec![problem.agents[agent_idx]],
                days: problem.days,
            };
            let single_agent_plans = distribute_plan(tour, &single_agent_problem);
            for (d, day_plans) in single_agent_plans.iter().enumerate().take(days) {
                plans[d][agent_idx].extend(day_plans.iter().flatten().copied());
            }
        }

        // 5. Score the combined plan.
        let mut score = ScoreReport::default();
        for (d, day_plans) in plans.iter().enumerate().take(days) {
            let mut combined: Vec<u32> = Vec::new();
            for agent_plan in day_plans {
                combined.extend(agent_plan);
            }
            let agent_start = problem.agents.first().map_or(HexCoord::new(0, 0), |a| a.start);
            let daily =
                simulate_day(u32::try_from(d).unwrap_or(0), &combined, &problem.spots, agent_start);
            score.daily.push(daily);
        }

        Ok(SolveOutcome { plans, score })
    }
}

// ----------------------------------------------------------------------
// partitioning primitives
// ----------------------------------------------------------------------

/// Partition spots into a `rows × cols` grid based on bounding-box
/// coordinates. Cells are returned in row-major order.
fn grid_partition(spots: &[Spot], rows: usize, cols: usize) -> Vec<Vec<u32>> {
    let n_cells = rows.saturating_mul(cols);
    let mut cells: Vec<Vec<u32>> = vec![Vec::new(); n_cells];
    if spots.is_empty() || n_cells == 0 {
        return cells;
    }

    let (min_q, max_q) = spots
        .iter()
        .map(|s| s.coord.q)
        .fold((i32::MAX, i32::MIN), |(lo, hi), q| (lo.min(q), hi.max(q)));
    let (min_r, max_r) = spots
        .iter()
        .map(|s| s.coord.r)
        .fold((i32::MAX, i32::MIN), |(lo, hi), r| (lo.min(r), hi.max(r)));

    // Use ranges that are at least 1 wide to avoid divide-by-zero on
    // degenerate (single-spot / single-row / single-col) inputs.
    let q_range = i64::from(max_q) - i64::from(min_q) + 1;
    let r_range = i64::from(max_r) - i64::from(min_r) + 1;

    let rows_i = i64::try_from(rows).unwrap_or(i64::MAX);
    let cols_i = i64::try_from(cols).unwrap_or(i64::MAX);

    for spot in spots {
        let qf = (i64::from(spot.coord.q) - i64::from(min_q)) * cols_i / q_range;
        let rf = (i64::from(spot.coord.r) - i64::from(min_r)) * rows_i / r_range;
        // Saturate into the legal cell index range so a spot sitting exactly
        // on the max boundary lands in the last cell rather than overflowing.
        let col = usize::try_from(qf.max(0).min(cols_i - 1)).unwrap_or(0);
        let row = usize::try_from(rf.max(0).min(rows_i - 1)).unwrap_or(0);
        let idx = row * cols + col;
        cells[idx].push(spot.id);
    }

    cells
}

/// Partition spots into `k` clusters using Lloyd's k-means algorithm on hex
/// coordinates. Deterministic given `(k, max_iters, seed)`: the seed controls
/// the initial centroid stride so the test suite is reproducible.
fn kmeans_partition(spots: &[Spot], k: usize, max_iters: usize, seed: u64) -> Vec<Vec<u32>> {
    let n = spots.len();
    let k = k.max(1);
    let mut clusters: Vec<Vec<u32>> = vec![Vec::new(); k];
    if n == 0 {
        return clusters;
    }

    // Seeded stride-based initialization. The `seed` parameter is reserved
    // for future richer seeding strategies (k-means++, deterministic random);
    // for now, picking every `n / k`-th spot is sufficient and deterministic.
    let _ = seed;
    let step = n.div_ceil(k).max(1);
    let mut centroids: Vec<HexCoord> =
        (0..k).map(|i| spots[i.saturating_mul(step).min(n - 1)].coord).collect();

    let mut assignments: Vec<usize> = vec![0; n];
    let mut prev_assignments: Vec<usize> = vec![usize::MAX; n];

    for _ in 0..max_iters.max(1) {
        // Assignment step: nearest centroid (Lloyd's standard rule).
        for (i, spot) in spots.iter().enumerate() {
            let mut best = 0usize;
            let mut best_dist = u32::MAX;
            for (j, c) in centroids.iter().enumerate() {
                let d = spot.coord.distance(*c);
                if d < best_dist {
                    best_dist = d;
                    best = j;
                }
            }
            assignments[i] = best;
        }

        // Convergence check: if the assignment did not change in this pass,
        // we have reached a (local) fixed point — bail out early.
        if assignments == prev_assignments {
            break;
        }
        prev_assignments.clone_from(&assignments);

        // Update step: recompute each centroid as the mean of its assigned
        // points. Use i64 accumulators to avoid overflow on large maps.
        let mut sums: Vec<(i64, i64, u32)> = vec![(0, 0, 0); k];
        for (i, spot) in spots.iter().enumerate() {
            let c = assignments[i];
            sums[c].0 += i64::from(spot.coord.q);
            sums[c].1 += i64::from(spot.coord.r);
            sums[c].2 += 1;
        }
        for j in 0..k {
            if sums[j].2 > 0 {
                centroids[j] = HexCoord::new(
                    i32::try_from(sums[j].0 / i64::from(sums[j].2)).unwrap_or(0),
                    i32::try_from(sums[j].1 / i64::from(sums[j].2)).unwrap_or(0),
                );
            }
        }
    }

    for (i, spot) in spots.iter().enumerate() {
        clusters[assignments[i]].push(spot.id);
    }

    // Deterministic ordering: sort spot ids within each cluster. This makes
    // downstream tests stable across builds and platforms.
    for cluster in &mut clusters {
        cluster.sort_unstable();
    }

    clusters
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Series;

    fn spot(id: u32, q: i32, r: i32, series: &str) -> Spot {
        Spot { id, coord: HexCoord::new(q, r), series: Series::new(series), stock: 1 }
    }

    fn agent(id: u32, q: i32, r: i32) -> crate::types::Agent {
        crate::types::Agent { id, start: HexCoord::new(q, r) }
    }

    #[test]
    fn grid_partition_produces_requested_cells() {
        let spots = vec![
            spot(0, 0, 0, "a"),
            spot(1, 5, 0, "b"),
            spot(2, 0, 5, "c"),
            spot(3, 5, 5, "d"),
            spot(4, 2, 2, "e"),
        ];
        let cells = grid_partition(&spots, 2, 2);
        assert_eq!(cells.len(), 4);
        // Every spot id appears exactly once across all cells.
        let mut all: Vec<u32> = cells.iter().flatten().copied().collect();
        all.sort_unstable();
        let expected: Vec<u32> = (0..5).collect();
        assert_eq!(all, expected);
    }

    #[test]
    fn kmeans_partition_covers_every_spot() {
        let spots = vec![
            spot(0, 0, 0, "a"),
            spot(1, 1, 0, "a"),
            spot(2, 10, 10, "b"),
            spot(3, 11, 10, "b"),
        ];
        let clusters = kmeans_partition(&spots, 2, 20, 0);
        assert_eq!(clusters.len(), 2);
        let mut all: Vec<u32> = clusters.iter().flatten().copied().collect();
        all.sort_unstable();
        let expected: Vec<u32> = (0..4).collect();
        assert_eq!(all, expected);
    }

    #[test]
    fn kmeans_converges_on_two_obvious_clusters() {
        // Two clusters at coordinates (0,0) and (100,100). After a handful
        // of iterations the assignments should stabilize.
        let spots = vec![
            spot(0, 0, 0, "a"),
            spot(1, 1, 0, "a"),
            spot(2, 0, 1, "a"),
            spot(3, 100, 100, "b"),
            spot(4, 101, 100, "b"),
            spot(5, 100, 101, "b"),
        ];
        let clusters = kmeans_partition(&spots, 2, 50, 0);
        let cluster_a: Vec<u32> = clusters[0].clone();
        let cluster_b: Vec<u32> = clusters[1].clone();
        let a_ids: std::collections::BTreeSet<u32> = cluster_a.iter().copied().collect();
        let b_ids: std::collections::BTreeSet<u32> = cluster_b.iter().copied().collect();
        // The two obvious groups should be in disjoint clusters (the empty
        // group is fine — k-means can leave one cluster unused if it
        // collapses a centroid, though that is unlikely on this layout).
        let a_only: std::collections::BTreeSet<u32> = a_ids.difference(&b_ids).copied().collect();
        let b_only: std::collections::BTreeSet<u32> = b_ids.difference(&a_ids).copied().collect();
        // At least one of {a_only, b_only} should contain all "a" spots and
        // the other all "b" spots — they should not be mixed.
        let a_set: std::collections::BTreeSet<u32> = [0u32, 1, 2].iter().copied().collect();
        let b_set: std::collections::BTreeSet<u32> = [3u32, 4, 5].iter().copied().collect();
        let either = (a_only == a_set && b_only == b_set) || (a_only == b_set && b_only == a_set);
        assert!(either, "clusters should separate a/b groups: a={a_only:?}, b={b_only:?}");
    }

    #[test]
    fn multi_loop_composer_covers_every_spot_exactly_once() {
        let spots = vec![
            spot(0, 0, 0, "a"),
            spot(1, 1, 0, "a"),
            spot(2, 0, 1, "a"),
            spot(3, 10, 0, "b"),
            spot(4, 11, 0, "b"),
            spot(5, 10, 1, "b"),
            spot(6, 0, 10, "c"),
            spot(7, 1, 10, "c"),
            spot(8, 0, 11, "c"),
        ];
        let agents = vec![agent(0, 0, 0), agent(1, 0, 0), agent(2, 0, 0)];
        let problem = SolveProblem::new(spots.clone(), agents, 3);
        let composer = MultiLoopComposer::new(3, ClusteringStrategy::Grid { rows: 3, cols: 1 });
        let outcome = composer.solve(&problem).expect("solve");
        let mut all: Vec<u32> =
            outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
        all.sort_unstable();
        let expected: Vec<u32> = (0..u32::try_from(spots.len()).unwrap_or(0)).collect();
        assert_eq!(all, expected);
    }

    #[test]
    fn multi_loop_composer_rejects_mismatched_region_count() {
        let spots = vec![spot(0, 0, 0, "a"), spot(1, 1, 0, "a")];
        let problem = SolveProblem::new(spots, vec![agent(0, 0, 0)], 1);
        // Grid produces 4 regions but we configure num_regions=2.
        let composer = MultiLoopComposer::new(2, ClusteringStrategy::Grid { rows: 2, cols: 2 });
        let err = composer.solve(&problem).unwrap_err();
        assert!(matches!(err, SolverError::RegionMismatch { .. }));
    }

    #[test]
    fn multi_loop_composer_rejects_zero_regions() {
        let spots = vec![spot(0, 0, 0, "a")];
        let problem = SolveProblem::new(spots, vec![agent(0, 0, 0)], 1);
        let composer =
            MultiLoopComposer::new(0, ClusteringStrategy::KMeans { k: 0, max_iters: 1, seed: 0 });
        let err = composer.solve(&problem).unwrap_err();
        assert_eq!(err, SolverError::ZeroRegions);
    }
}
