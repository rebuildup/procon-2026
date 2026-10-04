//! Multi-agent MCTS — joint-action UCB tree search (Issue #42).
//!
//! Extends the single-agent [`crate::MctsSolver`] with support for multiple
//! agents acting cooperatively in the same step. Each tree node expands
//! the joint-action space (Cartesian product of per-agent legal actions)
//! and uses UCB1 to pick which joint-action to simulate next.
//!
//! Search loop, mirroring [`crate::solver::MctsSolver::run`]:
//!
//! 1. **Select** — either expand an un-visited joint-action, or descend
//!    via UCB1 across the children that have already been expanded.
//! 2. **Expand** — append the chosen joint-action as a child of the root.
//! 3. **Rollout** — cooperate by summing each agent's greedy per-step
//!    `immediate_value` along the joint-action. A deterministic, cheap
//!    evaluator (the variety-aware reward lives in [`crate::rollout`]).
//! 4. **Back-propagate** — credit the reward to the root and the child.
//!
//! The returned joint-action is the most-visited child of the root
//! ("robust child" choice — lower variance than max-mean).

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use crate::rollout::Rng;
use crate::types::{Action, DayState};

/// Stable identifier for an agent inside the multi-agent MCTS.
///
/// Mirrors the `AgentId(pub u8)` shape used by sibling solver crates
/// (`solver-greedy`, `solver-ilp-cross`). The crate-local definition
/// keeps this module independent until the canonical `domain::AgentId`
/// lands via issue #13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct AgentId(pub u8);

impl AgentId {
    /// Construct an agent id from a raw byte.
    #[must_use]
    pub const fn new(v: u8) -> Self {
        Self(v)
    }

    /// Raw byte value.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Agent({})", self.0)
    }
}

/// One joint action — exactly one [`Action`] per cooperating agent.
///
/// The order of `actions` matches the order of the per-agent legal
/// action sets supplied to [`JointAction::enumerate`]. The solver uses
/// this stable ordering to compare joint actions deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JointAction {
    /// Per-agent actions. `actions[i]` is the action taken by agent `i`.
    pub actions: Vec<Action>,
}

impl JointAction {
    /// Construct a joint action from a per-agent action vector.
    #[must_use]
    pub fn new(actions: Vec<Action>) -> Self {
        Self { actions }
    }

    /// Borrow the per-agent action slice.
    #[must_use]
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// Number of agents in this joint action.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// True when no agents are participating.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Build the joint-action space as the Cartesian product of the
    /// per-agent legal-action sets.
    ///
    /// `per_agent[i]` is the set of legal actions for agent `i`. When
    /// every entry is non-empty the resulting vector has length
    /// `product(|per_agent[i]|)`. A single empty entry yields a single
    /// empty joint action (matches the "no choices" sentinel).
    #[must_use]
    pub fn enumerate(per_agent: &[Vec<Action>]) -> Vec<JointAction> {
        if per_agent.is_empty() {
            return vec![JointAction { actions: Vec::new() }];
        }
        let mut acc: Vec<JointAction> = vec![JointAction { actions: Vec::new() }];
        for actions in per_agent {
            let mut next = Vec::with_capacity(acc.len() * actions.len().max(1));
            for prefix in &acc {
                if actions.is_empty() {
                    // No choices for this agent — keep the prefix as-is so
                    // the search can still recognise the empty branch.
                    next.push(prefix.clone());
                } else {
                    for &a in actions {
                        let mut new_actions = prefix.actions.clone();
                        new_actions.push(a);
                        next.push(JointAction { actions: new_actions });
                    }
                }
            }
            acc = next;
        }
        acc
    }
}

impl fmt::Display for JointAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JointAction([")?;
        for (i, a) in self.actions.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{a}")?;
        }
        write!(f, "])")
    }
}

/// One node in the multi-agent MCTS tree.
#[derive(Debug, Clone)]
pub struct MultiAgentNode {
    /// Joint-action space available at this node — the full Cartesian
    /// product enumerated at node construction.
    pub joint_actions: Vec<JointAction>,
    /// Total visits through this node.
    pub visits: u32,
    /// Sum of rollout rewards back-propagated through this node.
    pub total_reward: f64,
    /// Children explored so far, keyed by the joint-action taken.
    pub children: HashMap<JointAction, MultiAgentNode>,
}

impl MultiAgentNode {
    /// Build a fresh root carrying the supplied joint-action space.
    #[must_use]
    pub fn root(joint_actions: Vec<JointAction>) -> Self {
        Self { joint_actions, visits: 0, total_reward: 0.0, children: HashMap::new() }
    }

    /// Mean reward across all visits to this node. `0.0` when unvisited.
    #[must_use]
    pub fn mean_reward(&self) -> f64 {
        if self.visits == 0 { 0.0 } else { self.total_reward / f64::from(self.visits) }
    }

