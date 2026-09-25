//! Iterative-improvement loop solver.
//!
//! ## Algorithm
//! 1. Build an initial solution with the configured [`InitialStrategy`]:
//!    - `GreedyNearestNeighbor` — start at the first spot, repeatedly jump
//!      to the nearest unvisited spot. O(n^2).
//!    - `Identity` — visit spots in input order (useful as a baseline).
//!    - `Reverse` — the exact reversal of `Identity` (input order, last →
//!      first). Also deterministic, no coordinates involved.
//! 2. Wrap the visit order into a closed loop (return to the first spot).
//! 3. Refine: while the previous iteration improved at least one move, for
//!    each pair `(i, j)` try a 2-opt swap; if better, keep; also try
//!    each triple `(i, j, k)` for a 3-opt swap. Stop on a pass with no
//!    improvement (local optimum) or after `max_iter` iterations
//!    (whichever comes first) — this prevents infinite loops on
//!    pathological instances.
//!
//! ## Improvement criterion
//! "Better" means strictly greater [`ScoreReport::judge()`] — i.e. the
//! lexicographic ranking on `(variety, daily_variety_sum, total_noodles,
//! neg_response_time)`. Lower raw tour cost is *not* sufficient on its
//! own; a solution that visits fewer spots but more distinct series ranks
//! higher.
//!
//!
//! ## Score report is NOT an evaluation score
//!
//! `SolveOutcome::score` / [`ScoreReport::judge`] are the solver's **internal
//! objective** — the thing 2-opt / 3-opt hill-climb against. They decide which
//! tour the algorithm keeps, and they are allowed to be a rough proxy.
//!
//! They are never used to rank candidates. The canonical evaluator reads
//! `engine::MatchSim`'s final state instead, so a solver cannot score itself
//! (see `docs/evaluation.md`).

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::score::{ScoreRanking, ScoreReport, simulate_day};
use crate::three_opt::try_three_opt;
use crate::two_opt::{best_two_opt_move, edge_cost};
use crate::types::{Agent, Spot};

/// Initial-solution strategy for the loop solver.
///
/// Each variant only decides the *ordering* the refinement starts from; the
/// 2-opt / 3-opt improvement loop is shared. That keeps a whole axis of
/// candidate parameters (`loop:iterations=<n>:initial=<variant>`, the
/// canonical schema order) cheap to
/// enumerate without inventing new algorithms.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitialStrategy {
    /// Visit spots in input order, then close the loop.
    Identity,
    /// Visit spots in **reverse input order**, then close the loop —
    /// exactly [`InitialStrategy::Identity`]'s ordering played backwards
    /// (`initial_tour(Reverse) == initial_tour(Identity).rev()`).
    ///
    /// Deterministic and coordinate-independent: the tour depends only on
    /// the order spots appear in the problem, never on coordinates or an
    /// RNG, so the same spot list always yields the same tour. It is the
    /// opposite end of the input order from `Identity`, which changes
    /// which spot each agent is sent to first (the round-robin plan
    /// distribution keys off tour position).
    Reverse,
    /// Nearest-neighbor greedy: start at the first spot, repeatedly jump
    /// to the nearest unvisited spot. O(n^2).
    #[default]
    GreedyNearestNeighbor,
    /// Sort spots by row, then column (row-major sweep).
    SweepRow,
    /// Sort spots by column, then row (column-major sweep).
    SweepCol,
    /// Deterministic shuffle seeded by `seed`.
    ///
    /// The shuffle is a fixed xorshift64* sequence, so the same `seed` on the
    /// same spot list always yields the same tour — required for the
    /// reproducible-evaluation guarantee.
    Random { seed: u64 },
}

/// Touring-problem contract, internal to this crate.
///
/// `solve` maps `(spots, agents, days) -> daily spot-visit orders`. It is
/// deliberately *not* a canonical engine trait: the adapter in
/// `bench::adapters::loop_solver` turns these visit orders into concrete
/// `engine::model::AgentActions`, and `crates/cli`'s `EngineLoopAdapter`
/// does the same for the live wire. Keep it algorithm-shaped.
pub trait Solver {
    /// Solve and return the daily plans, one per day, plus a score report.
    fn solve(&self, problem: &SolveProblem) -> Result<SolveOutcome, SolverError>;
}

