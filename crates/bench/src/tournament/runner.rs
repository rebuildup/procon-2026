//! Tournament runner: N match settings × M solvers → leaderboard.
//!
//! # Overview
//!
//! [`TournamentRunner`] composes [`MatchSetting`]s and [`Solver`]s, evaluates
//! each pair sequentially (parallelism lives one layer up — see issue #20's
//! `BenchHarness`) and produces a [`Leaderboard`] plus a flat
//! `Vec<([Solver; Report])` of every per-match [`ScoreReport`].
//!
//! The runner is deterministic with respect to a solver's output: identical
//! inputs + identical solver implementations produce identical
//! `Leaderboard`s. Parallel evaluation is opt-in via
//! [`TournamentConfig::sequential`]; parallel support is intentionally out of
//! scope for the first PR per issue #21 acceptance criteria.
//!
//! # Example
//!
//! ```rust
//! use bench::tournament::types::{MatchSetting, ScoreReport};
//! use bench::tournament::runner::{TournamentRunner, TournamentConfig};
//! use domain::PlayerId;
//! use std::time::Duration;
//!
//! struct ConstSolver(&'static str, u32, u32, u32);
//! impl bench::tournament::types::Solver for ConstSolver {
//!     fn name(&self) -> &str { self.0 }
//!     fn run(&self, setting: &MatchSetting) -> ScoreReport {
//!         ScoreReport::new(setting.match_id.clone(), self.name(),
//!             self.1, self.2, self.3, 100, Duration::from_millis(100), false)
//!     }
//! }
//!
//! let settings = vec![
//!     MatchSetting { match_id: "m1".into(), days: 2, day_steps: 30, day_seconds: 60,
//!         player: PlayerId::new("player-a").unwrap(), opponents: vec![PlayerId::new("player-b").unwrap()],
//!         scenario: "tiny".into() },
//! ];
//! let runners: Vec<Box<dyn bench::tournament::types::Solver>> = vec![
//!     Box::new(ConstSolver("alpha", 3, 5, 20)),
//!     Box::new(ConstSolver("beta",  2, 6, 25)),
//! ];
//! let board = TournamentRunner::new(TournamentConfig::default())
//!     .run(&settings, &runners);
//! assert_eq!(board.entries[0].solver, "alpha");
//! ```

use super::ranking::official_cmp;
use super::types::{MatchSetting, ScoreReport, Solver};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum TournamentError {
    #[error("no match settings supplied")]
    NoMatches,
    #[error("no solvers supplied")]
    NoSolvers,
    #[error("a solver reported scores for the wrong match: expected {expected}, got {actual}")]
    MismatchedMatchId { expected: String, actual: String },
}

/// Runtime configuration for the tournament.
///
/// Currently exposes a `sequential` toggle for forward compatibility; the
/// runner is sequential today. The flag is parsed from fixtures so downstream
/// integrations (CLI subcommand, GitHub Actions) can request parallel eval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentConfig {
    pub sequential: bool,
    /// Optional label used in the leaderboard header.
    #[serde(default)]
    pub title: String,
}

impl Default for TournamentConfig {
    fn default() -> Self {
        Self { sequential: true, title: String::new() }
    }
}

/// One row of the leaderboard.
///
/// `total_score` here is the *sum of `total`* across that solver's matches
/// (matches the JSON payload expected by downstream dashboards). `rank` is
/// 1-based per the official priority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub solver: String,
    pub rank: u32,
    pub total_score: u32,
    pub matches: u32,
    /// Number of matches the solver was credited a win for.
    pub wins: u32,
    /// `wins / matches`. `Draw` count + opponent reports are aggregated in
    /// the raw report vector; only the win rate is exposed in the board.
    pub win_rate: f64,
    /// Best (highest) per-match score seen for this solver, as a `ScoreReport`
    /// snapshot. Useful for sanity checking the headline row.
    pub best: ScoreReport,
}

/// Aggregated leaderboard output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leaderboard {
    pub config: TournamentConfig,
    pub entries: Vec<LeaderboardEntry>,
    /// Every per-match report produced during the tournament. Useful when
    /// debugging the run or feeding downstream dashboards.
    pub reports: Vec<ScoreReport>,
    /// Wall-clock time the runner took, end-to-end.
    pub elapsed_ms: u64,
}

/// Iterator-style view of the `(setting, solver)` pairs the runner consumes.
///
/// Exposed publicly so tests can verify the cross-product shape without
/// instantiating the full runner.
#[derive(Clone)]
pub struct MatchIter<'a> {
    settings: &'a [MatchSetting],
    solvers: &'a [Box<dyn Solver>],
    cursor: usize,
}

