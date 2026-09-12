//! Benchmark harness for contest solvers.
//!
//! Provides a [`Solver`] trait and a [`BenchHarness`] runner that produces
//! [`ScoreReport`]s comparable under the official contest priority order
//! (variety > daily variety sum > total noodles > cumulative response time).
//!
//! This crate is intentionally minimal so it can land before the full
//! `engine` simulator (issue #18) and the richer domain types (issues
//! #11/#12/#13). Today the harness drives a solver through every
//! `(day, step)` combination and records how long the solver took to
//! respond; once the simulator exists, [`BenchHarness::run_with_simulator`]
//! will fill in the scoring fields from the simulator's per-day results.

#![doc(test(attr(deny(warnings))))]

use std::fmt::{self, Display, Formatter};
use std::path::Path;
use std::time::{Duration, Instant};

use domain::{MatchId, PlayerId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-exports so downstream crates can pin a single dependency on `bench`
// when wiring reports / replays together. Keeps the `protocol` and
// `engine` deps non-cosmetic.
pub use protocol::PROTOCOL_VERSION;
pub use {engine as engine_reexport, protocol as protocol_reexport};

/// A single solver action. This is a stub enum; once the full domain types
/// from issues #11/#12/#13 land, this will be replaced by a richer action
/// set covering movement, supply, and pickup intents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Do nothing for this step.
    Noop,
    /// Move along a heading. `heading` is encoded as a `u8` (0..=5) to keep
    /// the type wire-stable before the real action vocabulary exists.
    Move { heading: u8 },
}

/// Per-step view passed to a [`Solver`] when it is asked to decide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionContext {
    pub match_id: MatchId,
    pub day: u32,
    pub step: u32,
    pub day_steps: u32,
    pub day_seconds: u32,
    pub seed: u64,
    pub agents: Vec<PlayerId>,
}

/// Static description of a match used by the bench harness to drive a solver.
///
/// Mirrors the JSON file format described in the contest problem statement
/// (`docs/pdf/.../試合設定用JSONファイル`). Until the full match format
/// lands, this is a deliberately small subset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchSetting {
    pub match_id: MatchId,
    pub days: u32,
    pub day_steps: u32,
    /// Per-day response budget, in seconds (Q&A Q3).
    pub day_seconds: u32,
    pub seed: u64,
    pub agents: Vec<PlayerId>,
}

impl MatchSetting {
    /// Load a [`MatchSetting`] from a JSON file on disk.
    pub fn load(path: &Path) -> Result<Self, BenchError> {
        let raw = std::fs::read_to_string(path).map_err(BenchError::Io)?;
        serde_json::from_str(&raw).map_err(BenchError::Json)
    }
}

/// Score produced by running a single solver against a single [`MatchSetting`].
///
/// Fields mirror the official judging order (descending):
///   1. `variety`                  — distinct noodle series touched across the match (highest wins)
///   2. `daily_variety_sum`        — sum of per-day variety counts          (highest wins)
///   3. `total_noodles`            — total balls collected across the match (highest wins)
///   4. `total_response_time_ms`   — sum of per-day final-answer times     (lowest wins)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreReport {
    pub solver: String,
    pub variety: u32,
    pub daily_variety_sum: u32,
    pub total_noodles: u32,
    pub total_response_time_ms: u64,
}

impl ScoreReport {
    /// Construct a fresh, all-zero report for the given solver name.
    #[must_use]
    pub fn zero(solver: impl Into<String>) -> Self {
        Self {
            solver: solver.into(),
            variety: 0,
            daily_variety_sum: 0,
            total_noodles: 0,
            total_response_time_ms: 0,
        }
    }

    /// Add a per-day contribution to the cumulative fields.
    ///
    /// `day_variety` is taken as the maximum of any single day for the
    /// headline `variety` field — that matches the official "うどんの種類数"
    /// metric as the best single-day variety achieved.
    pub fn add_day(&mut self, day_variety: u32, day_noodles: u32, day_response_ms: u64) {
        if day_variety > self.variety {
            self.variety = day_variety;
        }
        self.daily_variety_sum = self.daily_variety_sum.saturating_add(day_variety);
        self.total_noodles = self.total_noodles.saturating_add(day_noodles);
        self.total_response_time_ms = self.total_response_time_ms.saturating_add(day_response_ms);
    }
}

impl Display for ScoreReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== ScoreReport ({}) ===", self.solver)?;
        writeln!(f, "  variety              : {}", self.variety)?;
        writeln!(f, "  daily_variety_sum    : {}", self.daily_variety_sum)?;
        writeln!(f, "  total_noodles        : {}", self.total_noodles)?;
        writeln!(f, "  total_response_time  : {} ms", self.total_response_time_ms)?;
        Ok(())
    }
}

