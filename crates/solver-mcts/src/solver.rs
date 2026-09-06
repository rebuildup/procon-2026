//! `MctsSolver` — pluggable day-step planner.
//!
//! Each call to [`MctsSolver::plan_step`] runs an MCTS search confined to
//! the `time_budget`. The returned action is the most-visited child of
//! the root (a "robust child" choice with lower variance than max-mean).
//!
//! Loop shape:
//!
//! 1. **Select** a root child with UCB1, or expand a new one if any
//!    legal action is still un-visited.
//! 2. **Expand**: the selected / freshly-picked action becomes a child
//!    of the root representing "1 step ahead". The expansion policy is
//!    `greedy + ε-random` so the solver still explores.
//! 3. **Rollout** from the child's state with the variety-aware policy in
//!    [`crate::rollout`].
//! 4. **Backprop** the rollout reward up to the root (single hop, because
//!    the day-step tree is one level deep).
//!
//! Reference: `docs/contest/strategy.md` §5.3 Phase 3.

use std::time::{Duration, Instant};

use crate::node::MctsNode;
use crate::rollout::{Rng, rollout};
use crate::types::{Action, DayState, PlanError};
use crate::{MctsConfig, Solver};

/// Default implementation of the [`Solver`] trait backed by MCTS.
#[derive(Debug, Clone)]
pub struct MctsSolver {
    /// Tunable parameters. See [`MctsConfig`].
    pub config: MctsConfig,
}

/// Solver-specific errors.
///
/// Wraps [`PlanError`] so the [`Solver`] trait stays decoupled from the
/// concrete state representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MctsError {
    /// The state admits no legal action.
    EmptyActionSet,
    /// The caller passed a zero time budget.
    ZeroBudget,
}

impl std::fmt::Display for MctsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyActionSet => f.write_str("no legal actions available"),
            Self::ZeroBudget => f.write_str("zero time budget passed to solver"),
        }
    }
}

impl std::error::Error for MctsError {}

impl From<PlanError> for MctsError {
    fn from(e: PlanError) -> Self {
        match e {
            PlanError::EmptyActionSet => Self::EmptyActionSet,
            PlanError::ZeroBudget => Self::ZeroBudget,
        }
    }
}

impl Solver for MctsSolver {
    type State = DayState;
    type Action = Action;
    type Error = MctsError;

    fn plan_step(
        &self,
        state: &Self::State,
        time_budget: Duration,
    ) -> Result<Self::Action, Self::Error> {
        let picked = self.run(state, time_budget)?;
        Ok(picked.action)
    }
}

impl MctsSolver {
    /// Construct a solver with the default configuration.
    pub fn new() -> Self {
        Self { config: MctsConfig::default() }
    }

    /// Construct a solver with explicit configuration.
    pub fn with_config(config: MctsConfig) -> Self {
        Self { config }
    }

    /// Run the MCTS search and return the most-visited root child.
    ///
    /// Returns `Err(MctsError::EmptyActionSet)` when the state admits no
    /// legal action, and `Err(MctsError::ZeroBudget)` when the caller
    /// refuses to give any time.
    pub fn run(&self, state: &DayState, time_budget: Duration) -> Result<PickedAction, MctsError> {
        if time_budget.is_zero() {
            return Err(MctsError::ZeroBudget);
        }
        let legal = state.legal_actions();
        if legal.is_empty() {
            return Err(MctsError::EmptyActionSet);
        }
        let mut rng = Rng::from_seed(self.config.rng_seed);
        let start = Instant::now();
        let mut root = MctsNode::root();
        let mut explored: Vec<Action> = Vec::with_capacity(legal.len());

        for _ in 0..self.config.iterations {
            if start.elapsed() >= time_budget {
                break;
            }
            if let Some(time_budget) = self.config.time_budget {
                if start.elapsed() >= time_budget {
                    break;
                }
            }

            // 1) Pick a child: either expand a new action or descend via UCB1.
            let (action, is_new) = if explored.len() < legal.len() {
                let picked = sample_expansion(state, &explored, &mut rng, self.config.epsilon);
                explored.push(picked);
                (picked, true)
            } else {
                let idx = root
                    .best_child_ucb1(root.visits.max(1), self.config.explore_constant)
                    .unwrap_or(0);
                let child = &root.children[idx];
                (child.parent_action.unwrap_or(legal[0]), false)
            };

            // 2) Expand (idempotent — children are created lazily here).
            if is_new {
                root.expand(action);
            }

            // 3) Rollout from the post-action state.
            let child_state = state.apply(action);
            let reward =
                rollout(&child_state, self.config.rollout_depth, self.config.epsilon, &mut rng);

            // 4) Backprop. With a depth-1 tree the path is two nodes.
            apply_root_update(&mut root, action, reward);
        }

        let idx = root.most_visited_child().ok_or(MctsError::EmptyActionSet)?;
        let child = &root.children[idx];
        let visits = child.visits;
        Ok(PickedAction {
            action: child.parent_action.unwrap_or(legal[0]),
            visits,
            mean_reward: if visits == 0 { 0.0 } else { child.total_reward / f64::from(visits) },
            iterations_run: root.visits,
        })
    }
}

impl Default for MctsSolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of a single `plan_step` call. Exposed for telemetry and tests.
#[derive(Debug, Clone, PartialEq)]
pub struct PickedAction {
    /// Action the solver recommends for the current step.
    pub action: Action,
    /// Number of times the MCTS visited this root child.
    pub visits: u32,
    /// Mean rollout reward across those visits.
    pub mean_reward: f64,
    /// Total number of MCTS iterations that ran inside the budget.
    pub iterations_run: u32,
}

fn sample_expansion(state: &DayState, already: &[Action], rng: &mut Rng, epsilon: f64) -> Action {
    let legal = state.legal_actions();
    let unexplored: Vec<Action> = legal.iter().copied().filter(|a| !already.contains(a)).collect();
    if unexplored.is_empty() {
        return legal[0];
    }
    if rng.gen_bool(epsilon) || unexplored.len() == 1 {
        return rng.choose(&unexplored).copied().unwrap_or(unexplored[0]);
    }
    // Greedy among the unexplored.
    greedy_within(state, &unexplored)
}

fn greedy_within(state: &DayState, choices: &[Action]) -> Action {
    let mut best: Option<(Action, f64)> = None;
    for &a in choices {
        let v = state
            .pool
            .iter()
            .find(|s| s.index == a.0)
            .map_or(f64::NEG_INFINITY, |s| s.immediate_value);
        match best {
            None => best = Some((a, v)),
            Some((_, bv)) if v > bv => best = Some((a, v)),
            _ => {}
        }
    }
    best.map_or(choices[0], |(a, _)| a)
}

fn apply_root_update(root: &mut MctsNode, action: Action, reward: f64) {
    root.visits = root.visits.saturating_add(1);
    for child in &mut root.children {
        if child.parent_action == Some(action) {
            child.visits = child.visits.saturating_add(1);
            child.total_reward += reward;
            break;
        }
    }
}
