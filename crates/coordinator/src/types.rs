//! Domain stubs used by the coordinator.
//!
//! EXPECTED INTEGRATION: domain/engine
//!
//! The real procon match model lives in the `domain` and `engine` crates; those
//! crates are still being scaffolded by other tickets. To keep this crate
//! self-contained for unit testing, we mirror the small surface we need as a
//! local stub. Once the upstream types stabilise, replace these aliases with
//! re-exports from `domain` / `engine` and delete the duplicates.

use serde::{Deserialize, Serialize};

/// Map terrain cell types. The contest board is a small grid (≤ 32×32).
///
/// Mirrors the abstract notion used by `domain` / `engine`; the real enum will
/// arrive with the board module. For now we only need the road-vs-non-road
/// distinction to compute road density.
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

/// Minimal match setting description.
///
/// This is intentionally narrow — only the fields the picker reasons about.
/// EXPECTED INTEGRATION: domain/engine — replace with the real `MatchSetting`
/// once it ships.
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
///
/// EXPECTED INTEGRATION: domain/engine — the canonical list will live with
/// the solver registry; this stub uses static strings for ease of testing.
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
}