/// Three-way outcome of a 1v1 comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Win,
    Lose,
    Draw,
}

impl Display for Outcome {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Win => "win",
            Self::Lose => "lose",
            Self::Draw => "draw",
        };
        f.write_str(s)
    }
}

/// Compare two [`ScoreReport`]s under the official contest priority order.
///
/// The official judging order from the problem statement is, descending:
///   1. `variety` (higher wins)
///   2. `daily_variety_sum` (higher wins)
///   3. `total_noodles` (higher wins)
///   4. `total_response_time_ms` (lower wins)
///
/// The first criterion where one side is strictly better decides the
/// outcome. If all four are tied, the result is a draw.
#[must_use]
pub fn judge(my: &ScoreReport, opp: &ScoreReport) -> Outcome {
    let higher_wins = |a: u64, b: u64| match a.cmp(&b) {
        std::cmp::Ordering::Greater => Outcome::Win,
        std::cmp::Ordering::Less => Outcome::Lose,
        std::cmp::Ordering::Equal => Outcome::Draw,
    };
    let lower_wins = |a: u64, b: u64| match a.cmp(&b) {
        std::cmp::Ordering::Less => Outcome::Win,
        std::cmp::Ordering::Greater => Outcome::Lose,
        std::cmp::Ordering::Equal => Outcome::Draw,
    };

    for cmp in [
        higher_wins(u64::from(my.variety), u64::from(opp.variety)),
        higher_wins(u64::from(my.daily_variety_sum), u64::from(opp.daily_variety_sum)),
        higher_wins(u64::from(my.total_noodles), u64::from(opp.total_noodles)),
        lower_wins(my.total_response_time_ms, opp.total_response_time_ms),
    ] {
        if cmp != Outcome::Draw {
            return cmp;
        }
    }
    Outcome::Draw
}

/// Solver abstraction used by the bench harness.
///
/// Concrete solvers live in downstream crates; this trait is intentionally
/// minimal so it can be implemented before the full simulator (issue #18)
/// and the richer domain types (issues #11/#12/#13) land.
pub trait Solver {
    /// Stable display name. Used as the `solver` field in the [`ScoreReport`].
    fn name(&self) -> &'static str;

    /// Produce the per-agent actions for the supplied [`DecisionContext`].
    fn decide(&self, ctx: &DecisionContext) -> Vec<Action>;
}

/// Bench harness entry point.
///
/// Constructing a harness is cheap; it holds no state itself.
#[derive(Debug, Default, Clone, Copy)]
pub struct BenchHarness;

impl BenchHarness {
    /// Construct a new harness.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Run a solver against a match setting.
    ///
    /// Today this drives the solver through every `(day, step)` combination
    /// and records the actions it produces. Scoring fields stay at zero —
    /// until the full simulator (issue #18) plugs into
    /// [`BenchHarness::run_with_simulator`], per-day variety / noodle
    /// counts are not measurable in this crate.
    #[allow(
        clippy::unused_self,
        clippy::trivially_copy_pass_by_ref,
        reason = "API contract from issue #20 takes &self; harness gains state later."
    )]
    pub fn run(&self, solver: &dyn Solver, setting: &MatchSetting) -> ScoreReport {
        let mut report = ScoreReport::zero(solver.name());
        let mut elapsed = Duration::ZERO;
        for day in 0..setting.days {
            for step in 0..setting.day_steps {
                let ctx = DecisionContext {
                    match_id: setting.match_id.clone(),
                    day,
                    step,
                    day_steps: setting.day_steps,
                    day_seconds: setting.day_seconds,
                    seed: setting.seed,
                    agents: setting.agents.clone(),
                };
                let started = Instant::now();
                let _ = solver.decide(&ctx);
                elapsed += started.elapsed();
            }
        }
        // Wall-clock time spent inside `decide` is a stand-in until the
        // simulator measures response time against the per-day timer.
        // Saturate rather than truncate so the report stays meaningful even
        // when wall-clock measurement dwarfs u64::MAX (e.g. debug builds).
        report.total_response_time_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        report
    }

    /// Run a solver through the simulator.
    ///
    /// Placeholder until issue #18 (`MatchSim`) lands; today it forwards to
    /// [`BenchHarness::run`] and emits a warning-level log so callers know
    /// they are exercising the stub path.
    #[allow(
        clippy::unused_self,
        clippy::trivially_copy_pass_by_ref,
        reason = "API contract from issue #20 takes &self; harness gains state later."
    )]
    pub fn run_with_simulator(&self, solver: &dyn Solver, setting: &MatchSetting) -> ScoreReport {
        // TODO(simulator, #18): drive `engine::MatchSim` here and fill the
        // scoring fields from the simulator's per-day results.
        tracing::warn!(
            "BenchHarness::run_with_simulator is a stub until engine#18 lands; \
             falling back to run()"
        );
        self.run(solver, setting)
    }
}

