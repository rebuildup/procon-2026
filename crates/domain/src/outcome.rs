//! Match outcome / score with the official priority.
//!
//! Reference:
//! - Issue #54 (this file's origin)
//! - `docs/contest/strategy.md` §1 "勝ち筋の優先順位"
//! - Issue #20 `ScoreReport` for field name compatibility
//!
//! The official ordering (higher wins; earlier tier breaks ties):
//!
//! 1. [`Score::variety`]            — distinct udon lineages (種類の異なり数)
//! 2. [`Score::daily_variety_sum`]  — cumulative per-day variety (各日の種類数の累積)
//! 3. [`Score::total_noodles`]      — total balls harvested (うどんの総玉数)
//! 4. [`Score::total_response_time_ms`] — cumulative response time; **smaller is better**
//!
//! Ties on all four tiers resolve to [`Outcome::Draw`].

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Per-team end-of-match score. All four fields are independent statistics
/// reported by the judge; the winning order is fixed by [`compare_scores`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Score {
    /// Distinct udon lineages observed across the whole match.
    pub variety: u32,
    /// Sum of per-day distinct variety counts. Rewards teams that spread
    /// variety evenly rather than concentrating on a single day.
    pub daily_variety_sum: u32,
    /// Total number of udon balls harvested across the whole match.
    pub total_noodles: u32,
    /// Cumulative final-response timestamp across days. Lower is better.
    pub total_response_time_ms: u64,
}

impl Score {
    /// Construct a [`Score`] from already-aggregated values.
    #[must_use]
    pub const fn new(
        variety: u32,
        daily_variety_sum: u32,
        total_noodles: u32,
        total_response_time_ms: u64,
    ) -> Self {
        Self { variety, daily_variety_sum, total_noodles, total_response_time_ms }
    }

    /// Aggregate a [`Score`] from per-day observations.
    ///
    /// `daily_varieties[i]` is the distinct variety count on day `i`.
    /// `daily_response_ms[i]` is the final-response timestamp (ms) on day `i`.
    /// `variety` and `total_noodles` must be computed by the caller — they are
    /// not derivable from the per-day variety counts alone because the same
    /// lineage can appear on multiple days without inflating the global
    /// distinct count.
    #[must_use]
    pub fn from_daily(
        variety: u32,
        daily_varieties: &[u32],
        total_noodles: u32,
        daily_response_ms: &[u64],
    ) -> Self {
        let daily_variety_sum: u32 = daily_varieties.iter().sum();
        let total_response_time_ms: u64 = daily_response_ms.iter().sum();
        Self { variety, daily_variety_sum, total_noodles, total_response_time_ms }
    }
}

/// Three-way match outcome from a single team's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Outcome {
    /// This team's score is strictly better than the opponent's.
    Win,
    /// All four priority tiers compare equal.
    Draw,
    /// This team's score is strictly worse than the opponent's.
    Lose,
}

/// Compare two [`Score`] values using the official priority.
///
/// Returns `Ordering::Greater` when `a` beats `b`, `Less` when `b` beats `a`,
/// and `Equal` only when every tier (including the negated response time)
/// matches.
#[must_use]
pub fn compare_scores(a: &Score, b: &Score) -> Ordering {
    a.variety
        .cmp(&b.variety)
        .then(a.daily_variety_sum.cmp(&b.daily_variety_sum))
        .then(a.total_noodles.cmp(&b.total_noodles))
        // Response time: smaller is better → flip the comparison.
        .then(b.total_response_time_ms.cmp(&a.total_response_time_ms))
}

/// Decide [`Outcome`] for `my_team` against `opponent`.
#[must_use]
pub fn judge(my_team: &Score, opponent: &Score) -> Outcome {
    match compare_scores(my_team, opponent) {
        Ordering::Greater => Outcome::Win,
        Ordering::Equal => Outcome::Draw,
        Ordering::Less => Outcome::Lose,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(
        variety: u32,
        daily_variety_sum: u32,
        total_noodles: u32,
        total_response_time_ms: u64,
    ) -> Score {
        Score::new(variety, daily_variety_sum, total_noodles, total_response_time_ms)
    }

    #[test]
    fn tier1_variety_decides() {
        // Even though my opponent has higher cumulative variety and noodles,
        // my higher distinct-lineage count wins outright.
        let me = score(5, 2, 1, 1_000);
        let opp = score(4, 10, 100, 1_000);
        assert_eq!(judge(&me, &opp), Outcome::Win);
        assert_eq!(judge(&opp, &me), Outcome::Lose);
        assert_eq!(compare_scores(&me, &opp), Ordering::Greater);
    }

    #[test]
    fn tier2_daily_variety_sum_decides_when_variety_tied() {
        // Variety equal → daily_variety_sum breaks the tie. The team with the
        // higher cumulative per-day distinct count wins.
        let me = score(4, 9, 100, 5_000);
        let opp = score(4, 8, 100, 5_000);
        assert_eq!(judge(&me, &opp), Outcome::Win);
        assert_eq!(compare_scores(&me, &opp), Ordering::Greater);
    }

    #[test]
    fn tier3_total_noodles_decides_when_variety_and_daily_sum_tied() {
        // Variety and daily_variety_sum equal → total_noodles breaks the tie.
        let me = score(4, 8, 60, 5_000);
        let opp = score(4, 8, 59, 5_000);
        assert_eq!(judge(&me, &opp), Outcome::Win);
        assert_eq!(compare_scores(&me, &opp), Ordering::Greater);
    }

    #[test]
    fn tier4_response_time_decides_lower_is_better() {
        // First three tiers all tied → response time wins; smaller is better,
        // so even though the absolute value is "lower" numerically, the team
        // with the smaller response time is the WINNER.
        let me = score(4, 8, 50, 3_000);
        let opp = score(4, 8, 50, 4_000);
        assert_eq!(judge(&me, &opp), Outcome::Win);
        assert_eq!(judge(&opp, &me), Outcome::Lose);
        // Sanity check: compare_scores returns Greater for me (smaller response
        // time) against opp (larger response time).
        assert_eq!(compare_scores(&me, &opp), Ordering::Greater);
    }

    #[test]
    fn all_tiers_equal_is_draw() {
        let me = score(4, 8, 50, 3_000);
        let opp = score(4, 8, 50, 3_000);
        assert_eq!(judge(&me, &opp), Outcome::Draw);
        assert_eq!(compare_scores(&me, &opp), Ordering::Equal);
    }

    #[test]
    fn judge_is_antisymmetric() {
        // For any pair (a, b) where judge(a, b) != Draw, judge(b, a) must be
        // the opposite outcome.
        let a = score(5, 10, 100, 2_000);
        let b = score(4, 9, 99, 2_500);
        assert_eq!(judge(&a, &b), Outcome::Win);
        assert_eq!(judge(&b, &a), Outcome::Lose);
    }

    #[test]
    fn from_daily_aggregates_sums() {
        let daily_varieties = [3_u32, 4, 2];
        let daily_response_ms = [1_000_u64, 2_000, 1_500];
        let s = Score::from_daily(
            /* variety = */ 5,
            &daily_varieties,
            /* total_noodles = */ 42,
            &daily_response_ms,
        );
        assert_eq!(s.variety, 5);
        assert_eq!(s.daily_variety_sum, 9);
        assert_eq!(s.total_noodles, 42);
        assert_eq!(s.total_response_time_ms, 4_500);
    }

    #[test]
    fn from_daily_handles_empty_input() {
        let s = Score::from_daily(0, &[], 0, &[]);
        assert_eq!(s, Score::new(0, 0, 0, 0));
    }
}
