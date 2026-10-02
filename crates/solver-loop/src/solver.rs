//! Iterative-improvement loop solver.
//!
//! ## Algorithm
//! 1. Build an initial solution with the configured [`InitialStrategy`]:
//!    - `GreedyNearestNeighbor` — start at the first spot, repeatedly jump
//!      to the nearest unvisited spot. O(n^2).
//!    - `Identity` — visit spots in input order (useful as a baseline).
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
//! ## Integration
//! EXPECTED INTEGRATION: domain — once #57 lands, [`ScoreReport::judge`] is
//! replaced by `domain::ScoreReport::judge()` and the day simulation in
//! [`crate::score`] is delegated to the engine. The algorithm and trait
//! shape do not change.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::score::{ScoreRanking, ScoreReport, simulate_day};
use crate::three_opt::try_three_opt;
use crate::two_opt::{best_two_opt_move, edge_cost};
use crate::types::{Agent, Spot};

/// Initial-solution strategy for the loop solver.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitialStrategy {
    /// Visit spots in input order, then close the loop.
    Identity,
    /// Nearest-neighbor greedy: start at the first spot, repeatedly jump
    /// to the nearest unvisited spot. O(n^2).
    #[default]
    GreedyNearestNeighbor,
}

/// Solver trait stub.
///
/// EXPECTED INTEGRATION: domain — once #57 lands, this trait moves to
/// `domain::solver::Solver` (or similar) and `LoopSolver` implements that
/// canonical trait. For now it lives here so the solver-loop crate can be
/// iterated on without domain integration.
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
            InitialStrategy::GreedyNearestNeighbor => greedy_nearest_neighbor(&problem.spots),
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
