//! MCTS skeleton for day-step planning.
//!
//! Implements the Phase 3 solver from `docs/contest/strategy.md` §5.3:
//! Monte Carlo Tree Search constrained by `daySeconds`, with UCB1 selection,
//! greedy + ε-random expansion, and a variety-aware heuristic rollout.
//!
//! The [`Solver`] trait is a local stub; issue #57 lands the canonical
//! solver trait in the domain layer. The stub matches the expected shape so
//! migration is mechanical once #57 merges.

#![doc(test(attr(deny(warnings))))]

pub mod multi_agent;
pub mod node;
pub mod rollout;
pub mod solver;
pub mod types;

pub use multi_agent::{AgentId, JointAction, MultiAgentError, MultiAgentMCTS, MultiAgentNode};
pub use node::MctsNode;
pub use rollout::{
    GreedyRollout, NearestSpotHeuristic, RolloutError, RolloutOutcome, RolloutPolicy, rollout,
};
pub use solver::{MctsError, MctsSolver};
pub use types::{Action, DayState, MAX_ACTIONS, PlanError, plan_one_step};

use std::time::Duration;

/// Tunable parameters for the MCTS solver.
///
/// Defaults track `docs/contest/strategy.md` §5.3 (Phase 3): enough iterations
/// to find a good child within `daySeconds`, with the classical UCT constant
/// `√2` and a rollout depth that comfortably covers a typical day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MctsConfig {
    /// Maximum number of `select -> expand -> rollout -> backprop` cycles.
    pub iterations: u32,
    /// UCB1 exploration constant (UCT-style `c`). Defaults to `√2`.
    pub explore_constant: f64,
    /// Maximum rollout depth (steps simulated per rollout).
    pub rollout_depth: u32,
    /// Probability of picking a random legal action during expansion / rollout
    /// (ε-random). The remaining mass picks the greedy best.
    pub epsilon: f64,
    /// Optional wall-clock budget. When `Some`, the search stops as soon as
    /// the budget is exhausted, even if `iterations` have not been reached.
    pub time_budget: Option<Duration>,
    /// Seed for the deterministic PRNG. `0` derives a fresh stream from the
    /// clock; any non-zero value is reproducible.
    pub rng_seed: u64,
}

impl Default for MctsConfig {
    fn default() -> Self {
        Self {
            iterations: 1_000,
            explore_constant: core::f64::consts::SQRT_2,
            rollout_depth: 30,
            epsilon: 0.1,
            time_budget: None,
            rng_seed: 0,
        }
    }
}

/// Solver trait — local stub pending #57.
///
/// The canonical trait will be defined on the domain side; this local
/// definition exists so `MctsSolver` is testable in isolation.
pub trait Solver {
    /// The state handed to the solver by the match runner / engine.
    type State;

    /// The discrete action the solver commits to for the current step.
    type Action;

    /// Error type when planning is not possible (empty action set, terminal
    /// state, invariant violation, …).
    type Error;

    /// Pick the single next step's action from the current state.
    fn plan_step(
        &self,
        state: &Self::State,
        time_budget: Duration,
    ) -> Result<Self::Action, Self::Error>;
}
