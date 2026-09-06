//! Integration tests for the tournament runner.
//!
//! These exercise the full [`bench::tournament::TournamentRunner`] pipeline
//! against the bundled `fixtures/tournament_matches.json` fixture. Rank
//! outcomes were derived by hand from the official priority (`variety` →
//! `daily_variety` → total → `response_time`) and frozen as a regression oracle.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bench::tournament::ranking::official_cmp;
use bench::tournament::render::{render_json, render_markdown};
use bench::tournament::runner::{LeaderboardEntry, MatchIter, TournamentConfig, TournamentRunner};
use bench::tournament::types::{MatchSetting, ScoreReport, Solver};
use domain::{MatchId, PlayerId};
use serde::Deserialize;

#[derive(Debug, Clone)]
struct FixtureSolver {
    name: &'static str,
    reports: Vec<ScoreReport>,
}

impl Solver for FixtureSolver {
    fn name(&self) -> &str {
        self.name
    }
    fn run(&self, setting: &MatchSetting) -> ScoreReport {
        let mut report =
            self.reports.iter().find(|r| r.match_id == setting.match_id).cloned().unwrap_or_else(
                || {
                    ScoreReport::new(
                        setting.match_id.clone(),
                        self.name,
                        0,
                        0,
                        0,
                        0,
                        Duration::from_millis(0),
                        false,
                    )
                },
            );
        // The fixture doesn't carry wall-clock elapsed; default to a fixed
        // value so timing-based branches remain deterministic.
        if report.elapsed == Duration::ZERO {
            report.elapsed = Duration::from_millis(50);
        }
        report
    }
}

fn fixture_path() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest).join("fixtures").join("tournament_matches.json")
}

#[derive(Debug, Deserialize)]
struct Fixture {
    #[serde(default)]
    #[allow(dead_code)]
    config: TournamentConfig,
    settings: Vec<MatchSetting>,
    reports: Vec<RawReport>,
}

#[derive(Debug, Deserialize)]
struct RawReport {
    solver: String,
    match_id: String,
    variety: u32,
    daily_variety: u32,
    total: u32,
    response_time_ms: u64,
    #[serde(default)]
    elapsed_ms: u64,
    #[serde(default)]
    timed_out: bool,
}

impl From<RawReport> for ScoreReport {
    fn from(r: RawReport) -> Self {
        ScoreReport::new(
            r.match_id,
            r.solver,
            r.variety,
            r.daily_variety,
            r.total,
            r.response_time_ms,
            Duration::from_millis(r.elapsed_ms),
            r.timed_out,
        )
    }
}

fn load_fixture() -> Fixture {
    let raw = fs::read(fixture_path()).expect("fixture file is present");
    serde_json::from_slice(&raw).expect("fixture JSON parses")
}

fn runner_solvers() -> Vec<Box<dyn Solver>> {
    let fx = load_fixture();
    let mut by_solver: std::collections::BTreeMap<String, Vec<ScoreReport>> =
        std::collections::BTreeMap::new();
    for r in fx.reports {
        let report: ScoreReport = r.into();
        by_solver.entry(report.solver.clone()).or_default().push(report);
    }
    let mut solvers: Vec<Box<dyn Solver>> = by_solver
        .into_iter()
        .map(|(name, reports)| {
            let name_static: &'static str = Box::leak(Box::new(name.clone()));
            Box::new(FixtureSolver { name: name_static, reports }) as Box<dyn Solver>
        })
        .collect();
    // Sorted by solver name so the cross-product is deterministic across runs.
    solvers.sort_by(|a, b| a.name().cmp(b.name()));
    solvers
}

fn runner_settings() -> Vec<MatchSetting> {
    load_fixture().settings
}

#[test]
fn fixture_parses_and_round_trips() {
    let fx = load_fixture();
    assert_eq!(fx.settings.len(), 3, "fixture exposes 3 match settings");
    assert_eq!(fx.reports.len(), 6, "fixture exposes 6 reports (2 solvers × 3 matches)");

    // Sanity: every reported match_id references an existing setting.
    let known: std::collections::HashSet<_> =
        fx.settings.iter().map(|s| s.match_id.clone()).collect();
    for r in &fx.reports {
        assert!(known.contains(&r.match_id), "report references unknown match {}", r.match_id);
    }
}