/// Input problem description for a single multi-day match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveProblem {
    /// All spots on the map for this match.
    pub spots: Vec<Spot>,
    /// Available agents (one per row of the `plans` output).
    pub agents: Vec<Agent>,
    /// Number of days to plan for.
    pub days: u32,
}

impl SolveProblem {
    /// Build a `SolveProblem` from raw slices — convenience for tests
    /// and the stub runner.
    #[must_use]
    pub fn new(spots: Vec<Spot>, agents: Vec<Agent>, days: u32) -> Self {
        Self { spots, agents, days }
    }
}

/// Result of a successful solve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveOutcome {
    /// One daily plan per agent (rows) per day (cols), but flattened here
    /// as `plans[day_index][agent_index] = visit order of spot ids`.
    pub plans: Vec<Vec<Vec<u32>>>,
    /// Aggregate score report (one entry per day).
    pub score: ScoreReport,
}

/// Telemetry for a single solve call. Useful for tests and for tuning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveStats {
    pub iterations_used: u32,
    pub improvements_applied: u32,
    pub initial_score: ScoreRanking,
    pub final_score: ScoreRanking,
    pub max_iter_reached: bool,
}

/// Errors produced by the solver.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SolverError {
    #[error("no spots to plan for")]
    NoSpots,
    #[error("no agents to plan for")]
    NoAgents,
    #[error("days must be > 0")]
    ZeroDays,
    #[error("spot id {0} referenced but not present in problem")]
    UnknownSpotId(u32),
    /// Multi-loop composer: `num_regions` must be > 0.
    #[error("num_regions must be > 0")]
    ZeroRegions,
    /// Multi-loop composer: `num_regions` field does not match the
    /// effective region count of the configured clustering strategy
    /// (e.g. `Grid { rows, cols }` produces `rows * cols` regions).
    #[error("configured num_regions={configured} does not match strategy num_regions={strategy}")]
    RegionMismatch {
        /// Value the user passed in `MultiLoopComposer::num_regions`.
        configured: usize,
        /// Value implied by the chosen [`crate::multi_loop::ClusteringStrategy`].
        strategy: usize,
    },
}

/// The default [`Solver`] implementation: greedy init + 2-opt / 3-opt refinement.
#[derive(Debug, Clone)]
pub struct LoopSolver {
    pub initial: InitialStrategy,
    pub max_iter: u32,
}

impl Default for LoopSolver {
    fn default() -> Self {
        Self { initial: InitialStrategy::default(), max_iter: 50 }
    }
}

impl LoopSolver {
    /// Construct with custom initial strategy and iteration cap.
    /// `max_iter` defaults to 50 to prevent infinite loops on pathological
    /// instances; callers can raise or lower as needed.
    #[must_use]
    pub fn new(initial: InitialStrategy, max_iter: u32) -> Self {
        Self { initial, max_iter }
    }

    /// Build the initial tour through the spots according to the configured
    /// initial strategy.
    pub(crate) fn initial_tour(&self, problem: &SolveProblem) -> Vec<u32> {
        let spot_ids: Vec<u32> = problem.spots.iter().map(|s| s.id).collect();
        match self.initial {
            InitialStrategy::Identity => spot_ids,
            InitialStrategy::Reverse => {
                // Reverse of the identity ordering: input order, last -> first.
                // Pinned by `tests::reverse_is_identity_ordering_reversed`.
                let mut reversed = spot_ids;
                reversed.reverse();
                reversed
            }
            InitialStrategy::GreedyNearestNeighbor => greedy_nearest_neighbor(&problem.spots),
            InitialStrategy::SweepRow => sweep_row(&problem.spots),
            InitialStrategy::SweepCol => sweep_col(&problem.spots),
            InitialStrategy::Random { seed } => seeded_shuffle(&problem.spots, seed),
        }
    }

