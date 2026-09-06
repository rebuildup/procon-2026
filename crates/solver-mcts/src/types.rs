//! Stub types for the MCTS solver.
//!
//! EXPECTED INTEGRATION: domain — once issue #57 lands the canonical
//! `Solver` trait and #18 nails down `GameState` / `Turn`, this module is
//! replaced by `domain::{GameState, Solver, Turn}` plus a thin `Action`
//! newtype. Until then the shapes here are deliberately minimal — enough
//! to exercise UCB1, expansion, rollout, backprop, and the day-step
//! budget without dragging in the not-yet-merged engine kernel.
//!
//! The current stub models a single day's plan as a sequence of `Action`
//! indices drawn (without repetition) from a fixed pool. Each action carries
//! an `immediate_value` that the rollout aggregates; this mirrors the
//! "燃料経済 / 系列数" objective from `docs/contest/strategy.md` §3.1.

use std::collections::BTreeSet;
use std::fmt;

/// Upper bound on the size of a single step's action set.
///
/// Mirrors `domain::Turn::action_index` (u16). Constrained here so the
/// vector allocations stay stack-friendly in the hot MCTS loop.
pub const MAX_ACTIONS: usize = u16::MAX as usize;

/// Discrete action the solver commits to. Indexed into the state's action
/// pool. Mirrors `domain::Turn::action_index` (u16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Action(pub u16);

impl Action {
    /// Construct an action from a raw index. Returns `None` if the index is
    /// out of bounds.
    pub fn new(index: u16) -> Option<Self> {
        if (index as usize) < MAX_ACTIONS { Some(Self(index)) } else { None }
    }

    /// Raw index, mirroring `domain::Turn::action_index`.
    pub fn index(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Action({})", self.0)
    }
}

/// Per-action metadata used by the heuristic rollout and greedy expansion.
///
/// `immediate_value` is the score contribution when this action is picked
/// at the current step (think: spot reward minus travel cost). The rollout
/// uses it as the per-step objective; the greedy expansion picks the
/// action with the largest value among legal choices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActionSpec {
    /// Stable index used for tie-breaking.
    pub index: u16,
    /// Score contribution when this action is taken at the current step.
    pub immediate_value: f64,
}

/// A single day's plan state.
///
/// Internal stub. The real `domain::GameState` will replace this with a
/// richer model (agents, fuel, spot series, map position). The interface
/// is intentionally narrow — only the operations the MCTS kernel needs.
#[derive(Debug, Clone, PartialEq)]
pub struct DayState {
    /// Current step within the day. `0` means "nothing committed yet".
    pub step: u32,
    /// Maximum number of steps the day allows (clamped from `daySeconds`).
    pub max_steps: u32,
    /// Pool of legal actions and their immediate-value estimates.
    pub pool: Vec<ActionSpec>,
    /// Already-committed actions for this day.
    pub taken: Vec<Action>,
    /// Running total of `immediate_value` for the committed sequence.
    pub cumulative_value: f64,
}

impl DayState {
    /// Construct a fresh day state from an action pool.
    pub fn new(pool: Vec<ActionSpec>, max_steps: u32) -> Self {
        Self { step: 0, max_steps, pool, taken: Vec::new(), cumulative_value: 0.0 }
    }

    /// Legal actions for the current step, sorted by index for determinism.
    pub fn legal_actions(&self) -> Vec<Action> {
        let taken: BTreeSet<u16> = self.taken.iter().map(|a| a.0).collect();
        self.pool.iter().filter(|s| !taken.contains(&s.index)).map(|s| Action(s.index)).collect()
    }

    /// Apply an action, returning the next state. Consumed actions are
    /// permanently removed from the pool until `undo` is called.
    #[must_use]
    pub fn apply(&self, action: Action) -> Self {
        let mut next = self.clone();
        if let Some(spec) = next.pool.iter().find(|s| s.index == action.0) {
            next.cumulative_value += spec.immediate_value;
        }
        next.taken.push(action);
        next.step = next.step.saturating_add(1);
        next
    }

    /// Roll back the most recent action. Useful for backprop-free
    /// in-place look-ahead (e.g. testing). Returns the previous state.
    pub fn undo(&mut self) -> Option<Action> {
        let a = self.taken.pop()?;
        if let Some(spec) = self.pool.iter().find(|s| s.index == a.0) {
            self.cumulative_value -= spec.immediate_value;
        }
        self.step = self.step.saturating_sub(1);
        Some(a)
    }

    /// True when no more actions are possible or the step limit is hit.
    pub fn is_terminal(&self) -> bool {
        self.step >= self.max_steps || self.legal_actions().is_empty()
    }
}

/// Outcome of `MctsSolver::plan_step` when something goes wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// No legal actions exist; the state is terminal.
    EmptyActionSet,
    /// The day-step budget was zero — nothing to plan.
    ZeroBudget,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyActionSet => f.write_str("no legal actions available"),
            Self::ZeroBudget => f.write_str("zero time budget passed to solver"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Greedy planner used as the lower-bound baseline in tests.
///
/// Picks the legal action with the largest `immediate_value`. On ties it
/// breaks by lowest index so the result is deterministic.
pub fn plan_one_step(state: &DayState) -> Result<Action, PlanError> {
    let legal = state.legal_actions();
    let mut best: Option<(Action, f64)> = None;
    for a in legal {
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
    best.map_or_else(|| Err(PlanError::EmptyActionSet), |(a, _)| Ok(a))
}
