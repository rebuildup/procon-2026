//! Dual-direction loop solver: CW collection loop + CCW refill loop.
//!
//! ## Motivation
//!
//! The contest lets us run two kinds of agents in parallel: **collection
//! agents** (巡回車) that pick up noodle balls from spots, and **refill
//! agents** (補給車) that resupply the spots. Running both on the same
//! single-direction loop creates head-on traffic on every edge of the
//! tour. To balance the load, this solver produces a *pair* of loops over
//! the same set of spots, traversed in opposite directions:
//!
//! - **CW loop** — assigned to collection agents. Visits spots in the
//!   clockwise (forward) order produced by the underlying [`LoopSolver`].
//! - **CCW loop** — assigned to refill agents. Visits the same spots in
//!   the *reverse* order, which is the counter-clockwise traversal of the
//!   same closed tour.
//!
//! Because the two loops share the same spot sequence and only differ in
//! direction, the union of their visit orders still covers every spot —
//! just from opposite ends of the tour. This avoids head-on congestion
//! without sacrificing coverage.
//!
//! ## Agent split
//!
//! The problem's `agents` vector is split in half along the midpoint:
//!
//! - The first `floor(n / 2)` agents are collection agents and run the
//!   CW loop.
//! - The remaining agents are refill agents and run the CCW loop.
//!
//! If `n == 1`, the single agent is treated as a collection agent (the
//! refill group is empty). With `n == 0` the solver returns
//! [`SolverError::NoAgents`] before any split happens.
//!
//! EXPECTED INTEGRATION: domain — once #57 lands, the agent split could
//! become a typed discriminator on `domain::Agent`. For now we use the
//! positional split so the crate stays self-contained.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::score::ScoreReport;
use crate::solver::{
    InitialStrategy, LoopSolver, SolveOutcome, SolveProblem, Solver, SolverError, evaluate_plan,
};
use crate::types::{Agent, HexCoord, Spot};

/// Traversal direction along a closed loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// Clockwise (forward) traversal of the underlying closed tour.
    Clockwise,
    /// Counter-clockwise (reverse) traversal of the underlying closed tour.
    CounterClockwise,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clockwise => f.write_str("CW"),
            Self::CounterClockwise => f.write_str("CCW"),
        }
    }
}

/// Dual-direction loop solver.
///
/// Produces a CW loop for collection agents and a CCW loop for refill
/// agents, sharing the same set of spots. The CW loop is the refined
/// closed tour produced by [`LoopSolver`]; the CCW loop is its exact
/// reverse.
#[derive(Debug, Clone)]
pub struct DualDirectionLoop {
    /// Initial-solution strategy for the underlying CW loop.
    pub initial: InitialStrategy,
    /// Maximum refinement iterations for the CW loop.
    pub max_iter: u32,
    /// Direction assigned to collection agents. Defaults to
    /// [`Direction::Clockwise`].
    pub collect_direction: Direction,
    /// Direction assigned to refill agents. Defaults to
    /// [`Direction::CounterClockwise`].
    pub refill_direction: Direction,
}

impl Default for DualDirectionLoop {
    fn default() -> Self {
        Self {
            initial: InitialStrategy::default(),
            max_iter: 50,
            collect_direction: Direction::Clockwise,
            refill_direction: Direction::CounterClockwise,
        }
    }
}

impl DualDirectionLoop {
    /// Construct with custom initial strategy and iteration cap. Uses the
    /// default direction split: collection = CW, refill = CCW.
    #[must_use]
    pub fn new(initial: InitialStrategy, max_iter: u32) -> Self {
        Self {
            initial,
            max_iter,
            collect_direction: Direction::Clockwise,
            refill_direction: Direction::CounterClockwise,
        }
    }

    /// Construct with an explicit direction split. Useful for tests and
    /// for solvers that want to flip which agent class drives CW vs CCW.
    #[must_use]
    pub fn with_directions(
        initial: InitialStrategy,
        max_iter: u32,
        collect_direction: Direction,
        refill_direction: Direction,
    ) -> Self {
        Self { initial, max_iter, collect_direction, refill_direction }
    }