/// Library-wide error type.
#[derive(Debug, Error)]
pub enum BenchError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience: pretty-print a [`ScoreReport`] alongside the JSON form so
/// downstream tooling can choose between formats.
pub fn to_pretty_json(report: &ScoreReport) -> Result<String, BenchError> {
    Ok(serde_json::to_string_pretty(report)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with(
        solver: &str,
        variety: u32,
        daily_variety_sum: u32,
        total_noodles: u32,
        total_response_time_ms: u64,
    ) -> ScoreReport {
        ScoreReport {
            solver: solver.to_owned(),
            variety,
            daily_variety_sum,
            total_noodles,
            total_response_time_ms,
        }
    }

    #[test]
    fn judge_exact_tie_is_draw() {
        let a = report_with("a", 5, 12, 100, 50);
        let b = report_with("b", 5, 12, 100, 50);
        assert_eq!(judge(&a, &b), Outcome::Draw);
    }

    #[test]
    fn judge_variety_dominates_all_other_fields() {
        // My variety is higher; every other field is worse. I must still win.
        let my = report_with("mine", 6, 1, 1, u64::MAX);
        let opp = report_with("opp", 5, 99, 99, 0);
        assert_eq!(judge(&my, &opp), Outcome::Win);
    }

    #[test]
    fn judge_lower_variety_loses_even_with_better_other_fields() {
        let my = report_with("mine", 4, 0, 0, 0);
        let opp = report_with("opp", 5, 100, 100, 1000);
        assert_eq!(judge(&my, &opp), Outcome::Lose);
    }

    #[test]
    fn judge_daily_variety_sum_breaks_tie_when_variety_equal() {
        let my = report_with("mine", 5, 12, 1, u64::MAX);
        let opp = report_with("opp", 5, 11, u32::MAX, 0);
        assert_eq!(judge(&my, &opp), Outcome::Win);
    }

    #[test]
    fn judge_total_noodles_breaks_tie_when_higher_fields_equal() {
        let my = report_with("mine", 5, 12, 100, u64::MAX);
        let opp = report_with("opp", 5, 12, 99, 0);
        assert_eq!(judge(&my, &opp), Outcome::Win);
    }

    #[test]
    fn judge_response_time_breaks_tie_when_other_fields_equal() {
        let my = report_with("mine", 5, 12, 100, 10);
        let opp = report_with("opp", 5, 12, 100, 20);
        assert_eq!(judge(&my, &opp), Outcome::Win);
    }

    #[test]
    fn judge_response_time_favors_lower_not_higher() {
        // Sanity check: in this priority slot the *lower* value wins.
        let my = report_with("mine", 5, 12, 100, 100);
        let opp = report_with("opp", 5, 12, 100, 50);
        assert_eq!(judge(&my, &opp), Outcome::Lose);
    }

    #[test]
    fn judge_skips_response_time_when_already_decided_by_variety() {
        // Verifies the priority order is strictly descending: response time
        // is never consulted if variety already decided the outcome.
        let my = report_with("mine", 6, 100, 100, u64::MAX);
        let opp = report_with("opp", 5, 0, 0, 0);
        assert_eq!(judge(&my, &opp), Outcome::Win);
    }

    #[test]
    fn judge_skips_response_time_when_already_decided_by_noodles() {
        // Same as above but at the noodles tier.
        let my = report_with("mine", 5, 12, 100, u64::MAX);
        let opp = report_with("opp", 5, 12, 50, 0);
        assert_eq!(judge(&my, &opp), Outcome::Win);
    }

    #[test]
    fn report_serialization_round_trips_json() {
        let r = report_with("solver-x", 3, 8, 42, 1234);
        let json = serde_json::to_string(&r).unwrap();
        let back: ScoreReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn report_pretty_print_includes_solver_name_and_all_metrics() {
        let r = report_with("alpha", 1, 2, 3, 4);
        let printed = format!("{r}");
        assert!(printed.contains("alpha"));
        assert!(printed.contains("variety"));
        assert!(printed.contains("daily_variety_sum"));
        assert!(printed.contains("total_noodles"));
        assert!(printed.contains("total_response_time"));
    }

    #[test]
    fn to_pretty_json_is_valid_pretty_json() {
        let r = report_with("alpha", 1, 2, 3, 4);
        let pretty = to_pretty_json(&r).unwrap();
        // Pretty-printed JSON contains newlines + indentation.
        assert!(pretty.contains('\n'));
        assert!(pretty.contains("  "));
        // And round-trips back to the same report.
        let back: ScoreReport = serde_json::from_str(&pretty).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn report_add_day_accumulates_correctly() {
        let mut r = ScoreReport::zero("alpha");
        r.add_day(2, 10, 100);
        r.add_day(3, 5, 50);
        assert_eq!(r.variety, 3);
        assert_eq!(r.daily_variety_sum, 5);
        assert_eq!(r.total_noodles, 15);
        assert_eq!(r.total_response_time_ms, 150);
    }

    #[test]
    fn add_day_variety_tracks_max_not_sum() {
        // If a later day is weaker in variety, `variety` must not regress.
        let mut r = ScoreReport::zero("alpha");
        r.add_day(5, 1, 1);
        r.add_day(2, 1, 1);
        assert_eq!(r.variety, 5);
        assert_eq!(r.daily_variety_sum, 7);
    }

    #[test]
    fn harness_runs_solver_and_records_name() {
        struct Dummy;
        impl Solver for Dummy {
            fn name(&self) -> &'static str {
                "dummy"
            }
            fn decide(&self, _: &DecisionContext) -> Vec<Action> {
                vec![Action::Noop]
            }
        }
        let setting = MatchSetting {
            match_id: MatchId::new("match-test").unwrap(),
            days: 1,
            day_steps: 2,
            day_seconds: 60,
            seed: 1,
            agents: vec![PlayerId::new("p1").unwrap()],
        };
        let report = BenchHarness::new().run(&Dummy, &setting);
        assert_eq!(report.solver, "dummy");
    }

    #[test]
    fn harness_runs_solver_per_day_and_per_step() {
        // Solver that records how many times `decide` was called.
        use std::cell::Cell;
        struct Counting {
            calls: Cell<u32>,
        }
        impl Solver for Counting {
            fn name(&self) -> &'static str {
                "counting"
            }
            fn decide(&self, _: &DecisionContext) -> Vec<Action> {
                self.calls.set(self.calls.get() + 1);
                vec![]
            }
        }
        let setting = MatchSetting {
            match_id: MatchId::new("match-counting").unwrap(),
            days: 3,
            day_steps: 4,
            day_seconds: 60,
            seed: 0,
            agents: vec![PlayerId::new("p1").unwrap()],
        };
        let solver = Counting { calls: Cell::new(0) };
        let _ = BenchHarness::new().run(&solver, &setting);
        assert_eq!(solver.calls.get(), 3 * 4);
    }

    #[test]
    fn harness_supports_multiple_solvers_via_trait_object() {
        // Two distinct solvers share the harness dispatch path.
        struct Alpha;
        impl Solver for Alpha {
            fn name(&self) -> &'static str {
                "alpha"
            }
            fn decide(&self, _: &DecisionContext) -> Vec<Action> {
                vec![Action::Noop]
            }
        }
        struct Beta;
        impl Solver for Beta {
            fn name(&self) -> &'static str {
                "beta"
            }
            fn decide(&self, _: &DecisionContext) -> Vec<Action> {
                vec![Action::Move { heading: 3 }]
            }
        }
        let setting = MatchSetting {
            match_id: MatchId::new("match-multi").unwrap(),
            days: 1,
            day_steps: 1,
            day_seconds: 60,
            seed: 0,
            agents: vec![PlayerId::new("p1").unwrap()],
        };
        let a = BenchHarness::new().run(&Alpha, &setting);
        let b = BenchHarness::new().run(&Beta, &setting);
        assert_eq!(a.solver, "alpha");
        assert_eq!(b.solver, "beta");
    }

    #[test]
    fn match_setting_loads_from_json_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("example_match.json");
        let setting = MatchSetting::load(&path).expect("fixture must load");
        assert_eq!(setting.days, 2);
        assert_eq!(setting.day_steps, 100);
        assert_eq!(setting.day_seconds, 60);
        assert_eq!(setting.seed, 42);
        assert_eq!(setting.agents.len(), 2);
    }

    #[test]
    fn match_setting_round_trips_via_json() {
        let setting = MatchSetting {
            match_id: MatchId::new("match-rt").unwrap(),
            days: 2,
            day_steps: 64,
            day_seconds: 60,
            seed: 7,
            agents: vec![PlayerId::new("p1").unwrap(), PlayerId::new("p2").unwrap()],
        };
        let json = serde_json::to_string(&setting).unwrap();
        let back: MatchSetting = serde_json::from_str(&json).unwrap();
        assert_eq!(setting, back);
    }

    #[test]
    fn judge_display_strings() {
        assert_eq!(format!("{}", Outcome::Win), "win");
        assert_eq!(format!("{}", Outcome::Lose), "lose");
        assert_eq!(format!("{}", Outcome::Draw), "draw");
    }
}

// Tournament runner module (issue #21). Types live in `bench::tournament::*`
// to keep them namespaced from the scoring harness types defined above.
pub mod tournament;
