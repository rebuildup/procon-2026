//! Contest score report — a local approximation of `ScoreReport::judge()`.
//!
//! Ranks the team against the four official criteria described in
//! `docs/contest/strategy.md`, in the canonical order:
//!
//! 1. `variety` (distinct series touched, higher is better)
//! 2. `daily_variety_sum` (cumulative per-day variety, higher is better)
//! 3. `total_noodles` (cumulative balls picked up, higher is better)
//! 4. `response_time` (cumulative final-response timestamps, lower is better)
//!
//! When two solutions differ on a higher-priority criterion, the lower one
//! decides — there is no compromise. Lexicographic `Ordering` captures this
//! exactly.
//!
//! EXPECTED INTEGRATION: domain — once #57 lands, this becomes a thin shim over
//! `domain::ScoreReport::judge()`. Until then it is the source of truth for
//! "is solution A better than solution B?" inside the solver.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::types::{HexCoord, Spot};

/// Per-day observation used by the scorer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayObservation {
    /// Distinct series picked up on this day.
    pub distinct_series: u32,
    /// Total noodle balls picked up on this day.
    pub balls: u32,
    /// Final-response timestamp (in seconds-from-start) for this day.
    /// Lower means the team answered earlier.
    pub final_response_sec: u32,
}

impl DayObservation {
    #[must_use]
    pub const fn empty() -> Self {
        Self { distinct_series: 0, balls: 0, final_response_sec: 0 }
    }
}

/// Snapshot of how the team scored on a single day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyScore {
    pub day: u32,
    pub observation: DayObservation,
}

/// Aggregate score across all days. Lexicographic ordering reflects the
/// contest ranking criteria directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScoreReport {
    pub daily: Vec<DailyScore>,
}

impl ScoreReport {
    /// Distinct series touched across all days.
    /// In a stub scorer this is approximated by the union of distinct series
    /// observed each day (a daily observation cannot list which series it
    /// collected without an action trace, so this approximates the upper
    /// bound by taking the max daily distinct count).
    #[must_use]
    pub fn variety(&self) -> u32 {
        self.daily.iter().map(|d| d.observation.distinct_series).max().unwrap_or(0)
    }

    /// Cumulative per-day variety — sum of distinct series over all days.
    #[must_use]
    pub fn daily_variety_sum(&self) -> u32 {
        self.daily.iter().map(|d| d.observation.distinct_series).sum()
    }

    /// Cumulative noodle balls across all days.
    #[must_use]
    pub fn total_noodles(&self) -> u32 {
        self.daily.iter().map(|d| d.observation.balls).sum()
    }

    /// Cumulative final-response timestamps.
    /// Lower is better, so the scoring flips its sign when comparing.
    #[must_use]
    pub fn response_time(&self) -> u32 {
        self.daily.iter().map(|d| d.observation.final_response_sec).sum()
    }

    /// Local approximation of `ScoreReport::judge()`.
    ///
    /// Returns the `Ordering` that the contest judges would assign to this
    /// solution, used by the loop solver to decide whether a candidate
    /// solution is "better" than the incumbent.
    ///
    /// Sign convention: a *greater* `ScoreReport` wins. So `response_time`
    /// enters the comparison negated, since lower response time is better.
    ///
    /// Priority order (highest first):
    ///   `variety` > `daily_variety_sum` > `total_noodles` > `-response_time`
    #[must_use]
    pub fn judge(&self) -> ScoreRanking {
        ScoreRanking {
            variety: self.variety(),
            daily_variety_sum: self.daily_variety_sum(),
            total_noodles: self.total_noodles(),
            // Lower response time is better — flip so that a higher
            // negated value means "earlier", i.e. better.
            neg_response_time: u32::MAX.saturating_sub(self.response_time()),
        }
    }

    /// `true` if `self` ranks strictly better than `other` per the contest
    /// criteria.
    #[must_use]
    pub fn beats(&self, other: &Self) -> bool {
        self.judge().cmp(&other.judge()) == Ordering::Greater
    }
}

/// Tuple-shaped ranking key used for lexicographic comparison.
///
/// Ord is derived field-by-field in the order `variety`, `daily_variety_sum`,
/// `total_noodles`, `neg_response_time`. This matches the contest ranking
/// criteria in `docs/contest/strategy.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreRanking {
    pub variety: u32,
    pub daily_variety_sum: u32,
    pub total_noodles: u32,
    pub neg_response_time: u32,
}