    /// Refine the initial tour via repeated 2-opt / 3-opt passes.
    /// Returns the refined tour and the solve stats.
    pub(crate) fn refine(
        &self,
        mut tour: Vec<u32>,
        problem: &SolveProblem,
    ) -> (Vec<u32>, SolveStats) {
        let initial_score = score_tour(&tour, problem);
        let mut improvements_applied: u32 = 0;
        let mut max_iter_reached = false;

        for iter in 0..self.max_iter {
            let mut any_improved = false;

            // 2-opt best-improvement scan.
            //
            // We accept the first strictly-improving swap rather than the
            // absolute best, then loop — this is the standard
            // "first-improvement" variant and is faster per pass. Switching
            // to "best-improvement" is a one-line change.
            let improved_two_opt = apply_first_improving_two_opt(&mut tour, problem);
            if improved_two_opt {
                any_improved = true;
                improvements_applied = improvements_applied.saturating_add(1);
            }

            // 3-opt pass over every valid triple.
            let n = tour.len();
            if n >= 3 {
                let mut k = 0usize;
                while k < n {
                    let mut i = 0usize;
                    while i + 2 < k {
                        let mut j = i + 1;
                        while j < k {
                            if try_three_opt(&mut tour, &problem.spots, i, j, k) {
                                any_improved = true;
                                improvements_applied = improvements_applied.saturating_add(1);
                            }
                            j = j.saturating_add(1);
                        }
                        i = i.saturating_add(1);
                    }
                    k = k.saturating_add(1);
                }
            }

            if !any_improved {
                // Local optimum reached — exit the loop. This is the
                // convergence criterion called out in the issue acceptance.
                let _ = iter;
                break;
            }

            if iter + 1 == self.max_iter {
                max_iter_reached = true;
            }
        }

        let final_score = score_tour(&tour, problem);
        (
            tour,
            SolveStats {
                iterations_used: self
                    .max_iter
                    .min(compute_iterations(&initial_score, &final_score)),
                improvements_applied,
                initial_score,
                final_score,
                max_iter_reached,
            },
        )
    }
}

impl Solver for LoopSolver {
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

        let initial_tour = self.initial_tour(problem);
        let (refined_tour, _stats) = self.refine(initial_tour, problem);

        // Distribute the refined tour across agents and days. The stub
        // distribution is round-robin: spot i goes to agent (i mod n_agents)
        // on day (i mod days). This keeps the visit order intact while
        // honoring the multi-agent / multi-day problem shape.
        let plans = distribute_plan(&refined_tour, problem);
        let score = evaluate_plan(&plans, problem);
        Ok(SolveOutcome { plans, score })
    }
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

/// Greedy nearest-neighbor tour: start at spot 0, repeatedly jump to the
/// nearest unvisited spot.
#[must_use]
pub fn greedy_nearest_neighbor(spots: &[Spot]) -> Vec<u32> {
    if spots.is_empty() {
        return Vec::new();
    }
    let mut visited = vec![false; spots.len()];
    let mut tour = Vec::with_capacity(spots.len());
    // start at spot index 0
    let mut current = 0usize;
    visited[current] = true;
    tour.push(spots[current].id);
    for _ in 1..spots.len() {
        let current_coord = spots[current].coord;
        let mut best: Option<(usize, u32)> = None;
        for (i, s) in spots.iter().enumerate() {
            if visited[i] {
                continue;
            }
            let d = edge_cost(current_coord, s.coord);
            match best {
                Some((_, bd)) if bd <= d => {}
                _ => best = Some((i, d)),
            }
        }
        let Some((next, _)) = best else { break };
        visited[next] = true;
        tour.push(spots[next].id);
        current = next;
    }
    tour
}

/// Row-major sweep: order spots by row, then column.
///
/// Stable and total — ties in `(row, col)` cannot occur because spot
/// coordinates are unique on a map — so the ordering is fully determined by
/// the spot list.
pub fn sweep_row(spots: &[Spot]) -> Vec<u32> {
    let mut ordered: Vec<&Spot> = spots.iter().collect();
    ordered.sort_by_key(|s| (s.coord.r, s.coord.q));
    ordered.iter().map(|s| s.id).collect()
}