    /// UCB1 score against `parent_visits`.
    ///
    /// Unvisited nodes return `f64::INFINITY` so the expansion policy
    /// always explores them first. Mirrors [`crate::node::MctsNode::ucb1`].
    #[must_use]
    pub fn ucb1(&self, parent_visits: u32, c: f64) -> f64 {
        if self.visits == 0 {
            return f64::INFINITY;
        }
        let exploit = self.mean_reward();
        let explore = c * (f64::from(parent_visits).ln() / f64::from(self.visits)).sqrt();
        exploit + explore
    }

    /// Pick the child with the highest UCB1 score, breaking ties by
    /// lexicographic order on the joint-action for determinism.
    #[must_use]
    pub fn best_child_ucb1(&self, parent_visits: u32, c: f64) -> Option<&JointAction> {
        self.children
            .iter()
            .max_by(|(ka, a), (kb, b)| {
                let sa = a.ucb1(parent_visits, c);
                let sb = b.ucb1(parent_visits, c);
                match (sa.is_nan(), sb.is_nan()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => sa
                        .partial_cmp(&sb)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| ka.cmp(kb)),
                }
            })
            .map(|(k, _)| k)
    }

    /// Most-visited child (robust child) — preferred over max-mean
    /// because of its lower variance on noisy reward signals.
    #[must_use]
    pub fn most_visited_child(&self) -> Option<(&JointAction, &MultiAgentNode)> {
        self.children.iter().max_by(|(ka, a), (kb, b)| {
            a.visits
                .cmp(&b.visits)
                .then_with(|| {
                    // Same inversion as `node::most_visited_child`: order `a`
                    // relative to `b`, or a visit tie picks the worst joint
                    // action.
                    a.total_reward.partial_cmp(&b.total_reward).unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| ka.cmp(kb))
        })
    }
}

/// Multi-agent MCTS — joint-action tree search.
///
/// Configuration mirrors [`crate::MctsConfig`] but is expressed in the
/// joint-action vocabulary. The `agents` field carries the agent roster
/// the solver plans for; the runtime search uses the input states
/// (`Vec<DayState>`) to enumerate the joint-action space.
#[derive(Debug, Clone)]
pub struct MultiAgentMCTS {
    /// UCB1 exploration constant (UCT-style `c`). Defaults to `√2`.
    pub ucb_c: f64,
    /// Agent roster the solver is responsible for. Must be non-empty.
    pub agents: Vec<AgentId>,
    /// Number of `select -> expand -> rollout -> backprop` cycles per
    /// planning call.
    pub iterations: usize,
    /// PRNG seed. `0` is rewritten to a fixed fallback by
    /// [`Rng::from_seed`].
    pub seed: u64,
}

impl Default for MultiAgentMCTS {
    fn default() -> Self {
        Self { ucb_c: core::f64::consts::SQRT_2, agents: Vec::new(), iterations: 1_000, seed: 0 }
    }
}

impl MultiAgentMCTS {
    /// Construct a solver with explicit parameters.
    #[must_use]
    pub fn new(ucb_c: f64, agents: Vec<AgentId>, iterations: usize, seed: u64) -> Self {
        Self { ucb_c, agents, iterations, seed }
    }

    /// Compute the joint-action space size as the product of per-agent
    /// legal-action counts. Returns `1` when `states` is empty (the
    /// single empty joint action).
    #[must_use]
    pub fn joint_action_space_size(states: &[DayState]) -> usize {
        if states.is_empty() {
            return 1;
        }
        states.iter().map(|s| s.legal_actions().len().max(1)).product()
    }

    /// Plan a single multi-agent step. Returns the most-visited
    /// joint-action once `iterations` cycles have completed.
    pub fn plan(&self, states: &[DayState]) -> Result<JointAction, MultiAgentError> {
        if states.is_empty() {
            return Err(MultiAgentError::EmptyActionSet);
        }
        let per_agent: Vec<Vec<Action>> = states.iter().map(DayState::legal_actions).collect();
        if per_agent.iter().any(Vec::is_empty) {
            return Err(MultiAgentError::EmptyActionSet);
        }
        let joint = JointAction::enumerate(&per_agent);
        if joint.is_empty() {
            return Err(MultiAgentError::EmptyActionSet);
        }

        let mut root = MultiAgentNode::root(joint.clone());
        // `Rng` is reserved for future stochastic extensions (ε-random
        // expansion of joint actions). The current search is fully
        // deterministic so we only materialise the seed.
        let _rng = Rng::from_seed(self.seed);

        let iter_budget = self.iterations.max(1);
        for _ in 0..iter_budget {
            // 1) Pick: either expand a fresh joint-action or descend.
            let (action, is_new) = if root.children.len() < joint.len() {
                let next =
                    joint.iter().find(|ja| !root.children.contains_key(*ja)).cloned().expect(
                        "unexplored joint action exists while children.len() < joint.len()",
                    );
                (next, true)
            } else {
                let best = root
                    .best_child_ucb1(root.visits.max(1), self.ucb_c)
                    .cloned()
                    .expect("at least one child exists once expanded");
                (best, false)
            };

            // 2) Expand.
            if is_new {
                root.children.insert(
                    action.clone(),
                    MultiAgentNode {
                        joint_actions: Vec::new(),
                        visits: 0,
                        total_reward: 0.0,
                        children: HashMap::new(),
                    },
                );
            }

            // 3) Cooperative rollout: sum each agent's immediate value
            //    along the joint-action. A deterministic, allocation-light
            //    reward that matches the per-step objective used by
            //    `MctsSolver`'s rollout family.
            let reward = cooperative_value(states, &action);

            // 4) Backprop.
            apply_root_update(&mut root, &action, reward);
        }

        let (best, _) = root.most_visited_child().ok_or(MultiAgentError::EmptyActionSet)?;
        Ok(best.clone())
    }
}

