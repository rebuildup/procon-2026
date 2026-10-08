//! `solver-loop` — iterative improvement loop solver.
//!
//! Generates a greedy initial tour through the input spots, then refines it
//! with 2-opt / 3-opt swaps. Improvement is evaluated against the contest
//! ranking criteria (`variety` → `daily_variety_sum` → `total_noodles` →
//! `response_time`) rather than raw tour length, because the contest judges
//! rank on the four-criteria tuple, not on tour cost alone.
//!
//! EXPECTED INTEGRATION: domain — once #57 lands, the local stubs in
//! [`types`] are replaced by `domain::*`, the score report is delegated to
//! `domain::ScoreReport::judge()`, and the day simulation in [`score`] is
//! delegated to `domain::engine::simulate_day()`.
//!
//! Crate layout:
//! - [`types`]      : minimal stubs for `HexCoord`, `Spot`, `Series`, `Agent`, `Action`.
//! - [`score`]      : local approximation of `ScoreReport::judge()` + day simulation.
//! - [`two_opt`]    : 2-opt swap primitive + best-improvement scan.
//! - [`three_opt`]  : 3-opt swap primitive (the classic single move, plus
//!   all seven classical moves for `operators=extended`).
//! - [`or_opt`]     : segment relocation of length ≤ 3 (`operators=extended`).
//! - [`solver`]     : the [`Solver`] trait stub + the [`LoopSolver`] implementation.
//! - [`multi_loop`] : [`MultiLoopComposer`] — cluster spots into regions and build one
//!   independent loop per region. Used for parallel coverage of large maps.

#![doc(test(attr(deny(warnings))))]

pub mod dual_direction;
pub mod multi_loop;
pub mod or_opt;
pub mod score;
pub mod solver;
pub mod three_opt;
pub mod two_opt;
pub mod types;

pub use dual_direction::{Direction, DualDirectionLoop, ccw_position, cw_position, score_dual};
pub use multi_loop::{ClusteringStrategy, MultiLoopComposer, SurplusMode};
pub use or_opt::{OR_OPT_MAX_LEN, apply_or_opt, apply_or_opt_pass, gap_is_legal, or_opt_delta};
pub use score::{DailyScore, DayObservation, ScoreRanking, ScoreReport, simulate_day};
pub use solver::{
    InitialStrategy, LoopSolver, OperatorSet, SolveOutcome, SolveProblem, SolveStats, Solver,
    SolverError,
};
pub use three_opt::{
    THREE_OPT_MOVE_COUNT, THREE_OPT_MOVE_NAMES, apply_three_opt_move, three_opt_deltas,
    try_three_opt, try_three_opt_full,
};
pub use two_opt::{apply_two_opt, best_two_opt_move, edge_cost, tour_cost, two_opt_delta};
pub use types::{Action, Agent, HexCoord, Series, Spot, Terrain};

/// Strategy name used by the contest's external solver selector.
///
/// The contest pipeline picks one of several named strategies per match.
/// `INITIAL` is the solver-loop variant — the iterative-improvement loop
/// described in this crate. Other crates may register additional names.
pub const INITIAL: &str = "solver-loop::loop-2opt-3opt";
