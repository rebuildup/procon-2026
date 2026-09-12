//! Tournament runner: N match settings × M solvers → leaderboard.
//!
//! See [`runner::TournamentRunner`] for the main entry point. Solvers
//! implement the [`types::Solver`] trait; per-match scores flow through
//! [`types::ScoreReport`]. Rankings use the official priority
//! (`variety → daily_variety → total → response_time`).
//!
//! EXPECTED INTEGRATION: bench
//! Sibling solver crates (issue #21 + downstream `solver-*`) plug into this
//! crate without modifying the domain layer.

pub mod ranking;
pub mod render;
pub mod runner;
pub mod types;

pub use ranking::official_cmp;
pub use render::{render_json, render_markdown, render_markdown_with_reports};
pub use runner::{
    Leaderboard, LeaderboardEntry, MatchIter, TournamentConfig, TournamentRunner,
};
pub use types::{DailyScore, MatchSetting, ScoreReport, Solver};