/// Errors surfaced by [`MultiAgentMCTS::plan`] / the [`Solver`] impl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiAgentError {
    /// The state admits no legal action for at least one agent.
    EmptyActionSet,
}

impl fmt::Display for MultiAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyActionSet => f.write_str("no legal joint actions available"),
        }
    }
}

impl std::error::Error for MultiAgentError {}

/// [`Solver`] integration: state is a per-agent state vector, action is
/// a [`JointAction`].
impl crate::Solver for MultiAgentMCTS {
    type State = Vec<DayState>;
    type Action = JointAction;
    type Error = MultiAgentError;

    fn plan_step(
        &self,
        state: &Self::State,
        _time_budget: Duration,
    ) -> Result<Self::Action, Self::Error> {
        self.plan(state)
    }
}

/// Cooperative one-step reward for a joint action: sum of each agent's
/// `immediate_value` for its own action in the joint-action. Agents with
/// no matching spec contribute `0.0` (defensive — should not happen when
/// the joint-action was enumerated from the same states).
fn cooperative_value(states: &[DayState], joint: &JointAction) -> f64 {
    let mut total = 0.0;
    for (i, action) in joint.actions.iter().enumerate() {
        if let Some(s) = states.get(i) {
            if let Some(spec) = s.pool.iter().find(|sp| sp.index == action.0) {
                total += spec.immediate_value;
            }
        }
    }
    total
}

/// Backprop helper. Depth-1 tree → root + one child.
fn apply_root_update(root: &mut MultiAgentNode, action: &JointAction, reward: f64) {
    root.visits = root.visits.saturating_add(1);
    if let Some(child) = root.children.get_mut(action) {
        child.visits = child.visits.saturating_add(1);
        child.total_reward += reward;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ActionSpec;

    fn fixture_states() -> Vec<DayState> {
        vec![
            DayState::new(
                vec![
                    ActionSpec { index: 0, immediate_value: 1.0 },
                    ActionSpec { index: 1, immediate_value: 2.0 },
                    ActionSpec { index: 2, immediate_value: 3.0 },
                ],
                16,
            ),
            DayState::new(
                vec![
                    ActionSpec { index: 3, immediate_value: 4.0 },
                    ActionSpec { index: 4, immediate_value: 5.0 },
                ],
                16,
            ),
        ]
    }

    #[test]
    fn enumerate_yields_cartesian_product() {
        let states = fixture_states();
        let per_agent: Vec<Vec<Action>> = states.iter().map(DayState::legal_actions).collect();
        let joint = JointAction::enumerate(&per_agent);
        // 3 * 2 = 6 joint actions.
        assert_eq!(joint.len(), 6);
        // Each joint action has one entry per agent.
        for ja in &joint {
            assert_eq!(ja.actions().len(), states.len());
        }
    }

    #[test]
    fn enumerate_handles_empty_input() {
        let joint = JointAction::enumerate(&[]);
        assert_eq!(joint.len(), 1);
        assert!(joint[0].is_empty());
    }

    #[test]
    fn ucb_prefers_unvisited_child() {
        let actions = vec![JointAction::new(vec![Action(0)]), JointAction::new(vec![Action(1)])];
        let mut node = MultiAgentNode::root(actions.clone());
        // Manually populate one child with visits; leave the other empty.
        let mut visited = MultiAgentNode::root(Vec::new());
        visited.visits = 10;
        visited.total_reward = 5.0;
        node.children.insert(actions[0].clone(), visited);
        // Insert an unvisited child so UCB picks it via infinity.
        let unvisited = MultiAgentNode::root(Vec::new());
        node.children.insert(actions[1].clone(), unvisited);
        node.visits = 10;

        let best = node.best_child_ucb1(10, core::f64::consts::SQRT_2).expect("must pick");
        assert_eq!(best, &actions[1]);
    }

    #[test]
    fn cooperative_value_sums_per_agent() {
        let states = fixture_states();
        let joint = JointAction::new(vec![Action(2), Action(4)]);
        // Agent 0 → spec with value 3.0, Agent 1 → spec with value 5.0.
        let v = cooperative_value(&states, &joint);
        assert!((v - 8.0).abs() < 1e-9, "expected 8.0, got {v}");
    }
}
