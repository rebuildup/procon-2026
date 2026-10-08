//! [`LocalSearchSolver`] — top-level driver that combines
//! [`crate::types::greedy_route`], [`crate::neighbors`], and
//! [`crate::annealing`] into a single iteration budget.
//!
//! The solver is intentionally minimal: it owns the schedule, the RNG,
//! and the local stub types. The result is a [`crate::types::Solution`]
//! with concrete [`Action`]s (currently `Stay` / `Move` / `Take` derived
//! from the visited spot sequence) and the achieved [`DayScore`].

use serde::{Deserialize, Serialize};

use crate::annealing::{Schedule, accept_score};
use crate::neighbors::{self, NeighborhoodRng};
use crate::types::{Action, DayScore, Instance, Solution, evaluate_route, greedy_route};

/// Solver configuration. Exposes the three knobs the issue mandates
/// (`max_iter`, `T_init`, `T_min`) plus the optional seed for
/// determinism in tests.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalSearchConfig {
    pub max_iter: u32,
    pub t_init: f64,
    pub t_min: f64,
    /// RNG seed. When `None` the solver draws one from the clock — useful
    /// for production runs but never in tests.
    pub seed: Option<u64>,
}

impl LocalSearchConfig {
    /// Construct a config with a deterministic seed. Convenience for tests
    /// and CLI users that want reproducibility.
    #[must_use]
    pub fn deterministic(max_iter: u32, t_init: f64, t_min: f64, seed: u64) -> Self {
        Self { max_iter, t_init, t_min, seed: Some(seed) }
    }

    fn schedule(&self) -> Schedule {
        Schedule::new(self.t_init, self.t_min, self.max_iter)
    }
}

/// Local-search solver. Implements the local-stub [`Solver`] trait — once
/// `crates/domain` provides a real `Solver` interface this struct will
/// either gain a `domain::Solver` impl via newtype or be wrapped by an
/// adapter; the public API stays stable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalSearchSolver {
    pub config: LocalSearchConfig,
}

impl LocalSearchSolver {
    #[must_use]
    pub const fn new(config: LocalSearchConfig) -> Self {
        Self { config }
    }

    /// Improve `initial_route` (or a fresh greedy route when `None`) on
    /// `instance` using simulated annealing with the configured
    /// neighbor alphabet.
    #[must_use]
    pub fn improve(&self, instance: &Instance, initial_route: Option<Vec<u32>>) -> Solution {
        let mut route = initial_route.unwrap_or_else(|| greedy_route(instance));
        let mut best_route = route.clone();
        let mut best_score = evaluate_route(&best_route, instance);
        let mut current_score = best_score;

        let schedule = self.config.schedule();
        let mut rng = if let Some(s) = self.config.seed {
            NeighborhoodRng::from_seed(s)
        } else {
            // Tiny fallback entropy pool — sufficient for SA where we
            // only need unique-ish 64-bit seeds. Truncating to u64 is
            // intentional: the XOR-mix below scrambles any structure.
            #[allow(
                clippy::cast_possible_truncation,
                reason = "nanos since UNIX_EPOCH fit in 64 bits until year ~2554"
            )]
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0u64, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
            NeighborhoodRng::from_seed(nanos ^ 0x9E37_79B9_7F4A_7C15)
        };

        for iter in 0..self.config.max_iter {
            if route.len() < 2 {
                break;
            }
            let Some(mv) = neighbors::propose(&route, &mut rng) else {
                break;
            };
            neighbors::apply(&mut route, mv);
            let new_score = evaluate_route(&route, instance);
            // Score delta: positive = new is worse, negative = new is better.
            // We use this convention directly via `accept_score`.
            let delta = score_delta(new_score, current_score);
            let t = schedule.temperature(iter);
            if accept_score(delta, t, &mut rng) {
                current_score = new_score;
                if current_score > best_score {
                    best_score = current_score;
                    best_route.clone_from(&route);
                }
            } else {
                // Roll back. `apply` mutates in place, so the inverse
                // operator undoes the move exactly.
                neighbors::apply(&mut route, neighbors::inverse(mv));
            }
        }

        Solution { actions: route_to_actions(&best_route, instance), score: best_score }
    }
}

impl Solver for LocalSearchSolver {
    type Output = Solution;

    fn decide(&self, instance: &Instance) -> Self::Output {
        self.improve(instance, None)
    }
}

/// Local-stub Solver trait. Mirrors the shape `crates/bench` (#20) will
/// require; once the bench harness lands this trait moves into a shared
/// `solver-api` crate and `LocalSearchSolver` keeps its impl verbatim.
pub trait Solver {
    type Output;

    fn decide(&self, instance: &Instance) -> Self::Output;
}