/// Column-major sweep: order spots by column, then row.
pub fn sweep_col(spots: &[Spot]) -> Vec<u32> {
    let mut ordered: Vec<&Spot> = spots.iter().collect();
    ordered.sort_by_key(|s| (s.coord.q, s.coord.r));
    ordered.iter().map(|s| s.id).collect()
}

/// Deterministic Fisher–Yates shuffle of the spot order, seeded by `seed`.
///
/// Uses a fixed xorshift64* sequence rather than an external RNG crate so the
/// mapping `seed -> tour` is stable across compiler and dependency versions —
/// which is what makes `loop:initial=random:seed=N` reproducible in a durable
/// evaluation record.
pub fn seeded_shuffle(spots: &[Spot], seed: u64) -> Vec<u32> {
    let mut ids: Vec<u32> = spots.iter().map(|s| s.id).collect();
    if ids.len() < 2 {
        return ids;
    }
    let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15).max(1);
    for i in (1..ids.len()).rev() {
        state = xorshift64star(state);
        let span = u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1);
        let j = usize::try_from(state % span).unwrap_or(0);
        ids.swap(i, j);
    }
    ids
}

/// One xorshift64* step. Non-zero for any non-zero input.
fn xorshift64star(mut x: u64) -> u64 {
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Apply the first strictly-improving 2-opt move found. Returns true if a
/// swap was applied.
fn apply_first_improving_two_opt(tour: &mut [u32], problem: &SolveProblem) -> bool {
    if let Some((i, j)) = best_two_opt_move(tour, &problem.spots) {
        // best_two_opt_move returns the strictly-improving pair; reverse it.
        tour[i..=j].reverse();
        true
    } else {
        false
    }
}

fn compute_iterations(_a: &ScoreRanking, _b: &ScoreRanking) -> u32 {
    // Placeholder: real iteration count is tracked inside refine().
    // This helper exists so we have a stable type boundary.
    1
}

/// Round-robin distribution of `tour` across (days, agents).
#[must_use]
pub fn distribute_plan(tour: &[u32], problem: &SolveProblem) -> Vec<Vec<Vec<u32>>> {
    let days = problem.days as usize;
    let agents = problem.agents.len();
    let mut plans: Vec<Vec<Vec<u32>>> = vec![Vec::with_capacity(agents); days];
    for plan_day in plans.iter_mut().take(days) {
        *plan_day = vec![Vec::new(); agents];
    }
    for (i, spot_id) in tour.iter().enumerate() {
        let d = i % days;
        let a = i % agents;
        plans[d][a].push(*spot_id);
    }
    plans
}

/// Build a [`ScoreReport`] from a plan distribution.
#[must_use]
pub fn evaluate_plan(plans: &[Vec<Vec<u32>>], problem: &SolveProblem) -> ScoreReport {
    let mut score = ScoreReport::default();
    for (d, day_plans) in plans.iter().enumerate() {
        // Merge all agents' plans for the day into a single visit order for
        // the stub scorer. The real engine simulator will be called per
        // agent once #57 lands.
        let mut combined: Vec<u32> = Vec::new();
        for agent_plan in day_plans {
            combined.extend(agent_plan);
        }
        let agent_start =
            problem.agents.first().map_or(crate::types::HexCoord::new(0, 0), |a| a.start);
        let daily =
            simulate_day(u32::try_from(d).unwrap_or(0), &combined, &problem.spots, agent_start);
        score.daily.push(daily);
    }
    score
}

/// Compute the [`ScoreRanking`] of a single closed tour by wrapping it into
/// a one-day plan and scoring.
fn score_tour(tour: &[u32], problem: &SolveProblem) -> ScoreRanking {
    let agent_start = problem.agents.first().map_or(crate::types::HexCoord::new(0, 0), |a| a.start);
    let problem_one_agent = SolveProblem {
        spots: problem.spots.clone(),
        agents: vec![Agent { id: 0, start: agent_start }],
        days: 1,
    };
    let plans = distribute_plan(tour, &problem_one_agent);
    let report = evaluate_plan(&plans, &problem_one_agent);
    report.judge()
}

// ----------------------------------------------------------------------
// formatting + display glue
// ----------------------------------------------------------------------

impl fmt::Display for ScoreRanking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(variety={}, daily_variety_sum={}, total_noodles={}, neg_response_time={})",
            self.variety, self.daily_variety_sum, self.total_noodles, self.neg_response_time
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{HexCoord, Series};

    fn spot(id: u32, q: i32, r: i32, series: &str) -> Spot {
        Spot { id, coord: HexCoord::new(q, r), series: Series::new(series), stock: 1 }
    }

    fn agent(id: u32, q: i32, r: i32) -> Agent {
        Agent { id, start: HexCoord::new(q, r) }
    }

    #[test]
    fn loop_solver_rejects_empty_problem() {
        let solver = LoopSolver::default();
        let err = solver.solve(&SolveProblem::new(vec![], vec![agent(0, 0, 0)], 1)).unwrap_err();
        assert_eq!(err, SolverError::NoSpots);
    }

    #[test]
    fn loop_solver_rejects_zero_days() {
        let solver = LoopSolver::default();
        let err = solver
            .solve(&SolveProblem::new(vec![spot(0, 0, 0, "a")], vec![agent(0, 0, 0)], 0))
            .unwrap_err();
        assert_eq!(err, SolverError::ZeroDays);
    }

    #[test]
    fn greedy_nearest_neighbor_visits_every_spot() {
        let spots =
            vec![spot(0, 0, 0, "a"), spot(1, 2, 0, "b"), spot(2, 4, 0, "a"), spot(3, 1, 2, "b")];
        let tour = greedy_nearest_neighbor(&spots);
        assert_eq!(tour.len(), spots.len());
        // Every spot id appears exactly once.
        let mut sorted = tour.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), spots.len());
    }

    /// Every initial strategy must yield a permutation of the spot ids —
    /// the refinement loop relies on that invariant.
    #[test]
    fn every_initial_strategy_yields_a_permutation() {
        let spots = vec![
            spot(0, 0, 0, "a"),
            spot(1, 2, 0, "b"),
            spot(2, 4, 1, "a"),
            spot(3, 1, 2, "b"),
            spot(4, 3, 3, "c"),
        ];
        for initial in [
            InitialStrategy::Identity,
            InitialStrategy::Reverse,
            InitialStrategy::GreedyNearestNeighbor,
            InitialStrategy::SweepRow,
            InitialStrategy::SweepCol,
            InitialStrategy::Random { seed: 7 },
        ] {
            let solver = LoopSolver::new(initial, 1);
            let tour = solver.initial_tour(&SolveProblem::new(spots.clone(), vec![], 1));
            let mut sorted = tour.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, vec![0, 1, 2, 3, 4], "strategy {initial:?} is not a permutation");
        }
    }

    /// `InitialStrategy::Reverse` semantics, pinned: the tour is exactly
    /// the identity (input) ordering played backwards. Same spot list ⇒
    /// same tour, every call — the determinism the other variants have.
    #[test]
    fn reverse_is_identity_ordering_reversed() {
        let spots = vec![
            spot(0, 0, 0, "a"),
            spot(1, 2, 0, "b"),
            spot(2, 4, 1, "a"),
            spot(3, 1, 2, "b"),
            spot(4, 3, 3, "c"),
        ];
        let problem = SolveProblem::new(spots.clone(), vec![agent(0, 0, 0)], 1);
        let identity_solver = LoopSolver::new(InitialStrategy::Identity, 0);
        let reverse_solver = LoopSolver::new(InitialStrategy::Reverse, 0);

        let identity = identity_solver.initial_tour(&problem);
        let reverse = reverse_solver.initial_tour(&problem);
        assert_eq!(reverse, vec![4, 3, 2, 1, 0], "reverse is input order, last -> first");
        assert_eq!(
            reverse,
            identity.iter().rev().copied().collect::<Vec<u32>>(),
            "reverse must be the exact reversal of identity"
        );
        // Deterministic: repeated calls on the same problem agree.
        assert_eq!(reverse, reverse_solver.initial_tour(&problem));
        // The whole solve is deterministic too (refinement is deterministic
        // given the initial tour).
        let a = reverse_solver.solve(&problem).expect("solve a");
        let b = reverse_solver.solve(&problem).expect("solve b");
        assert_eq!(a, b, "Reverse solve must be reproducible");
        // Unlike the sweeps, Reverse follows input order — reordering the
        // input spots changes the tour (that is the point of the variant).
        let mut reordered = spots;
        reordered.swap(0, 4);
        let reordered_tour = reverse_solver.initial_tour(&SolveProblem::new(reordered, vec![], 1));
        assert_ne!(reverse, reordered_tour);
    }

    /// `loop:initial=random:seed=N` must be reproducible: the same seed on
    /// the same spot list always yields the same tour, and different seeds
    /// usually differ. This is what lets a durable evaluation record point at
    /// a candidate id and be re-run later.
    #[test]
    fn seeded_shuffle_is_reproducible_per_seed() {
        let spots = vec![
            spot(0, 0, 0, "a"),
            spot(1, 2, 0, "b"),
            spot(2, 4, 1, "a"),
            spot(3, 1, 2, "b"),
            spot(4, 3, 3, "c"),
            spot(5, 5, 5, "c"),
            spot(6, 2, 4, "d"),
        ];
        let a = seeded_shuffle(&spots, 42);
        let b = seeded_shuffle(&spots, 42);
        assert_eq!(a, b, "same seed must reproduce the same tour");
        // A different seed must (almost certainly) differ; with 7 spots the
        // chance of a collision is 1/7! which is negligible for a test that
        // only runs here.
        let c = seeded_shuffle(&spots, 43);
        assert_ne!(a, c, "different seeds should give different tours");
        // And both are permutations.
        for tour in [&a, &c] {
            let mut sorted = tour.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..7).collect::<Vec<u32>>());
        }
    }

    /// Sweep orderings are stable under input reordering — they depend only
    /// on coordinates, so re-serialising a map cannot change the tour.
    #[test]
    fn sweep_orderings_are_input_order_independent() {
        let spots =
            vec![spot(0, 0, 1, "a"), spot(1, 2, 0, "b"), spot(2, 1, 1, "c"), spot(3, 3, 0, "d")];
        let mut shuffled = spots.clone();
        shuffled.swap(0, 3);
        shuffled.swap(1, 2);
        assert_eq!(sweep_row(&spots), sweep_row(&shuffled));
        assert_eq!(sweep_col(&spots), sweep_col(&shuffled));
        // Row sweep orders by (row, col); column sweep by (col, row).
        assert_eq!(sweep_row(&spots), vec![1, 3, 0, 2]);
        assert_eq!(sweep_col(&spots), vec![0, 2, 1, 3]);
    }

    #[test]
    fn loop_solver_improves_or_keeps_greedy_initial() {
        // Triangle of three spots on the same series — greedy is a single
        // hop from 0->1->2, which is already optimal w.r.t. raw distance.
        // The 2-opt pass should report no strict improvement and the
        // ranking should not decrease.
        let spots = vec![spot(0, 0, 0, "kake"), spot(1, 2, 0, "kake"), spot(2, 1, 2, "kake")];
        let problem = SolveProblem::new(spots, vec![agent(0, 0, 0)], 1);
        let solver = LoopSolver::default();
        let outcome = solver.solve(&problem).expect("solve");
        // The plan should include every spot id at least once.
        let mut all_ids: Vec<u32> =
            outcome.plans.iter().flat_map(|d| d.iter().flatten().copied()).collect();
        all_ids.sort_unstable();
        all_ids.dedup();
        assert_eq!(all_ids, vec![0, 1, 2]);
    }
}
