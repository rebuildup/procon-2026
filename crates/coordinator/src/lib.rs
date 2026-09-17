//! Coordinator: per-match approach picker + ensemble selector.
//!
//! Strategy reference: `docs/contest/strategy.md` §5 (solver selection)
//! and §7 (ensemble / voting).
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
//!      │
//!      ▼
//! EnsembleSelector (best-of / majority-vote / weighted-by-score)
//! ```
//!
//! `coordinator` is the small orchestration crate that, given a set of
//! solvers (`greedy`, `random`, `loop`, ...), runs them in parallel on the
//! same `MatchInput` and picks a winner under one of three modes:
//!
//! - [`EnsembleMode::BestOf`]          — single highest-scored plan wins.
//! - [`EnsembleMode::MajorityVote`]    — action-sequence majority.
//! - [`EnsembleMode::WeightedByScore`] — score-weighted votes.

#![doc(test(attr(deny(warnings))))]

pub mod ensemble;
pub mod features;
pub mod picker;
pub mod rules;
pub mod score;
pub mod types;
pub mod voting;

pub use ensemble::{EnsembleMode, EnsembleResult, EnsembleSelector, RegisteredSolver};
pub use features::{Characteristics, extract};
pub use picker::{PickerConfig, StrategyPicker};
pub use rules::{
    AGENT_RICH_THRESHOLD, MOUNTAIN_AVG_COST, ROAD_DENSE_THRESHOLD, ROAD_SPARSE_THRESHOLD,
    SERIES_HIGH_THRESHOLD,
};
pub use score::{StubScoreReport, judge, outranks};
pub use types::{
    Action, ActionSequence, MatchInput, MatchSetting, Solver, SolverName, SolverOutput, Terrain,
};
pub use voting::{VoteTally, plan_key, tally, weighted_tally};

/// Library-wide error type. The selector itself is infallible today; this
/// exists for forward-compatibility with future solver traits that can
/// fail (e.g. timeout-mapped to errors).
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    /// A solver thread panicked or otherwise failed to produce output.
    #[error("solver failed: {0}")]
    SolverFailure(String),
}

/// Library-wide result alias.
pub type CoordinatorResult<T> = Result<T, CoordinatorError>;

#[cfg(test)]
mod tests {
    use super::*;

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
        let _: EnsembleMode = EnsembleMode::BestOf;
        let _: Box<dyn Solver> = Box::new(StubSolver);
    }

    /// Minimal solver for the public-reexports compile-time check.
    struct StubSolver;
    impl Solver for StubSolver {
        fn id(&self) -> &'static str {
            "stub"
        }
        fn solve(&self, _input: &MatchInput) -> SolverOutput {
            SolverOutput { score: StubScoreReport::zero("stub"), plan: ActionSequence::empty() }
        }
    }
}
