//! Shared types: solver trait, action sequence, match input, output of a
//! single solver run, plus the picker-side stubs for terrain / match
//! setting / solver name.
//!
//! These types are the minimum viable shape needed to dispatch multiple
//! solvers in parallel, decide between their results, and pick a strategy.
//! They intentionally avoid coupling to `engine`, `protocol`, or the
//! unmerged `solver-*` crates; the `EngineServer` / `MatchSnapshot`
//! boundary is normalised into plain Rust values here.

use serde::{Deserialize, Serialize};

use crate::score::StubScoreReport;

// ---------------------------------------------------------------------
// Picker-side types (Issue #46)
// ---------------------------------------------------------------------

/// Map terrain cell types. The contest board is a small grid (≤ 32×32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Terrain {
    Plain,
    Mountain,
    Road,
}

impl Terrain {
    /// Whether the cell counts as a road for density statistics.
    #[must_use]
    pub const fn is_road(self) -> bool {
        matches!(self, Self::Road)
    }
}

/// Minimal match setting description (picker input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchSetting {
    /// Board width in cells (≤ 32).
    pub width: u8,
    /// Board height in cells (≤ 32).
    pub height: u8,
    /// Grid cells, row-major, `width * height` items.
    pub terrain: Vec<Terrain>,
    /// Number of udon series (a.k.a. noodle types) on the board.
    pub series_count: u8,
    /// Number of spot locations (≤ 32 per series count).
    pub spot_count: u8,
    /// Number of agents in the team (3..=8).
    pub agent_count: u8,
}

impl MatchSetting {
    /// Number of cells in the board. Used to normalise density metrics.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }

    /// Road cell count.
    #[must_use]
    pub fn road_cell_count(&self) -> usize {
        self.terrain.iter().filter(|t| t.is_road()).count()
    }
}

/// Solver identifier. The picker outputs one of these per match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SolverName {
    /// Greedy step-by-step heuristic. Strategy.md §5.3 Phase 1.
    Greedy,
    /// A* with per-spot precomputed shortest paths. §5.3 Phase 2.
    Astar,
    /// Iterative Local Search over cluster assignments. §5.3 Phase 2.
    LocalSearch,
    /// ILP solver for the joint assignment. §5.3 Phase 3.
    Ilp,
    /// Monte-Carlo Tree Search over action plans. §5.3 Phase 3.
    Mcts,
    /// Fallback when no rule fires. Defaults to greedy.
    Fallback,
}

impl SolverName {
    /// Stable lower-case identifier for logging / metric labels.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Greedy => "greedy",
            Self::Astar => "astar",
            Self::LocalSearch => "local-search",
            Self::Ilp => "ilp",
            Self::Mcts => "mcts",
            Self::Fallback => "fallback",
        }
    }
}

// ---------------------------------------------------------------------
// Ensemble-side types (Issue #45)
// ---------------------------------------------------------------------

/// A single atomic action chosen by a solver.
pub type Action = u32;

/// An ordered list of actions. Equality of action sequences is the basis
/// for voting consensus (see [`crate::voting`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionSequence {
    pub actions: Vec<Action>,
}

impl ActionSequence {
    /// Empty sequence.
    #[must_use]
    pub fn empty() -> Self {
        Self { actions: Vec::new() }
    }

    /// Sequence with the given actions.
    #[must_use]
    pub fn from_actions(actions: Vec<Action>) -> Self {
        Self { actions }
    }

    /// Number of actions in the sequence.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// True if there are no actions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

impl From<Vec<Action>> for ActionSequence {
    fn from(actions: Vec<Action>) -> Self {
        Self::from_actions(actions)
    }
}

/// Inputs handed to each solver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchInput {
    pub match_id: String,
    pub turn: u32,
    /// Free-form opaque payload (e.g. JSON mirror of the snapshot).
    pub payload: serde_json::Value,
}

impl MatchInput {
    /// Construct a minimal match input.
    #[must_use]
    pub fn new(match_id: impl Into<String>, turn: u32) -> Self {
        Self {
            match_id: match_id.into(),
            turn,
            payload: serde_json::Value::Null,
        }
    }
}

/// Output of a single solver run: its action plan plus the score report
/// it (or the judge) produced for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverOutput {
    pub score: StubScoreReport,
    pub plan: ActionSequence,
}

/// The trait every solver must implement to be orchestrated by the
/// coordinator.
pub trait Solver: Send + Sync + 'static {
    /// Stable identifier (used in reports and tie-breaks).
    fn id(&self) -> &'static str;

    /// Produce an action plan for the given input.
    fn solve(&self, input: &MatchInput) -> SolverOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_road_predicate() {
        assert!(Terrain::Road.is_road());
        assert!(!Terrain::Plain.is_road());
        assert!(!Terrain::Mountain.is_road());
    }

    #[test]
    fn solver_name_strings_are_distinct() {
        let names = [
            SolverName::Greedy,
            SolverName::Astar,
            SolverName::LocalSearch,
            SolverName::Ilp,
            SolverName::Mcts,
            SolverName::Fallback,
        ];
        let labels: Vec<_> = names.iter().map(|n| n.as_str()).collect();
        let unique: std::collections::HashSet<_> = labels.iter().copied().collect();
        assert_eq!(unique.len(), labels.len());
    }

    #[test]
    fn action_sequence_helpers() {
        let empty = ActionSequence::empty();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let nonempty = ActionSequence::from_actions(vec![1, 2, 3]);
        assert!(!nonempty.is_empty());
        assert_eq!(nonempty.len(), 3);
        assert_eq!(nonempty.actions, vec![1, 2, 3]);
    }

    #[test]
    fn match_input_default_payload_is_null() {
        let input = MatchInput::new("match-1", 7);
        assert_eq!(input.match_id, "match-1");
        assert_eq!(input.turn, 7);
        assert_eq!(input.payload, serde_json::Value::Null);
    }

    #[test]
    fn solver_trait_is_object_safe() {
        // Compile-time assertion: Solver is dyn-compatible.
        fn _accept(_s: &dyn Solver) {}
    }
}
