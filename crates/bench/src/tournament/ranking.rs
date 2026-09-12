//! Official ranking logic for [`ScoreReport`].
//!
//! Priority (descending), per `docs/contest/strategy.md` §1 + acceptance
//! criteria of issue #21:
//!  1. `variety` (descending)        — distinct udon series collected.
//!  2. `daily_variety` (descending)  — accumulated per-day distinct series.
//!  3. `total` (descending)          — total balls.
//!  4. `response_time_ms` (ascending) — cumulative final-response latency.

use super::types::ScoreReport;
use std::cmp::Ordering;

/// Compare two score reports under the official priority.
///
/// Returns `Ordering::Less` when `a` outranks `b`.
#[must_use]
pub fn official_cmp(a: &ScoreReport, b: &ScoreReport) -> Ordering {
    // Higher variety first.
    match b.variety.cmp(&a.variety) {
        Ordering::Equal => {}
        ord => return ord,
    }
    // Higher cumulative daily variety next.
    match b.daily_variety.cmp(&a.daily_variety) {
        Ordering::Equal => {}
        ord => return ord,
    }
    // Total balls (last "real" criterion).
    match b.total.cmp(&a.total) {
        Ordering::Equal => {}
        ord => return ord,
    }
    // Earlier response wins.
    a.response_time_ms.cmp(&b.response_time_ms)
}

/// True if `a` strictly outranks `b`.
#[must_use]
pub fn a_wins(a: &ScoreReport, b: &ScoreReport) -> bool {
    official_cmp(a, b) == Ordering::Less
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn report(variety: u32, daily: u32, total: u32, rt: u64) -> ScoreReport {
        ScoreReport::new("m", "s", variety, daily, total, rt, Duration::ZERO, false)
    }

    #[test]
    fn variety_beats_daily_variety() {
        let hi = report(5, 0, 0, 0);
        let lo = report(4, 99, 99, 0);
        assert_eq!(official_cmp(&hi, &lo), Ordering::Less);
    }

    #[test]
    fn daily_variety_breaks_variety_tie() {
        let hi = report(4, 10, 0, 0);
        let lo = report(4, 9, 0, 0);
        assert_eq!(official_cmp(&hi, &lo), Ordering::Less);
    }

    #[test]
    fn total_breaks_daily_tie() {
        let hi = report(4, 10, 50, 0);
        let lo = report(4, 10, 49, 0);
        assert_eq!(official_cmp(&hi, &lo), Ordering::Less);
    }

    #[test]
    fn response_time_breaks_total_tie() {
        let hi = report(4, 10, 50, 100);
        let lo = report(4, 10, 50, 200);
        assert_eq!(official_cmp(&hi, &lo), Ordering::Less);
    }

    #[test]
    fn identical_reports_tie() {
        let a = report(3, 7, 42, 1500);
        let b = report(3, 7, 42, 1500);
        assert_eq!(official_cmp(&a, &b), Ordering::Equal);
    }
}
