//! Tree node + UCB1 selection for the MCTS skeleton.
//!
//! Each [`MctsNode`] owns its child slots; the root owns the entire tree.
//! UCB1 scores are recomputed on demand — the hot loop never allocates
//! past the initial child expansion.

use std::fmt;

use crate::types::Action;

/// One node in the MCTS tree.
///
/// `state` is intentionally not stored — once issue #57's canonical state
/// lands, the kernel passes snapshots by reference and the tree only
/// carries the exploration statistics.
#[derive(Debug, Clone)]
pub struct MctsNode {
    /// Edge label leading into this node (the action taken from the parent).
    /// `None` for the root.
    pub parent_action: Option<Action>,
    /// Visit count for the path through this node.
    pub visits: u32,
    /// Sum of rollout rewards backpropagated through this node.
    pub total_reward: f64,
    /// Children keyed by action. Lazily filled on expansion.
    pub children: Vec<MctsNode>,
}

impl MctsNode {
    /// Construct an empty root.
    pub fn root() -> Self {
        Self { parent_action: None, visits: 0, total_reward: 0.0, children: Vec::new() }
    }

    /// True when the node has any child slots.
    pub fn is_expanded(&self) -> bool {
        !self.children.is_empty()
    }

    /// Append a new child for the given action.
    pub fn expand(&mut self, action: Action) {
        self.children.push(MctsNode {
            parent_action: Some(action),
            visits: 0,
            total_reward: 0.0,
            children: Vec::new(),
        });
    }

    /// Mean reward so far. Returns `0.0` for unvisited nodes.
    pub fn mean_reward(&self) -> f64 {
        if self.visits == 0 { 0.0 } else { self.total_reward / f64::from(self.visits) }
    }

    /// UCB1 score against the parent visit count.
    ///
    /// Unvisited children return `f64::INFINITY` so the expansion policy
    /// always explores them first.
    pub fn ucb1(&self, parent_visits: u32, c: f64) -> f64 {
        if self.visits == 0 {
            return f64::INFINITY;
        }
        let exploit = self.mean_reward();
        let explore = c * (f64::from(parent_visits).ln() / f64::from(self.visits)).sqrt();
        exploit + explore
    }

    /// Pick the child with the highest UCB1 score, breaking ties by lowest
    /// action index for determinism.
    pub fn best_child_ucb1(&self, parent_visits: u32, c: f64) -> Option<usize> {
        self.children
            .iter()
            .enumerate()
            .max_by(|(ai, a), (bi, b)| {
                let sa = a.ucb1(parent_visits, c);
                let sb = b.ucb1(parent_visits, c);
                // NaN-safe: prefer finite values.
                match (sa.is_nan(), sb.is_nan()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => sa
                        .partial_cmp(&sb)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| ai.cmp(bi)),
                }
            })
            .map(|(i, _)| i)
    }

    /// Pick the most-visited child once the search budget is spent. This is
    /// the "robust child" choice — preferred over max-mean because of its
    /// lower variance.
    pub fn most_visited_child(&self) -> Option<usize> {
        self.children
            .iter()
            .enumerate()
            .max_by(|(ai, a), (bi, b)| {
                a.visits
                    .cmp(&b.visits)
                    .then_with(|| {
                        b.total_reward
                            .partial_cmp(&a.total_reward)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| ai.cmp(bi))
            })
            .map(|(i, _)| i)
    }
}

impl fmt::Display for MctsNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}n={} avg={:.3}",
            self.parent_action.map_or_else(|| "root ".to_string(), |a| format!("[{a}] ")),
            self.visits,
            self.mean_reward()
        )
    }
}
