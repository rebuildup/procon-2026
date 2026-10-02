//! Minimal context / action / path stubs.
//!
//! These types fill the gap until `crates/domain` and `crates/engine`
//! publish their canonical `SolveContext` / `Action` / `Path`. The shapes
//! here deliberately mirror what issue #57's `Solver` contract expects, so
//! swapping the stub types for the real ones is a one-commit migration.

use crate::hex::{Direction, HexCoord};
use crate::map::HexMap;

/// One agent action.
///
/// The 7 reachable actions are `Wait` plus the six hex directions from
/// [`Direction`] (募集要項「移動と燃料」 + the 8-move `6 hex + wait`
/// requirement from issue #25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Stay put. Always legal; never consumes fuel or step budget.
    Wait,
    /// Move one cell in the given direction.
    Move(Direction),
}

impl Action {
    /// Step budget consumed by an action. `Wait` always consumes one step
    /// so days tick uniformly regardless of whether the agent moved.
    #[must_use]
    pub const fn step_budget() -> u32 {
        1
    }
}

/// One agent's planned trajectory through the day.
///
/// The `steps` slice is the **ordered** action list: `steps[0]` is the
/// first action taken at step 1, `steps[t-1]` is the action at step `t`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Path {
    pub steps: Vec<Action>,
}

impl Path {
    /// Empty path — the agent stays at `start` for the whole day.
    #[must_use]
    pub fn empty() -> Self {
        Self { steps: Vec::new() }
    }

    /// Total step budget consumed.
    #[must_use]
    pub fn step_total(&self) -> u32 {
        // Wait also ticks the day; using `step_budget` keeps the contract
        // honest even if we later split out "no-op" actions.
        u32::try_from(self.steps.len()).unwrap_or(u32::MAX) * Action::step_budget()
    }

    /// Reconstruct the agent's position after every step of the path.
    ///
    /// `starts` is the position before the first action; the returned
    /// vector is one entry longer than `self.steps` and skips `Wait`
    /// positions (they are equal to the previous position).
    #[must_use]
    pub fn positions(&self, start: HexCoord) -> Vec<HexCoord> {
        let mut out = Vec::with_capacity(self.steps.len() + 1);
        out.push(start);
        let mut cur = start;
        for &action in &self.steps {
            if let Action::Move(dir) = action {
                cur = dir.step(cur);
            }
            out.push(cur);
        }
        out
    }
}

/// Input for a single A* solve.
///
/// `max_steps` mirrors the `daySteps` budget; the solver will refuse to
/// return a path that needs more than that.
#[derive(Debug, Clone)]
pub struct SolveContext {
    pub map: HexMap,
    pub start: HexCoord,
    pub goal: HexCoord,
    pub initial_fuel: u32,
    pub max_steps: u32,
}

impl SolveContext {
    /// Build a context. Returns `None` if the start or goal is outside the
    /// map or if either sits on a pond (no point planning a trip into a
    /// wall).
    #[must_use]
    pub fn new(
        map: HexMap,
        start: HexCoord,
        goal: HexCoord,
        initial_fuel: u32,
        max_steps: u32,
    ) -> Option<Self> {
        let start_cell = map.get(start)?;
        let goal_cell = map.get(goal)?;
        if !start_cell.is_passable() || !goal_cell.is_passable() {
            return None;
        }
        Some(Self { map, start, goal, initial_fuel, max_steps })
    }
}

/// Output of [`super::Solver::solve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverResult {
    /// `Some(path)` if a path was found, `None` otherwise.
    pub path: Option<Path>,
    /// Number of cells the solver expanded. Useful for benchmarking.
    pub expanded: usize,
    /// Optional explanation of why no path was found.
    pub failure: Option<SolverFailure>,
}

/// Reasons a solver may return no path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolverFailure {
    /// Goal is unreachable under the given cost model / fuel budget.
    Unreachable,
    /// The smallest possible cost would exceed `max_steps`.
    ExceedsStepBudget { min_cost: u32 },
    /// The smallest possible fuel consumption exceeds `initial_fuel`.
    OutOfFuel { min_fuel: u32 },
}