/// Compute a signed delta in SA score space: `new - old`. Positive means
/// the new solution is worse (because [`annealing::accept_score`] expects
/// "worse = positive"), negative means better.
fn score_delta(new: DayScore, old: DayScore) -> i64 {
    // Use the same ranking ordering the contest applies — distinct series
    // first, then cumulative, then balls. We express the delta in units of
    // the lowest-priority field to keep the magnitude in a sane range for
    // `exp(delta/T)`.
    let ds = i64::from(new.distinct_series) - i64::from(old.distinct_series);
    let cd = i64::from(new.cumulative_distinct) - i64::from(old.cumulative_distinct);
    let tb = i64::from(new.total_balls) - i64::from(old.total_balls);
    // Weight: distinct series dominates, but at the SA level we keep the
    // base magnitude small enough that `exp(delta/T)` is numerically stable
    // for T in [1e-3, 1e3].
    ds * 1_000_000 + cd * 10_000 + tb
}

/// Convert a route (spot ids) into a concrete [`Action`] trace. We emit
/// one `Move` per spot on the way to the spot and a `Take` on arrival.
/// This is intentionally a coarse expansion — the real engine will
/// substitute terrain-aware moves and fuel accounting; here we use the
/// spot's coordinate as the destination of the `Move` so the action list
/// is self-describing for downstream stages.
///
/// Crate-visible so that sibling solver drivers (e.g. [`crate::simulated_annealing`])
/// can reuse the same expansion without duplicating the route-to-actions
/// mapping.
pub(crate) fn route_to_actions(route: &[u32], instance: &Instance) -> Vec<Action> {
    let mut actions: Vec<Action> = Vec::with_capacity(route.len() * 2);
    let mut budget = instance.agent.step_budget;
    for &spot_id in route {
        let Some(spot) = instance.spots.iter().find(|s| s.id == spot_id) else {
            continue;
        };
        if budget == 0 {
            break;
        }
        actions.push(Action::Move(spot.coord));
        actions.push(Action::Take { spot: spot_id, ball_index: 0 });
        budget = budget.saturating_sub(1);
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Agent, HexCoord, Series, Spot};

    fn fixture() -> Instance {
        // Two clusters: a "near" series A and a "far" series B. Greedy
        // visits A first, then B — the optimal route is the same here, but
        // we exercise the SA loop on a non-trivial instance.
        let spots = vec![
            Spot {
                id: 1,
                coord: HexCoord::new(0, 1),
                series: Series(0),
                remaining: 2,
                max_stock: 2,
            },
            Spot {
                id: 2,
                coord: HexCoord::new(1, 0),
                series: Series(0),
                remaining: 1,
                max_stock: 1,
            },
            Spot {
                id: 3,
                coord: HexCoord::new(0, 5),
                series: Series(1),
                remaining: 3,
                max_stock: 3,
            },
            Spot {
                id: 4,
                coord: HexCoord::new(1, 6),
                series: Series(1),
                remaining: 1,
                max_stock: 1,
            },
        ];
        let agent = Agent { id: 0, coord: HexCoord::new(0, 0), fuel_budget: 100, step_budget: 50 };
        Instance::new(spots, agent)
    }

    #[test]
    fn solver_runs_and_returns_a_solution() {
        let solver = LocalSearchSolver::new(LocalSearchConfig::deterministic(200, 10.0, 0.1, 1));
        let sol = solver.decide(&fixture());
        // Score must be non-degenerate: at least the greedy outcome.
        assert!(sol.score.distinct_series >= 1);
        assert_ne!(sol.actions.len(), 0);
    }

    #[test]
    fn solver_beats_or_matches_greedy_baseline() {
        let solver = LocalSearchSolver::new(LocalSearchConfig::deterministic(500, 5.0, 0.01, 42));
        let instance = fixture();
        let greedy_route = greedy_route(&instance);
        let greedy_score = evaluate_route(&greedy_route, &instance);
        let sol = solver.improve(&instance, Some(greedy_route.clone()));
        assert!(sol.score >= greedy_score, "local search regressed: {sol:?} < {greedy_score:?}");
    }

    #[test]
    fn solver_determinism_for_same_seed() {
        let cfg = LocalSearchConfig::deterministic(300, 8.0, 0.05, 123);
        let solver = LocalSearchSolver::new(cfg);
        let a = solver.decide(&fixture());
        let b = solver.decide(&fixture());
        assert_eq!(a.actions, b.actions);
        assert_eq!(a.score, b.score);
    }

    #[test]
    fn empty_instance_returns_empty_solution() {
        let solver = LocalSearchSolver::new(LocalSearchConfig::deterministic(10, 1.0, 0.1, 0));
        let instance = Instance::new(
            Vec::new(),
            Agent { id: 0, coord: HexCoord::new(0, 0), fuel_budget: 0, step_budget: 0 },
        );
        let sol = solver.decide(&instance);
        assert_eq!(sol.actions.len(), 0);
        assert_eq!(sol.score, DayScore::default());
    }
}
