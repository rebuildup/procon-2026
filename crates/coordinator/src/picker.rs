//! Per-match approach picker.
//!
//! Maps a [`MatchSetting`]'s [`Characteristics`] to a [`SolverName`] using a
//! small ordered rule table. The rule table is intentionally explicit (rather
//! than a learned model) so behaviour is auditable during contest play.
//!
//! ## Rule precedence
//!
//! 1. Mountainous maps with many agents → `ILP` (joint assignment + tight fuel).
//! 2. Many series → `ILP` (joint assignment dominates).
//! 3. Series-poor but agent-rich → `LocalSearch`.
//! 4. Road dense → `A*` (precomputed shortest paths).
//! 5. Road sparse + mountainous → `MCTS` (handles uncertainty).
//! 6. Road sparse → `Greedy` (cheap fallback).
//! 7. Otherwise → fallback ([`SolverName::Greedy`]).
//!
//! The first matching rule wins; later rules are ignored.

use std::collections::BTreeMap;

use crate::features::{self, Characteristics};
use crate::rules::{
    default_solver, many_series, mountainous, road_dense, road_sparse, series_poor_but_agent_rich,
};
use crate::types::{MatchSetting, SolverName};

/// Tunable configuration for [`StrategyPicker`].
///
/// Lets callers nudge thresholds without rewriting the rule table. Defaults
/// match the constants in [`crate::rules`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickerConfig {
    /// Multiplier applied to `Characteristics::series_count` when reading
    /// from the override table.
    pub series_round: u32,
}

impl Default for PickerConfig {
    fn default() -> Self {
        Self { series_round: 1 }
    }
}

/// Rule-driven approach picker.
///
/// The picker is stateless and pure: it owns a [`PickerConfig`] and exposes
/// [`StrategyPicker::pick`] which takes a [`MatchSetting`] and returns the
/// [`SolverName`] it recommends. Tests assert specific inputs map to
/// specific outputs.
#[derive(Debug, Clone, Default)]
pub struct StrategyPicker {
    config: PickerConfig,
}

impl StrategyPicker {
    /// Build a picker with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a picker with a custom configuration.
    #[must_use]
    pub fn with_config(config: PickerConfig) -> Self {
        Self { config }
    }

    /// Pick the most appropriate solver for the given match setting.
    ///
    /// Returns [`SolverName::Fallback`] only when the rule table fails to
    /// match anything — every well-formed board should hit at least one rule.
    #[must_use]
    pub fn pick(&self, setting: &MatchSetting) -> SolverName {
        let characteristics = features::extract(setting);
        self.pick_from_characteristics(characteristics)
    }

    /// Same as [`StrategyPicker::pick`] but takes pre-computed characteristics.
    ///
    /// Useful for testing and for callers that already cached the features.
    #[must_use]
    pub fn pick_from_characteristics(&self, c: Characteristics) -> SolverName {
        let _ = self.config.series_round; // reserved for future override tables

        // Rule 1: mountainous + many series → ILP
        if mountainous(&c) && many_series(&c) {
            return SolverName::Ilp;
        }
        // Rule 2: many series → ILP
        if many_series(&c) {
            return SolverName::Ilp;
        }
        // Rule 3: series-poor but agent-rich → LocalSearch
        if series_poor_but_agent_rich(&c) {
            return SolverName::LocalSearch;
        }
        // Rule 4: road dense → A*
        if road_dense(&c) {
            return SolverName::Astar;
        }
        // Rule 5: road sparse + mountainous → MCTS
        if road_sparse(&c) && mountainous(&c) {
            return SolverName::Mcts;
        }
        // Rule 6: road sparse → Greedy
        if road_sparse(&c) {
            return SolverName::Greedy;
        }
        // Rule 7: fallback
        default_solver()
    }

    /// Return the rule table as a deterministic `BTreeMap` keyed by
    /// characteristics. The keys are descriptive labels; values are the
    /// solvers selected by each rule. Used by introspection tooling and
    /// tests.
    #[must_use]
    pub fn rule_table() -> BTreeMap<&'static str, SolverName> {
        let mut table = BTreeMap::new();
        table.insert("mountainous+many_series", SolverName::Ilp);
        table.insert("many_series", SolverName::Ilp);
        table.insert("series_poor+agent_rich", SolverName::LocalSearch);
        table.insert("road_dense", SolverName::Astar);
        table.insert("road_sparse+mountainous", SolverName::Mcts);
        table.insert("road_sparse", SolverName::Greedy);
        table.insert("fallback", default_solver());
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Terrain;

    fn make_setting(
        width: u8,
        height: u8,
        terrain: Vec<Terrain>,
        s: u8,
        sp: u8,
        a: u8,
    ) -> MatchSetting {
        MatchSetting { width, height, terrain, series_count: s, spot_count: sp, agent_count: a }
    }

    #[test]
    fn many_series_picks_ilp() {
        let s = make_setting(4, 4, vec![Terrain::Plain; 16], 8, 32, 4);
        let p = StrategyPicker::new();
        assert_eq!(p.pick(&s), SolverName::Ilp);
    }

    #[test]
    fn series_poor_agent_rich_picks_local_search() {
        let s = make_setting(4, 4, vec![Terrain::Plain; 16], 4, 16, 8);
        let p = StrategyPicker::new();
        assert_eq!(p.pick(&s), SolverName::LocalSearch);
    }

    #[test]
    fn road_dense_picks_astar() {
        // 16 of 16 cells are roads.
        let s = make_setting(4, 4, vec![Terrain::Road; 16], 3, 8, 4);
        let p = StrategyPicker::new();
        assert_eq!(p.pick(&s), SolverName::Astar);
    }

    #[test]
    fn road_sparse_picks_greedy() {
        // 1 of 16 cells is a road → density ≈ 0.0625
        let mut terrain = vec![Terrain::Plain; 16];
        terrain[0] = Terrain::Road;
        let s = make_setting(4, 4, terrain, 3, 8, 4);
        let p = StrategyPicker::new();
        assert_eq!(p.pick(&s), SolverName::Greedy);
    }

    #[test]
    fn road_sparse_mountainous_picks_mcts() {
        // 1 of 16 cells is road; rest mountain → dense avg cost, sparse roads.
        let mut terrain = vec![Terrain::Mountain; 16];
        terrain[0] = Terrain::Road;
        let s = make_setting(4, 4, terrain, 3, 8, 4);
        let p = StrategyPicker::new();
        assert_eq!(p.pick(&s), SolverName::Mcts);
    }

    #[test]
    fn mountainous_many_series_picks_ilp() {
        let s = make_setting(2, 2, vec![Terrain::Mountain; 4], 8, 16, 4);
        let p = StrategyPicker::new();
        assert_eq!(p.pick(&s), SolverName::Ilp);
    }

    #[test]
    fn rule_table_is_complete() {
        let t = StrategyPicker::rule_table();
        assert_eq!(t.len(), 7);
        assert_eq!(t["many_series"], SolverName::Ilp);
        assert_eq!(t["fallback"], SolverName::Greedy);
    }

    #[test]
    fn pick_from_characteristics_is_consistent() {
        let c = Characteristics {
            series_count: 8,
            spot_count: 32,
            agent_count: 4,
            map_avg_cost: 1.0,
            road_density: 0.3,
        };
        let p = StrategyPicker::new();
        assert_eq!(p.pick_from_characteristics(c), SolverName::Ilp);
    }
}