impl Ord for ScoreRanking {
    fn cmp(&self, other: &Self) -> Ordering {
        // First three fields: higher wins.
        self.variety
            .cmp(&other.variety)
            .then(self.daily_variety_sum.cmp(&other.daily_variety_sum))
            .then(self.total_noodles.cmp(&other.total_noodles))
            // Last field: higher `neg_response_time` = earlier answer = better.
            .then(self.neg_response_time.cmp(&other.neg_response_time))
    }
}

impl PartialOrd for ScoreRanking {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Simulate a day's plan and produce a [`DailyScore`].
///
/// This is a deterministic stub: it does not run a real game simulation
/// (the engine crate is not merged yet — see #57). Instead it inspects
/// the *shape* of the plan and approximates the daily observations:
///
/// - `distinct_series` = distinct series in the spots visited that day
/// - `balls`            = 1 ball per visited spot (1-day-1-spot-1-ball rule)
/// - `final_response_sec` = number of steps taken (proxy for response time)
///
/// EXPECTED INTEGRATION: domain — `domain::engine::simulate_day()` once #57 lands.
#[must_use]
pub fn simulate_day(day: u32, plan: &[u32], spots: &[Spot], _agent_start: HexCoord) -> DailyScore {
    let mut distinct_series = std::collections::BTreeSet::new();
    let mut balls: u32 = 0;
    for spot_id in plan {
        if let Some(spot) = spots.iter().find(|s| s.id == *spot_id) {
            distinct_series.insert(spot.series.clone());
            balls = balls.saturating_add(1); // each visit = 1 ball
        }
    }
    // step count proxy: monotonically increases; one tick per visit.
    let final_response_sec = u32::try_from(plan.len()).unwrap_or(u32::MAX);
    DailyScore {
        day,
        observation: DayObservation {
            distinct_series: u32::try_from(distinct_series.len()).unwrap_or(u32::MAX),
            balls,
            final_response_sec,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Series;

    fn spot(id: u32, q: i32, r: i32, series: &str) -> Spot {
        Spot { id, coord: HexCoord::new(q, r), series: Series::new(series), stock: 1 }
    }

    #[test]
    fn judge_orders_by_variety_first() {
        // Same everything else, but solution B touches more series -> B wins.
        let mut a = ScoreReport::default();
        a.daily.push(DailyScore {
            day: 0,
            observation: DayObservation { distinct_series: 1, balls: 5, final_response_sec: 10 },
        });
        let mut b = ScoreReport::default();
        b.daily.push(DailyScore {
            day: 0,
            observation: DayObservation { distinct_series: 2, balls: 5, final_response_sec: 10 },
        });
        assert!(b.beats(&a));
        assert!(!a.beats(&b));
    }

    #[test]
    fn judge_orders_by_response_time_last() {
        // Same variety / daily_variety_sum / total_noodles — earlier answer wins.
        let mut a = ScoreReport::default();
        a.daily.push(DailyScore {
            day: 0,
            observation: DayObservation { distinct_series: 1, balls: 1, final_response_sec: 100 },
        });
        let mut b = ScoreReport::default();
        b.daily.push(DailyScore {
            day: 0,
            observation: DayObservation { distinct_series: 1, balls: 1, final_response_sec: 50 },
        });
        assert!(b.beats(&a));
    }

    #[test]
    fn judge_does_not_compromise_higher_priority_criterion() {
        // Even if B has way better response_time, A wins on variety.
        let mut a = ScoreReport::default();
        a.daily.push(DailyScore {
            day: 0,
            observation: DayObservation { distinct_series: 5, balls: 1, final_response_sec: 1000 },
        });
        let mut b = ScoreReport::default();
        b.daily.push(DailyScore {
            day: 0,
            observation: DayObservation { distinct_series: 1, balls: 100, final_response_sec: 0 },
        });
        assert!(a.beats(&b));
    }

    #[test]
    fn simulate_day_counts_distinct_series_and_balls() {
        let spots = vec![spot(0, 0, 0, "kake"), spot(1, 1, 0, "kake"), spot(2, 2, 0, "bukkake")];
        let day = simulate_day(0, &[0, 1, 2], &spots, HexCoord::new(0, 0));
        assert_eq!(day.observation.distinct_series, 2);
        assert_eq!(day.observation.balls, 3);
    }
}