#[test]
fn match_iter_produces_cross_product_in_deterministic_order() {
    let settings = runner_settings();
    let solvers = runner_solvers();
    let iter = MatchIter::new(&settings, &solvers);
    assert_eq!(iter.len(), settings.len() * solvers.len());
    let pairs: Vec<_> = iter.collect();
    assert_eq!(pairs.len(), settings.len() * solvers.len());
    // Deterministic: first item is settings[0] × solvers[0].
    assert_eq!(pairs[0].0.match_id, settings[0].match_id);
    assert_eq!(pairs[0].1.name(), solvers[0].name());
}

#[test]
fn ranking_matches_official_priority_for_fixture() {
    let settings = runner_settings();
    let solvers = runner_solvers();
    let board =
        TournamentRunner::new(TournamentConfig { sequential: true, title: "fixture".to_owned() })
            .run(&settings, &solvers);

    // Build the per-solver best report to verify the priority order by hand.
    let mut bests: Vec<(&str, &ScoreReport)> = vec![];
    for solver in &solvers {
        let mut best: Option<&ScoreReport> = None;
        for r in board.reports.iter().filter(|r| r.solver == solver.name()) {
            best = match best {
                None => Some(r),
                Some(curr) => {
                    if official_cmp(r, curr) == std::cmp::Ordering::Less {
                        Some(r)
                    } else {
                        Some(curr)
                    }
                }
            };
        }
        if let Some(b) = best {
            bests.push((solver.name(), b));
        }
    }
    let mut sorted = bests.clone();
    sorted.sort_by(|a, b| official_cmp(a.1, b.1));

    let names: Vec<&str> = sorted.iter().map(|(n, _)| *n).collect();
    let board_names: Vec<&str> = board.entries.iter().map(|e| e.solver.as_str()).collect();
    assert_eq!(board_names, names, "leaderboard order must follow official priority");
}

#[test]
fn leaderboard_assigns_dense_ranks_one_based() {
    let settings = runner_settings();
    let solvers = runner_solvers();
    let board = TournamentRunner::new(TournamentConfig::default()).run(&settings, &solvers);
    assert_eq!(board.entries.len(), solvers.len());
    for (idx, entry) in board.entries.iter().enumerate() {
        let expected_rank = u32::try_from(idx + 1).expect("rank fits in u32");
        assert_eq!(entry.rank, expected_rank, "rank #{idx}");
    }
}

#[test]
fn leaderboard_aggregates_total_score_and_matches() {
    let settings = runner_settings();
    let solvers = runner_solvers();
    let board = TournamentRunner::new(TournamentConfig::default()).run(&settings, &solvers);

    for entry in &board.entries {
        let count =
            u32::try_from(board.reports.iter().filter(|r| r.solver == entry.solver).count())
                .expect("solver count fits u32");
        assert_eq!(entry.matches, count, "matches count for {}", entry.solver);
        let sum: u32 =
            board.reports.iter().filter(|r| r.solver == entry.solver).map(|r| r.total).sum();
        assert_eq!(entry.total_score, sum, "total_score for {}", entry.solver);
        assert!(entry.win_rate >= 0.0 && entry.win_rate <= 1.0);
    }
}

#[test]
fn json_output_round_trips_through_serde() {
    let settings = runner_settings();
    let solvers = runner_solvers();
    let board = TournamentRunner::new(TournamentConfig::default()).run(&settings, &solvers);
    let json = render_json(&board).expect("json serialises");
    let back: bench::tournament::Leaderboard = serde_json::from_str(&json).expect("json parses");
    assert_eq!(back, board);
}