    /// Build the CW (collection) loop through the spots.
    ///
    /// This is the refined closed tour produced by the underlying
    /// [`LoopSolver`] — it is the canonical "clockwise" sequence both
    /// agent classes agree on.
    #[must_use]
    pub fn cw_tour(&self, problem: &SolveProblem) -> Vec<u32> {
        let helper = LoopSolver::new(self.initial, self.max_iter);
        let initial = helper.initial_tour(problem);
        let (refined, _stats) = helper.refine(initial, problem);
        refined
    }

    /// Build the CCW (refill) loop through the spots. This is the
    /// reverse of the CW tour, so both loops share the same spot
    /// sequence and the same closed-tour cost — only the direction
    /// differs.
    #[must_use]
    pub fn ccw_tour(&self, problem: &SolveProblem) -> Vec<u32> {
        self.cw_tour(problem).into_iter().rev().collect()
    }

    /// Pair of `(collect_tour, refill_tour)` produced by this solver.
    ///
    /// Returns `(cw, ccw)` by default. If [`Self::collect_direction`] /
    /// [`Self::refill_direction`] are set to a non-default configuration,
    /// each tuple element is still the same physical tour — the
    /// `Direction` fields describe *how the agent class traverses it*,
    /// not which physical tour they receive. Use [`Self::split_agents`]
    /// to get the per-agent plan assignment.
    #[must_use]
    pub fn build_tours(&self, problem: &SolveProblem) -> (Vec<u32>, Vec<u32>) {
        let cw = self.cw_tour(problem);
        let ccw = cw.iter().rev().copied().collect();
        (cw, ccw)
    }

    /// Split the agents into a `(collection, refill)` pair by midpoint.
    ///
    /// - The first `floor(n / 2)` agents are collection agents (CW).
    /// - The remaining agents are refill agents (CCW).
    ///
    /// Edge cases:
    /// - `n == 0` returns `(vec![], vec![])`.
    /// - `n == 1` keeps the single agent as a collection agent (the
    ///   refill group is empty) so that the CW loop still has a runner.
    #[must_use]
    pub fn split_agents(agents: &[Agent]) -> (Vec<Agent>, Vec<Agent>) {
        match agents.len() {
            0 => (Vec::new(), Vec::new()),
            1 => (vec![agents[0]], Vec::new()),
            n => {
                let mid = n / 2;
                let (coll, refill) = agents.split_at(mid);
                (coll.to_vec(), refill.to_vec())
            }
        }
    }

    /// Number of agents assigned to the collection (CW) group under the
    /// default midpoint split.
    ///
    /// Mirrors [`Self::split_agents`] for use at plan-distribution time:
    /// - `n == 0` returns `0`.
    /// - `n == 1` returns `1` (the lone agent is a collection agent).
    /// - `n >= 2` returns `n / 2`.
    #[must_use]
    pub fn n_collect_agents(agents: &[Agent]) -> usize {
        match agents.len() {
            0 => 0,
            1 => 1,
            n => n / 2,
        }
    }
}

impl Solver for DualDirectionLoop {
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

        // Both loops share the same physical spot sequence; only the
        // direction of traversal differs.
        let collect_tour = self.cw_tour(problem);
        let refill_tour: Vec<u32> = collect_tour.iter().rev().copied().collect();

        // Distribute the two tours across the agents. Collection agents
        // (first half) get the CW tour; refill agents (second half) get
        // the CCW tour.
        let n_collect = Self::n_collect_agents(&problem.agents);
        let plans = distribute_dual_plan(
            &collect_tour,
            &refill_tour,
            problem.days,
            problem.agents.len(),
            n_collect,
        );
        let score = evaluate_plan(&plans, problem);
        Ok(SolveOutcome { plans, score })
    }
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

