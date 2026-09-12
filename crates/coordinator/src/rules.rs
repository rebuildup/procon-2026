//! Rule predicates over [`Characteristics`].
//!
//! Each predicate captures one decision the picker makes. Predicates are
//! pure functions so they can be unit-tested independently and composed
//! inside [`crate::picker::StrategyPicker`].
//!
//! Thresholds were chosen from `docs/contest/strategy.md` §5:
//! * Many series (≥ 8): greedy is too coarse; switch to a global optimiser.
//! * Few series × many agents: per-agent assignment dominates → ILP / local-search.
//! * Dense roads (≥ 0.5): precompute A* shortest paths.
//! * Sparse roads (< 0.2): avoid A*; use greedy or MCTS.

use crate::features::Characteristics;
use crate::types::SolverName;

/// Minimum series count that triggers a non-greedy solver.
pub const SERIES_HIGH_THRESHOLD: u32 = 8;
/// Minimum agent count that justifies a global assignment solver when series
/// count is small (i.e. there are enough agents to cover the series).
pub const AGENT_RICH_THRESHOLD: u32 = 6;
/// Road density above which we lean on precomputed shortest paths.
pub const ROAD_DENSE_THRESHOLD: f32 = 0.5;
/// Road density below which shortest-path precomputation gives little gain.
pub const ROAD_SPARSE_THRESHOLD: f32 = 0.2;
/// Average traversal cost above which the map is mostly mountainous.
pub const MOUNTAIN_AVG_COST: f32 = 2.5;

/// The board has many distinct udon series. Greedy is too coarse.
#[must_use]
pub const fn many_series(c: &Characteristics) -> bool {
    c.series_count >= SERIES_HIGH_THRESHOLD
}

/// Series count is low but the team has enough agents to assign one per
/// series. ILP / local-search can plan the global assignment.
#[must_use]
pub const fn series_poor_but_agent_rich(c: &Characteristics) -> bool {
    c.series_count < SERIES_HIGH_THRESHOLD && c.agent_count >= AGENT_RICH_THRESHOLD
}

/// Road cells dominate the board → A* shortest paths pay off.
#[must_use]
pub const fn road_dense(c: &Characteristics) -> bool {
    c.road_density >= ROAD_DENSE_THRESHOLD
}

/// Road cells are scarce → A* offers little; use greedy or MCTS.
#[must_use]
pub const fn road_sparse(c: &Characteristics) -> bool {
    c.road_density < ROAD_SPARSE_THRESHOLD
}

/// Map is mostly mountainous → routes are short but expensive.
#[must_use]
pub const fn mountainous(c: &Characteristics) -> bool {
    c.map_avg_cost >= MOUNTAIN_AVG_COST
}

/// Default solver when no rule fires. Strategy.md §5.3 Phase 1.
#[must_use]
pub const fn default_solver() -> SolverName {
    SolverName::Greedy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(series: u32, spot: u32, agent: u32, avg: f32, road: f32) -> Characteristics {
        Characteristics {
            series_count: series,
            spot_count: spot,
            agent_count: agent,
            map_avg_cost: avg,
            road_density: road,
        }
    }

    #[test]
    fn many_series_predicate() {
        assert!(many_series(&c(8, 16, 4, 1.0, 0.3)));
        assert!(!many_series(&c(7, 16, 4, 1.0, 0.3)));
    }

    #[test]
    fn series_poor_agent_rich_predicate() {
        assert!(series_poor_but_agent_rich(&c(4, 8, 8, 1.0, 0.3)));
        assert!(!series_poor_but_agent_rich(&c(8, 8, 8, 1.0, 0.3)));
        assert!(!series_poor_but_agent_rich(&c(4, 8, 5, 1.0, 0.3)));
    }

    #[test]
    fn road_density_thresholds() {
        assert!(road_dense(&c(4, 8, 4, 1.0, 0.5)));
        assert!(!road_dense(&c(4, 8, 4, 1.0, 0.49)));
        assert!(road_sparse(&c(4, 8, 4, 1.0, 0.19)));
        assert!(!road_sparse(&c(4, 8, 4, 1.0, 0.2)));
    }

    #[test]
    fn mountainous_predicate() {
        assert!(mountainous(&c(4, 8, 4, 2.5, 0.0)));
        assert!(!mountainous(&c(4, 8, 4, 2.49, 0.0)));
    }

    #[test]
    fn default_solver_is_greedy() {
        assert_eq!(default_solver(), SolverName::Greedy);
    }
}
