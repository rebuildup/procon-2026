//! Local bench types: solver trait, scoring, and match settings.
//!
//! These are intentionally defined locally in `bench` to keep this ticket
//! self-contained while sibling crates (solver-*, engine simulator) land.
//! EXPECTED INTEGRATION: bench
//!
//! Once `engine` exposes a `Simulator` and `solver-*` crates materialise, the
//! types here should be replaced with thin aliases pointing at the canonical
//! definitions. Until then, callers can plug in their own solver
//! implementations via the [`Solver`] trait without depending on engine.

use domain::PlayerId;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A single solver under evaluation.
///
/// Solvers produce a deterministic `ScoreReport` for a given [`MatchSetting`].
/// Implementors must be cheap to construct (cloning is allowed) because the
/// tournament reuses one instance per match.
pub trait Solver {
    /// Human readable name. Used as the leaderboard key.
    fn name(&self) -> &str;

    /// Run the solver against the supplied match setting and return its score
    /// report along with the wall-clock time it took.
    ///
    /// Implementors should record best-effort wall-clock time even when the
    /// solver times out, so the leaderboard reflects real response latency.
    fn run(&self, setting: &MatchSetting) -> ScoreReport;
}

/// Match setting fed to a solver.
///
/// Lightweight placeholder; mirrors the shape of the official `試合設定用JSON`
/// but is domain-agnostic so it can be assembled without engine plumbing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchSetting {
    /// Match identifier. Echoed in [`ScoreReport::match_id`].
    pub match_id: String,
    /// Number of simulated days (`D`).
    pub days: u32,
    /// Steps allowed per day (`S`).
    pub day_steps: u32,
    /// Wall-clock budget per day in seconds (Q3 in the official Q&A).
    pub day_seconds: u32,
    /// Player this solver is playing as.
    pub player: PlayerId,
    /// Opponents paired in this match. Leaderboard win-rate is computed
    /// across `player` vs. each opponent.
    pub opponents: Vec<PlayerId>,
    /// Free-form scenario tag (e.g. `"map-3x3"`). Useful for filtering.
    pub scenario: String,
}

impl MatchSetting {
    /// Total wall-clock budget across all days (`day_seconds * days`).
    /// Solvers exceeding this are marked as time-out in the report.
    #[must_use]
    pub fn total_budget(&self) -> Duration {
        Duration::from_secs(u64::from(self.day_seconds).saturating_mul(u64::from(self.days)))
    }
}

/// Per-match scoring report.
///
/// Mirrors the four official priorities, in order:
/// 1. `variety`            — count of distinct udon series collected.
/// 2. `daily_variety`      — sum of per-day distinct series counts.
/// 3. `total`              — total udun balls collected.
/// 4. `response_time_ms`   — cumulative final-response time per day (lower is
///    better; only consulted when the first three tie).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreReport {
    pub match_id: String,
    pub solver: String,
    pub variety: u32,
    pub daily_variety: u32,
    pub total: u32,
    pub response_time_ms: u64,
    /// Wall-clock duration the solver actually consumed.
    pub elapsed: Duration,
    /// Did the solver respect the per-match budget?
    pub timed_out: bool,
}

impl ScoreReport {
    /// Convenience constructor used by synthetic fixtures and tests.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        match_id: impl Into<String>,
        solver: impl Into<String>,
        variety: u32,
        daily_variety: u32,
        total: u32,
        response_time_ms: u64,
        elapsed: Duration,
        timed_out: bool,
    ) -> Self {
        Self {
            match_id: match_id.into(),
            solver: solver.into(),
            variety,
            daily_variety,
            total,
            response_time_ms,
            elapsed,
            timed_out,
        }
    }
}

/// Aggregate of one [`ScoreReport`] per day. Solvers may populate this when
/// they have access to day-level telemetry; the tournament runner rolls it up
/// into a single [`ScoreReport`] for ranking.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyScore {
    pub day: u32,
    pub variety: u32,
    pub total: u32,
    pub response_time_ms: u64,
}