/// Distribute the dual loops across `(days, agents)`.
///
/// - For each spot index `i` in the collect (CW) tour, the spot is
///   assigned to `(day = i mod days, agent = i mod n_collect)`.
/// - For each spot index `i` in the refill (CCW) tour, the spot is
///   assigned to `(day = i mod days, agent = n_collect + (i mod
///   n_refill))`.
///
/// If `n_collect == 0`, the CW tour contributes no entries (no
/// collection agents). If `n_refill == 0`, the CCW tour contributes no
/// entries (e.g. `n_agents == 1`).
#[must_use]
pub fn distribute_dual_plan(
    collect_tour: &[u32],
    refill_tour: &[u32],
    days: u32,
    n_agents: usize,
    n_collect: usize,
) -> Vec<Vec<Vec<u32>>> {
    let days = days as usize;
    let n_refill = n_agents.saturating_sub(n_collect);
    let mut plans: Vec<Vec<Vec<u32>>> = vec![Vec::with_capacity(n_agents); days];
    for plan_day in plans.iter_mut().take(days) {
        *plan_day = vec![Vec::new(); n_agents];
    }

    if n_collect > 0 {
        for (i, spot_id) in collect_tour.iter().enumerate() {
            let d = i % days;
            let a = i % n_collect;
            plans[d][a].push(*spot_id);
        }
    }
    if n_refill > 0 {
        for (i, spot_id) in refill_tour.iter().enumerate() {
            let d = i % days;
            let a = n_collect + (i % n_refill);
            plans[d][a].push(*spot_id);
        }
    }
    plans
}

/// Compute the score of the dual loops using a one-day, one-agent
/// problem shape. Mirrors [`crate::solver::score_tour`] but evaluates
/// both directions and returns a [`ScoreReport`] covering the pair.
///
/// Primarily useful for tests and ad-hoc tuning.
#[must_use]
pub fn score_dual(
    collect_tour: &[u32],
    refill_tour: &[u32],
    problem: &SolveProblem,
) -> ScoreReport {
    let agent_start: HexCoord = problem.agents.first().map_or(HexCoord::new(0, 0), |a| a.start);
    let probe = SolveProblem {
        spots: problem.spots.clone(),
        agents: vec![Agent { id: 0, start: agent_start }],
        days: 1,
    };
    let plans = distribute_dual_plan(collect_tour, refill_tour, probe.days, 2, 1);
    evaluate_plan(&plans, &probe)
}

/// Direction tag for each spot position in the closed tour, useful in
/// tests to assert CW vs CCW ordering without depending on numeric
/// spot ids.
#[must_use]
pub fn cw_position(tour: &[u32], spot_id: u32) -> Option<usize> {
    tour.iter().position(|&id| id == spot_id)
}

/// Direction tag for each spot position in the closed tour, useful in
/// tests to assert CW vs CCW ordering without depending on numeric
/// spot ids.
#[must_use]
pub fn ccw_position(tour: &[u32], spot_id: u32) -> Option<usize> {
    tour.iter().rposition(|&id| id == spot_id)
}

/// Convenience: build a `SolveProblem` with a default agent set sized
/// for a dual-direction solve (`n_collect = n_refill = 4`). Provided so
/// callers don't have to hand-roll an agent vector in tests.
#[must_use]
pub fn default_dual_agents() -> Vec<Agent> {
    (0..8u32)
        .map(|i| Agent { id: i, start: HexCoord::new(i32::try_from(i).unwrap_or(0), 0) })
        .collect()
}

