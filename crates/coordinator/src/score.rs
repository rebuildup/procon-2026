//! Stub of the future `bench::ScoreReport`.
//!
//! `bench::ScoreReport` (Issue #20) will own the canonical scoring metrics that
//! mirror the official ranking criteria. Until that crate merges, the
//! coordinator carries a minimum compatible stub so the selector can be
//! developed and tested in isolation.
//!
//! EXPECTED INTEGRATION: bench
//! - When `crates/bench` lands, this module will be deleted and the
//!   coordinator will `use bench::{ScoreReport, judge}` instead. The
//!   shape of the four ranking criteria (うどん種類数 / 累積種類数 /
//!   総玉数 / 累積回答時刻) must remain identical to the field set below
//!   so existing callers and tests keep compiling.
//! - `judge()` here implements the official priority order described in
//!   `docs/contest/strategy.md` §1.

use serde::{Deserialize, Serialize};

/// Stub score report mirroring the future `bench::ScoreReport`.
///
/// Field order is deliberate and matches the official ranking priority:
/// 1. うどんの種類数 (number of distinct series collected)
/// 2. 累積種類数 (cumulative distinct-series count across all days)
/// 3. 総玉数 (total balls collected)
/// 4. 累積回答時刻 (cumulative final-answer timestamp; smaller is better)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StubScoreReport {
    /// Solver identifier (e.g. "greedy", "random", "loop").
    pub solver_id: String,
    /// Number of distinct udon series collected.
    pub distinct_series: u32,
    /// Cumulative distinct-series count across days.
    pub cumulative_distinct_series: u32,
    /// Total balls collected.
    pub total_balls: u32,
    /// Cumulative final-answer timestamp; smaller is better.
    pub cumulative_answer_time_ms: u64,
    /// Wall-clock duration the solver actually spent (ms). Reported for
    /// observability; not part of the ranking.
    pub solver_runtime_ms: u64,
}

impl StubScoreReport {
    /// Construct a stub score report from a solver id and raw metrics.
    #[must_use]
    pub fn new(
        solver_id: impl Into<String>,
        distinct_series: u32,
        cumulative_distinct_series: u32,
        total_balls: u32,
        cumulative_answer_time_ms: u64,
        solver_runtime_ms: u64,
    ) -> Self {
        Self {
            solver_id: solver_id.into(),
            distinct_series,
            cumulative_distinct_series,
            total_balls,
            cumulative_answer_time_ms,
            solver_runtime_ms,
        }
    }

    /// Convenience: a zero-score report, useful for tests.
    #[must_use]
    pub fn zero(solver_id: impl Into<String>) -> Self {
        Self::new(solver_id, 0, 0, 0, 0, 0)
    }
}

/// Judge two [`StubScoreReport`]s and return the winner.
///
/// Implements the official priority order from `docs/contest/strategy.md` §1:
///
/// 1. Larger `distinct_series` wins.
/// 2. Tie-break by larger `cumulative_distinct_series`.
/// 3. Tie-break by larger `total_balls`.
/// 4. Tie-break by **smaller** `cumulative_answer_time_ms`.
///
/// Returns `Some((winner, loser))` if `a` outranks `b`. Returns
/// `None` if `b` outranks or ties `a`.
///
/// This is the `coordinator`-side placeholder for the future
/// `bench::ScoreReport::judge` method (Issue #20). When `bench` lands,
/// `EnsembleSelector` will switch to `ScoreReport::judge` and this helper
/// is removed.
///
/// EXPECTED INTEGRATION: bench
/// - When `bench::ScoreReport::judge` becomes available, route calls to
///   that implementation and drop this function.
#[must_use]
pub fn judge(
    a: &StubScoreReport,
    b: &StubScoreReport,
) -> Option<(StubScoreReport, StubScoreReport)> {
    if outranks(a, b) { Some((a.clone(), b.clone())) } else { None }
}

/// Return `true` iff `a` strictly outranks `b` under the official
/// priority order. Ties return `false`.
#[must_use]
pub fn outranks(a: &StubScoreReport, b: &StubScoreReport) -> bool {
    if a.distinct_series != b.distinct_series {
        return a.distinct_series > b.distinct_series;
    }
    if a.cumulative_distinct_series != b.cumulative_distinct_series {
        return a.cumulative_distinct_series > b.cumulative_distinct_series;
    }
    if a.total_balls != b.total_balls {
        return a.total_balls > b.total_balls;
    }
    // Smaller cumulative answer time wins.
    a.cumulative_answer_time_ms < b.cumulative_answer_time_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_series_is_top_priority() {
        let hi = StubScoreReport::new("hi", 5, 0, 0, 1_000, 0);
        let lo = StubScoreReport::new("lo", 4, 999, 999, 0, 0);
        assert!(outranks(&hi, &lo));
        assert!(judge(&hi, &lo).is_some());
        assert!(judge(&lo, &hi).is_none());
    }

    #[test]
    fn cumulative_distinct_series_breaks_first_tie() {
        let hi = StubScoreReport::new("hi", 3, 10, 0, 1_000, 0);
        let lo = StubScoreReport::new("lo", 3, 9, 999, 0, 0);
        assert!(outranks(&hi, &lo));
    }

    #[test]
    fn total_balls_breaks_second_tie() {
        let hi = StubScoreReport::new("hi", 3, 10, 5, 1_000, 0);
        let lo = StubScoreReport::new("lo", 3, 10, 4, 0, 0);
        assert!(outranks(&hi, &lo));
    }

    #[test]
    fn answer_time_breaks_final_tie_with_smaller_winning() {
        let hi = StubScoreReport::new("hi", 3, 10, 5, 100, 0);
        let lo = StubScoreReport::new("lo", 3, 10, 5, 200, 0);
        assert!(outranks(&hi, &lo));
    }

    #[test]
    fn identical_scores_do_not_outrank() {
        let a = StubScoreReport::new("a", 3, 10, 5, 100, 0);
        let b = StubScoreReport::new("b", 3, 10, 5, 100, 0);
        assert!(!outranks(&a, &b));
        assert!(judge(&a, &b).is_none());
    }
}
