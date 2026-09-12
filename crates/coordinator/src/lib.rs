//! Per-match approach picker.
//!
//! Maps match-setting characteristics (series count, spot count, agent
//! count, average traversal cost, road density) to the most appropriate
//! solver. This crate is the home for the *picker*, not the solver
//! implementations themselves; once `engine` / `bench` / `solver-*` land, the
//! chosen [`SolverName`] will dispatch to the real binary.
//!
//! ## Architecture
//!
//! ```text
//! MatchSetting
//!      │
//!      ▼
//! features::extract  ──►  Characteristics
//!      │
//!      ▼
//! picker::StrategyPicker
//!      │
//!      ▼
//! SolverName
//! ```
//!
//! Reference: `docs/contest/strategy.md` §5 (アルゴリズム設計).

#![doc(test(attr(deny(warnings))))]

pub mod features;
pub mod picker;
pub mod rules;
pub mod types;

pub use features::{Characteristics, extract};
pub use picker::{PickerConfig, StrategyPicker};
pub use rules::{
    AGENT_RICH_THRESHOLD, MOUNTAIN_AVG_COST, ROAD_DENSE_THRESHOLD, ROAD_SPARSE_THRESHOLD,
    SERIES_HIGH_THRESHOLD,
};
pub use types::{MatchSetting, SolverName, Terrain};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Terrain;

    #[test]
    fn end_to_end_pick_pipeline() {
        let setting = MatchSetting {
            width: 4,
            height: 4,
            terrain: vec![Terrain::Plain; 16],
            series_count: 8,
            spot_count: 32,
            agent_count: 4,
        };
        let chars = extract(&setting);
        let picker = StrategyPicker::new();
        assert_eq!(picker.pick(&setting), picker.pick_from_characteristics(chars));
    }

    #[test]
    fn public_reexports_resolve() {
        let _: PickerConfig = PickerConfig::default();
        let _: SolverName = SolverName::Greedy;
        let _: Terrain = Terrain::Plain;
    }
}