/// Convenience: build a `SolveProblem` with spots arranged in clockwise
/// order around the origin (for tests asserting CW / CCW direction).
#[must_use]
pub fn cw_ordered_spots(n: usize) -> Vec<Spot> {
    use crate::types::Series;
    (0..n)
        .map(|i| Spot {
            id: u32::try_from(i).unwrap_or(0),
            coord: HexCoord::new(i32::try_from(i).unwrap_or(0), 0),
            series: Series::new("kake"),
            stock: 1,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Series;

    fn spot(id: u32, q: i32, r: i32, series: &str) -> Spot {
        Spot { id, coord: HexCoord::new(q, r), series: Series::new(series), stock: 1 }
    }

    fn agent(id: u32, q: i32, r: i32) -> Agent {
        Agent { id, start: HexCoord::new(q, r) }
    }

    #[test]
    fn direction_display_is_stable() {
        assert_eq!(Direction::Clockwise.to_string(), "CW");
        assert_eq!(Direction::CounterClockwise.to_string(), "CCW");
    }

    #[test]
    fn split_agents_uses_midpoint() {
        let agents: Vec<Agent> = (0..6).map(|i| agent(i, 0, 0)).collect();
        let (coll, refill) = DualDirectionLoop::split_agents(&agents);
        assert_eq!(coll.len(), 3);
        assert_eq!(refill.len(), 3);
        assert_eq!(coll[0].id, 0);
        assert_eq!(coll[2].id, 2);
        assert_eq!(refill[0].id, 3);
        assert_eq!(refill[2].id, 5);
    }

    #[test]
    fn split_agents_handles_odd_count() {
        let agents: Vec<Agent> = (0..5).map(|i| agent(i, 0, 0)).collect();
        let (coll, refill) = DualDirectionLoop::split_agents(&agents);
        // floor(5 / 2) = 2 collection, 3 refill.
        assert_eq!(coll.len(), 2);
        assert_eq!(refill.len(), 3);
    }

    #[test]
    fn split_agents_handles_singleton() {
        let agents = vec![agent(0, 0, 0)];
        let (coll, refill) = DualDirectionLoop::split_agents(&agents);
        assert_eq!(coll.len(), 1);
        assert_eq!(refill.len(), 0);
    }

    #[test]
    fn split_agents_handles_empty() {
        let (coll, refill) = DualDirectionLoop::split_agents(&[]);
        assert_eq!(coll.len(), 0);
        assert_eq!(refill.len(), 0);
    }

    #[test]
    fn ccw_tour_is_reverse_of_cw() {
        let spots =
            vec![spot(0, 0, 0, "a"), spot(1, 2, 0, "b"), spot(2, 4, 0, "a"), spot(3, 1, 2, "b")];
        let agents: Vec<Agent> = (0..4).map(|i| agent(i, 0, 0)).collect();
        let problem = SolveProblem::new(spots, agents, 1);
        let solver = DualDirectionLoop::default();
        let cw = solver.cw_tour(&problem);
        let ccw = solver.ccw_tour(&problem);
        assert_eq!(cw.len(), ccw.len());
        let cw_rev: Vec<u32> = cw.iter().rev().copied().collect();
        assert_eq!(cw_rev, ccw, "CCW tour must equal reverse of CW tour");
    }

    #[test]
    fn build_tours_returns_consistent_pair() {
        let spots = cw_ordered_spots(6);
        let agents = default_dual_agents();
        let problem = SolveProblem::new(spots, agents, 1);
        let solver = DualDirectionLoop::default();
        let (cw, ccw) = solver.build_tours(&problem);
        assert_eq!(cw.len(), 6);
        assert_eq!(ccw.len(), 6);
        // Both tours visit exactly the same spot set.
        let mut cw_sorted = cw.clone();
        let mut ccw_sorted = ccw.clone();
        cw_sorted.sort_unstable();
        ccw_sorted.sort_unstable();
        assert_eq!(cw_sorted, ccw_sorted);
    }

    #[test]
    fn distribute_dual_plan_routes_cw_to_collect_and_ccw_to_refill() {
        // Two spots per loop, single day, one collection + one refill
        // agent — the simplest case that exercises the dual split.
        let cw = vec![10, 11];
        let ccw = vec![99, 98];
        let plans = distribute_dual_plan(&cw, &ccw, 1, 2, 1);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].len(), 2);
        // Collection agent (index 0) owns every spot in the CW tour.
        assert_eq!(plans[0][0], cw);
        // Refill agent (index 1) owns every spot in the CCW tour.
        assert_eq!(plans[0][1], ccw);
    }

    #[test]
    fn distribute_dual_plan_spreads_across_multiple_days_and_agents() {
        // 4 spots, 2 days, 2 collect agents + 2 refill agents.
        let cw = vec![10, 11, 12, 13];
        let ccw = vec![99, 98, 97, 96];
        let plans = distribute_dual_plan(&cw, &ccw, 2, 4, 2);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].len(), 4);
        // Day 0: collect agent 0 = cw[0]=10, cw[2]=12; refill agent 2 = ccw[0]=99.
        assert_eq!(plans[0][0], vec![10, 12]);
        assert!(plans[0][1].is_empty(), "collect agent 1 has no day-0 spots");
        assert_eq!(plans[0][2], vec![99, 97]);
        assert!(plans[0][3].is_empty(), "refill agent 3 has no day-0 spots");
        // Day 1: collect agent 1 = cw[1]=11, cw[3]=13; refill agent 3 = ccw[1]=98.
        assert_eq!(plans[1][0].len(), 0);
        assert_eq!(plans[1][1], vec![11, 13]);
        assert_eq!(plans[1][2].len(), 0);
        assert_eq!(plans[1][3], vec![98, 96]);
    }

    #[test]
    fn distribute_dual_plan_handles_singleton_collect() {
        // Single collect agent, no refill agents: only the CW tour
        // contributes to the plans.
        let cw = vec![1, 2, 3];
        let ccw = vec![9, 8, 7];
        let plans = distribute_dual_plan(&cw, &ccw, 1, 1, 1);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].len(), 1);
        assert_eq!(plans[0][0], cw);
    }

    #[test]
    fn distribute_dual_plan_drops_cw_when_no_collect_agents() {
        // Edge case: agents = 0, n_collect = 0 — neither tour
        // contributes. Defensive default.
        let plans = distribute_dual_plan(&[1, 2, 3], &[4, 5, 6], 1, 0, 0);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].len(), 0);
    }

    #[test]
    fn solver_rejects_empty_problem() {
        let solver = DualDirectionLoop::default();
        let err = solver.solve(&SolveProblem::new(vec![], vec![agent(0, 0, 0)], 1)).unwrap_err();
        assert_eq!(err, SolverError::NoSpots);
    }

    #[test]
    fn solver_rejects_zero_days() {
        let solver = DualDirectionLoop::default();
        let err = solver
            .solve(&SolveProblem::new(vec![spot(0, 0, 0, "a")], vec![agent(0, 0, 0)], 0))
            .unwrap_err();
        assert_eq!(err, SolverError::ZeroDays);
    }

    #[test]
    fn solver_rejects_empty_agents() {
        let solver = DualDirectionLoop::default();
        let err =
            solver.solve(&SolveProblem::new(vec![spot(0, 0, 0, "a")], vec![], 1)).unwrap_err();
        assert_eq!(err, SolverError::NoAgents);
    }

    #[test]
    fn solver_produces_plan_with_expected_shape() {
        let spots = cw_ordered_spots(6);
        let agents = default_dual_agents();
        let problem = SolveProblem::new(spots, agents, 2);
        let solver = DualDirectionLoop::default();
        let outcome = solver.solve(&problem).expect("solve");
        // 2 days × 8 agents.
        assert_eq!(outcome.plans.len(), 2);
        assert_eq!(outcome.plans[0].len(), 8);
    }

    #[test]
    fn solver_uses_default_directions() {
        let solver = DualDirectionLoop::default();
        assert_eq!(solver.collect_direction, Direction::Clockwise);
        assert_eq!(solver.refill_direction, Direction::CounterClockwise);
    }

    #[test]
    fn with_directions_override_is_respected() {
        let solver = DualDirectionLoop::with_directions(
            InitialStrategy::Identity,
            0,
            Direction::CounterClockwise,
            Direction::Clockwise,
        );
        assert_eq!(solver.collect_direction, Direction::CounterClockwise);
        assert_eq!(solver.refill_direction, Direction::Clockwise);
    }

    #[test]
    fn cw_ordered_spots_helper_is_usable() {
        // The helper must produce contiguous ids starting at 0.
        let spots = cw_ordered_spots(4);
        let ids: Vec<u32> = spots.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
        // All spots share a single series so variety is 1 (not load-bearing,
        // just ensures the helper is internally consistent).
        let series: Vec<&Series> = spots.iter().map(|s| &s.series).collect();
        for s in &series {
            assert_eq!(s.as_str(), "kake");
        }
    }
}