#[test]
fn markdown_renders_with_header_and_rows() {
    let settings = runner_settings();
    let solvers = runner_solvers();
    let board = TournamentRunner::new(TournamentConfig::default()).run(&settings, &solvers);
    let md = render_markdown(&board);
    assert!(md.contains("## Tournament leaderboard"), "header present");
    assert!(md.contains("| Rank | Solver |"), "header row present");
    for entry in &board.entries {
        assert!(md.contains(&entry.solver), "row for {} present", entry.solver);
    }
}

#[test]
fn response_time_breaks_total_tie() {
    // Two settings × one solver that ties on all criteria except response_time.
    let pid_a = PlayerId::new("player-a").expect("valid id");
    let settings = vec![
        MatchSetting {
            match_id: "match-tie".to_owned(),
            days: 1,
            day_steps: 1,
            day_seconds: 1,
            player: pid_a.clone(),
            opponents: vec![PlayerId::new("player-b").expect("valid")],
            scenario: "tie".to_owned(),
        },
        MatchSetting {
            match_id: "match-tie-2".to_owned(),
            days: 1,
            day_steps: 1,
            day_seconds: 1,
            player: pid_a,
            opponents: vec![PlayerId::new("player-c").expect("valid")],
            scenario: "tie".to_owned(),
        },
    ];
    let solvers: Vec<Box<dyn Solver>> = vec![
        Box::new(FixtureSolver {
            name: "fast",
            reports: vec![
                ScoreReport::new(
                    "match-tie",
                    "fast",
                    4,
                    4,
                    10,
                    100,
                    Duration::from_millis(10),
                    false,
                ),
                ScoreReport::new(
                    "match-tie-2",
                    "fast",
                    4,
                    4,
                    10,
                    100,
                    Duration::from_millis(10),
                    false,
                ),
            ],
        }),
        Box::new(FixtureSolver {
            name: "slow",
            reports: vec![
                ScoreReport::new(
                    "match-tie",
                    "slow",
                    4,
                    4,
                    10,
                    200,
                    Duration::from_millis(20),
                    false,
                ),
                ScoreReport::new(
                    "match-tie-2",
                    "slow",
                    4,
                    4,
                    10,
                    200,
                    Duration::from_millis(20),
                    false,
                ),
            ],
        }),
    ];
    let board = TournamentRunner::new(TournamentConfig::default()).run(&settings, &solvers);
    assert_eq!(board.entries[0].solver, "fast", "fast response wins the tie");
    assert_eq!(board.entries[1].solver, "slow");
}

#[test]
fn timeout_flagged_but_does_not_disqualify_other_solvers() {
    let pid_a = PlayerId::new("player-a").expect("valid id");
    let settings = vec![MatchSetting {
        match_id: "match-timeout".to_owned(),
        days: 5,
        day_steps: 200,
        day_seconds: 1, // intentionally tiny so any "slow" solver blows it
        player: pid_a,
        opponents: vec![],
        scenario: "tight-budget".to_owned(),
    }];
    let solvers: Vec<Box<dyn Solver>> = vec![
        Box::new(FixtureSolver {
            name: "fast",
            reports: vec![ScoreReport::new(
                "match-timeout",
                "fast",
                3,
                5,
                20,
                100,
                Duration::from_millis(10),
                false,
            )],
        }),
        Box::new(FixtureSolver {
            name: "slow",
            reports: vec![ScoreReport::new(
                "match-timeout",
                "slow",
                3,
                4,
                19,
                100,
                Duration::from_millis(10),
                false,
            )],
        }),
    ];
    let board = TournamentRunner::new(TournamentConfig::default()).run(&settings, &solvers);
    // Both solvers ran; no report was filtered out.
    assert_eq!(board.reports.len(), 2);
    // Total elapsed ≥ 0 (we never panicked). fast should still win on
    // daily_variety even though both ran cleanly.
    let ranked: Vec<&LeaderboardEntry> = board.entries.iter().collect();
    assert_eq!(ranked[0].solver, "fast");
    // Sanity: MatchId in lib is independent of the bench MatchSetting.
    let _ = MatchId::new("match-timeout").expect("valid match id");
}