impl<'a> MatchIter<'a> {
    /// Construct a new iterator over the cross product of `settings × solvers`.
    #[must_use]
    pub fn new(settings: &'a [MatchSetting], solvers: &'a [Box<dyn Solver>]) -> Self {
        Self { settings, solvers, cursor: 0 }
    }

    /// Total number of `(setting, solver)` pairs the iterator will yield.
    #[must_use]
    pub fn len(&self) -> usize {
        self.settings.len().saturating_mul(self.solvers.len())
    }

    /// True if the cross product is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a> Iterator for MatchIter<'a> {
    type Item = (MatchSetting, &'a dyn Solver);

    fn next(&mut self) -> Option<Self::Item> {
        let total = self.len();
        if self.cursor >= total {
            return None;
        }
        let s_idx = self.cursor / self.solvers.len().max(1);
        let p_idx = self.cursor % self.solvers.len().max(1);
        self.cursor += 1;
        let setting = self.settings.get(s_idx)?.clone();
        let solver = self.solvers.get(p_idx)?.as_ref();
        Some((setting, solver))
    }
}

/// Tournament runner façade.
#[derive(Debug, Default, Clone)]
pub struct TournamentRunner {
    config: TournamentConfig,
}

impl TournamentRunner {
    #[must_use]
    pub fn new(config: TournamentConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn config(&self) -> &TournamentConfig {
        &self.config
    }

    /// Plan the cross product of `(settings, solvers)` without running them.
    #[must_use]
    pub fn plan<'a>(settings: &'a [MatchSetting], solvers: &'a [Box<dyn Solver>]) -> MatchIter<'a> {
        MatchIter::new(settings, solvers)
    }

    /// Execute the tournament and produce a [`Leaderboard`].
    ///
    /// Solvers are called sequentially in `MatchIter` order. Per-pair timing is
    /// captured into [`ScoreReport::elapsed`]; solvers exceeding
    /// [`MatchSetting::total_budget`] are flagged via [`ScoreReport::timed_out`].
    #[must_use]
    pub fn run(&self, settings: &[MatchSetting], solvers: &[Box<dyn Solver>]) -> Leaderboard {
        let start = Instant::now();
        let mut reports = Vec::with_capacity(settings.len().saturating_mul(solvers.len()));
        let mut skipped: Vec<(String, String)> = Vec::new();

        for (setting, solver) in Self::plan(settings, solvers) {
            let started = Instant::now();
            let mut report = solver.run(&setting);
            if report.elapsed == std::time::Duration::ZERO {
                report.elapsed = started.elapsed();
            }
            if report.match_id != setting.match_id {
                tracing::warn!(
                    solver = %solver.name(),
                    expected = %setting.match_id,
                    actual = %report.match_id,
                    "solver returned mismatched match_id, skipping",
                );
                skipped.push((solver.name().to_owned(), setting.match_id.clone()));
                continue;
            }
            if report.timed_out || report.elapsed > setting.total_budget() {
                report.timed_out = true;
                tracing::warn!(
                    solver = %solver.name(),
                    match_id = %setting.match_id,
                    "solver exceeded budget, marking as timed_out",
                );
            }
            reports.push(report);
        }

        let entries = aggregate_leaderboard(solvers, &reports);
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Leaderboard { config: self.config.clone(), entries, reports, elapsed_ms }
    }
}

/// Reduce per-match reports into [`LeaderboardEntry`]s.
///
/// Sorts by official priority (`variety → daily → total → response_time`),
/// then assigns dense ranks.
fn aggregate_leaderboard(
    solvers: &[Box<dyn Solver>],
    reports: &[ScoreReport],
) -> Vec<LeaderboardEntry> {
    let mut aggregates: Vec<AggRow> = solvers
        .iter()
        .map(|s| AggRow {
            solver: s.name().to_owned(),
            total_score: 0,
            variety_total: 0,
            daily_variety_total: 0,
            response_time_total: 0,
            matches: 0,
            wins: 0,
            best: None,
        })
        .collect();

    for report in reports {
        let Some(row) = aggregates.iter_mut().find(|r| r.solver == report.solver) else {
            // Solver that produced a report but is no longer in the registry;
            // keep the report but ignore the row. The leaderboard keeps the
            // report intact in `Leaderboard::reports`.
            continue;
        };
        row.matches = row.matches.saturating_add(1);
        row.total_score = row.total_score.saturating_add(report.total);
        row.variety_total = row.variety_total.saturating_add(report.variety);
        row.daily_variety_total = row.daily_variety_total.saturating_add(report.daily_variety);
        row.response_time_total = row.response_time_total.saturating_add(report.response_time_ms);
        let won = is_self_win(report);
        if won {
            row.wins = row.wins.saturating_add(1);
        }
        match &row.best {
            None => row.best = Some(report.clone()),
            Some(current) => {
                if official_cmp(report, current) == std::cmp::Ordering::Less {
                    row.best = Some(report.clone());
                }
            }
        }
    }

    let mut rows: Vec<AggRow> = aggregates.into_iter().filter(|r| r.matches > 0).collect();
    // Compare by best per-match report (the canonical scorecard per solver).
    rows.sort_by(|a, b| match (&a.best, &b.best) {
        (Some(x), Some(y)) => official_cmp(x, y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    rows.sort_by(|a, b| {
        // Tie-break on aggregate response time when best scores tie.
        match a.best.as_ref().zip(b.best.as_ref()) {
            Some((x, y)) if official_cmp(x, y) == std::cmp::Ordering::Equal => {
                a.response_time_total.cmp(&b.response_time_total)
            }
            _ => std::cmp::Ordering::Equal,
        }
    });

    rows.into_iter()
        .enumerate()
        .map(|(idx, row)| {
            let win_rate =
                if row.matches == 0 { 0.0 } else { f64::from(row.wins) / f64::from(row.matches) };
            let solver = row.solver.clone();
            LeaderboardEntry {
                solver: row.solver,
                rank: u32::try_from(idx + 1).unwrap_or(u32::MAX),
                total_score: row.total_score,
                matches: row.matches,
                wins: row.wins,
                win_rate,
                best: row.best.unwrap_or_else(|| {
                    ScoreReport::new(
                        "<empty>",
                        solver,
                        0,
                        0,
                        0,
                        0,
                        std::time::Duration::ZERO,
                        false,
                    )
                }),
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct AggRow {
    solver: String,
    total_score: u32,
    variety_total: u32,
    daily_variety_total: u32,
    response_time_total: u64,
    matches: u32,
    wins: u32,
    best: Option<ScoreReport>,
}

/// "Did the solver's own player win?" — placeholder until the engine
/// simulator exposes per-match outcomes. For now we treat any non-timeout
/// report with positive scoring as a synthetic `Win`. Real cross-team
/// detection will land alongside the simulator in #18.
fn is_self_win(report: &ScoreReport) -> bool {
    !report.timed_out && (report.variety > 0 || report.daily_variety > 0 || report.total > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::PlayerId;
    use std::time::Duration;

    fn setting(id: &str) -> MatchSetting {
        MatchSetting {
            match_id: id.to_owned(),
            days: 2,
            day_steps: 30,
            day_seconds: 60,
            player: PlayerId::new("player-a").unwrap(),
            opponents: vec![PlayerId::new("player-b").unwrap()],
            scenario: "tiny".to_owned(),
        }
    }

    #[derive(Debug)]
    struct Const(&'static str, u32, u32, u32, u64);
    impl Solver for Const {
        fn name(&self) -> &str {
            self.0
        }
        fn run(&self, s: &MatchSetting) -> ScoreReport {
            ScoreReport::new(
                s.match_id.clone(),
                self.0,
                self.1,
                self.2,
                self.3,
                self.4,
                Duration::from_millis(self.4),
                false,
            )
        }
    }

    #[test]
    fn match_iter_crosses_product() {
        let s = vec![setting("m1"), setting("m2")];
        let solvers: Vec<Box<dyn Solver>> =
            vec![Box::new(Const("a", 0, 0, 0, 0)), Box::new(Const("b", 0, 0, 0, 0))];
        let mut iter = MatchIter::new(&s, &solvers);
        assert_eq!(iter.len(), 4);
        let mut count = 0;
        while iter.next().is_some() {
            count += 1;
        }
        assert_eq!(count, 4);
    }

    #[test]
    fn ranking_orders_by_official_priority() {
        let s = vec![setting("m1")];
        // "best" outranks "mid" outranks "low" under official priority.
        let solvers: Vec<Box<dyn Solver>> = vec![
            Box::new(Const("low", 1, 5, 10, 200)),
            Box::new(Const("best", 3, 5, 20, 100)),
            Box::new(Const("mid", 2, 6, 25, 100)),
        ];
        let board = TournamentRunner::default().run(&s, &solvers);
        let names: Vec<_> = board.entries.iter().map(|e| e.solver.clone()).collect();
        assert_eq!(names, vec!["best", "mid", "low"]);
        assert_eq!(board.entries[0].rank, 1);
        assert_eq!(board.entries[1].rank, 2);
        assert_eq!(board.entries[2].rank, 3);
    }
}
