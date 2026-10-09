//! Aggregation over durable [`RunRecord`]s.
//!
//! Everything here is a pure function of the records — no re-running, no
//! hidden state. That is the point of persisting them: a batch can be
//! re-aggregated later with a different baseline, a different subset of
//! cases, or a different comparison policy, without touching a solver.
//!
//! Ranking and win / draw / lose use [`domain::compare_scores`] — the same
//! lexicographic order the contest uses. There is no weighted scalar hiding
//! in here.

use std::collections::BTreeMap;

use domain::{OfficialScore, compare_scores};
use serde::{Deserialize, Serialize};

use crate::evaluate::{RunRecord, RunStatus};

/// One candidate's entry in a single case's ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseRankEntry {
    /// 1-based rank on this case. Tied scores share a rank.
    pub rank: u32,
    pub candidate_id: String,
    /// Official score tuple. Zero when the run was not scored.
    ///
    /// Tier 4 here is in whole milliseconds (the official unit). The full
    /// microsecond measurement lives in [`Self::response_time_us`].
    pub score: OfficialScore,
    /// Tier 4 at full resolution, microseconds. Lower is better.
    pub response_time_us: u64,
    pub status: RunStatus,
    /// True when the run produced an authoritative score.
    pub scored: bool,
}

/// All candidates ranked on one case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseRanking {
    pub case_id: String,
    /// Best first. Unscored runs sort last.
    pub entries: Vec<CaseRankEntry>,
}

impl CaseRanking {
    /// The part of the ranking that is reproducible: rank, candidate id, and
    /// the deterministic competitive tiers (1–3).
    ///
    /// [`CaseRankEntry::response_time_us`] is a *measurement* and varies run
    /// to run even under identical conditions, so it is deliberately excluded
    /// here. Use this to assert that two batches agree; use
    /// [`Self::entries`] when you actually want the timings.
    #[must_use]
    pub fn deterministic_projection(&self) -> Vec<(u32, String, (u32, u32, u32))> {
        self.entries
            .iter()
            .map(|e| {
                (
                    e.rank,
                    e.candidate_id.clone(),
                    (e.score.variety, e.score.daily_variety_sum, e.score.total_noodles),
                )
            })
            .collect()
    }
}

/// Arithmetic mean of `sum` over `n` observations.
///
/// The cast is fine here: this is a display statistic over microsecond
/// timings, far below the point where `f64`'s 53-bit mantissa loses whole
/// units.
#[allow(clippy::cast_precision_loss, reason = "display-only mean of microsecond timings")]
fn mean_of(sum: u64, n: usize) -> f64 {
    if n == 0 { 0.0 } else { sum as f64 / n as f64 }
}

/// Min / max / mean over a set of measurements.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    pub min: u64,
    pub max: u64,
    pub mean: f64,
    /// Number of observations behind the stat.
    pub n: u32,
}

impl Stat {
    /// Compute a stat from observations. Empty input yields all-zero.
    #[must_use]
    pub fn of(values: &[u64]) -> Self {
        if values.is_empty() {
            return Self { min: 0, max: 0, mean: 0.0, n: 0 };
        }
        let min = values.iter().copied().min().unwrap_or(0);
        let max = values.iter().copied().max().unwrap_or(0);
        let n = u32::try_from(values.len()).unwrap_or(u32::MAX);
        let sum: u64 = values.iter().copied().fold(0_u64, u64::saturating_add);
        Self { min, max, mean: mean_of(sum, values.len()), n }
    }
}

/// Distribution of one candidate's official score tiers across a corpus.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoreTupleSummary {
    pub variety: Stat,
    pub daily_variety_sum: Stat,
    pub total_balls: Stat,
    /// Microseconds; lower is better.
    pub response_time_us: Stat,
    /// Wall-clock evidence, never ranked.
    pub elapsed_us: Stat,
}

impl ScoreTupleSummary {
    /// Summarise a candidate's scored runs.
    #[must_use]
    pub fn of(records: &[&RunRecord]) -> Self {
        let scored: Vec<&&RunRecord> = records.iter().filter(|r| r.is_scored()).collect();
        let pick = |f: fn(&RunRecord) -> u64| -> Vec<u64> { scored.iter().map(|r| f(r)).collect() };
        Self {
            variety: Stat::of(&pick(|r| u64::from(r.variety))),
            daily_variety_sum: Stat::of(&pick(|r| u64::from(r.daily_variety_sum))),
            total_balls: Stat::of(&pick(|r| u64::from(r.total_balls))),
            response_time_us: Stat::of(&pick(|r| r.response_time_us)),
            elapsed_us: Stat::of(&pick(|r| r.elapsed_us)),
        }
    }
}

/// Result of comparing one run against the baseline on the same case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Win,
    Draw,
    Lose,
    /// Not comparable: at least one side was not scored (timeout / panic /
    /// invalid without a score).
    NotCompared,
}

/// One candidate's roll-up across the whole corpus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateSummary {
    pub candidate_id: String,
    /// Total `(candidate, case)` pairs evaluated.
    pub runs: u32,
    /// Runs that produced an authoritative score.
    pub scored: u32,
    /// Wins / draws / losses against the baseline candidate on the same
    /// cases. Only cases where both sides are scored contribute.
    pub wins: u32,
    pub draws: u32,
    pub losses: u32,
    /// Cases where comparison was impossible because a side was unscored.
    pub not_compared: u32,
    pub timeouts: u32,
    pub invalids: u32,
    pub panics: u32,
    pub score_summary: ScoreTupleSummary,
}

/// Why an aggregation was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AggregateError {
    /// The records come from more than one commit, so a single ranking would
    /// silently mix evidence from different sources.
    #[error("records span {} commits ({}); refusing to rank across them", .found.len(), .found.join(", "))]
    MixedCommits {
        /// Every distinct commit, in first-seen order.
        found: Vec<String>,
    },
    /// No records to aggregate.
    #[error("no runs to aggregate")]
    Empty,
}

/// Complete aggregation of one batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateReport {
    /// Every distinct `source_commit_sha` among the runs, in first-seen
    /// order.
    ///
    /// Deliberately a *list*: collapsing it to the last record seen would let
    /// a mixed `runs.jsonl` masquerade as evidence from one commit. A single
    /// element is the normal case; see [`AggregateReport::single_commit`].
    pub source_commit_shas: Vec<String>,
    /// Candidate used as the win / draw / lose anchor, when one was chosen.
    pub baseline_candidate_id: Option<String>,
    /// Candidate ids, in evaluation order.
    pub candidates: Vec<String>,
    /// Case ids, in evaluation order.
    pub cases: Vec<String>,
    /// Per-case ranking, one entry per case.
    pub per_case: Vec<CaseRanking>,
    /// Per-candidate roll-up, sorted best-first by mean position across cases.
    pub per_candidate: Vec<CandidateSummary>,
    /// Every run, in evaluation order. Mirrors the JSONL file.
    pub runs: Vec<RunRecord>,
}

impl AggregateReport {
    /// Aggregate a batch of records, **refusing to mix commits**.
    ///
    /// `baseline_candidate_id` selects the candidate that win / draw / lose is
    /// measured against. It must appear in the records; passing `None` leaves
    /// every comparison as [`Outcome::NotCompared`].
    ///
    /// # Errors
    /// [`AggregateError::MixedCommits`] when the records span more than one
    /// commit, and [`AggregateError::Empty`] when there are none. Ranking two
    /// commits' evidence together would attribute one commit's scores to the
    /// other, so this is an error rather than a warning.
    pub fn build(
        runs: Vec<RunRecord>,
        baseline_candidate_id: Option<&str>,
    ) -> Result<Self, AggregateError> {
        if runs.is_empty() {
            return Err(AggregateError::Empty);
        }
        let mut candidate_order: Vec<String> = Vec::new();
        let mut case_order: Vec<String> = Vec::new();
        let mut source_commit_shas: Vec<String> = Vec::new();
        for r in &runs {
            if !source_commit_shas.contains(&r.source_commit_sha) {
                source_commit_shas.push(r.source_commit_sha.clone());
            }
            if !candidate_order.contains(&r.candidate_id) {
                candidate_order.push(r.candidate_id.clone());
            }
            if !case_order.contains(&r.case_id) {
                case_order.push(r.case_id.clone());
            }
        }
        if source_commit_shas.len() > 1 {
            return Err(AggregateError::MixedCommits { found: source_commit_shas });
        }

        let per_case = case_order.iter().map(|case_id| rank_case(case_id, &runs)).collect();

        let per_candidate = candidate_order
            .iter()
            .map(|id| summarise_candidate(id, &runs, baseline_candidate_id))
            .collect();

        Ok(Self {
            source_commit_shas,
            baseline_candidate_id: baseline_candidate_id.map(ToOwned::to_owned),
            candidates: candidate_order,
            cases: case_order,
            per_case,
            per_candidate,
            runs,
        })
    }

    /// The single commit all runs came from, when they all agree.
    #[must_use]
    pub fn single_commit(&self) -> Option<&str> {
        match self.source_commit_shas.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }

    /// Look up one candidate's summary.
    #[must_use]
    pub fn candidate(&self, id: &str) -> Option<&CandidateSummary> {
        self.per_candidate.iter().find(|c| c.candidate_id == id)
    }

    /// Look up one case's ranking.
    #[must_use]
    pub fn case(&self, case_id: &str) -> Option<&CaseRanking> {
        self.per_case.iter().find(|c| c.case_id == case_id)
    }
}

/// Rank every candidate on one case, best first.
///
/// Unscored runs (timeout / panic) sort after every scored run, ordered by
/// candidate id so the result stays deterministic.
fn rank_case(case_id: &str, runs: &[RunRecord]) -> CaseRanking {
    let mut rows: Vec<(String, OfficialScore, u64, RunStatus, bool)> = runs
        .iter()
        .filter(|r| r.case_id == case_id)
        .map(|r| {
            (r.candidate_id.clone(), r.to_score(), r.response_time_us, r.status, r.is_scored())
        })
        .collect();

    rows.sort_by(|a, b| {
        // Scored runs first; then official order; then candidate id for a
        // stable, reproducible layout.
        b.4.cmp(&a.4)
            .then_with(|| {
                if a.4 && b.4 {
                    compare_scores(&b.1, &a.1) // best first
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| a.0.cmp(&b.0))
    });

    let mut entries = Vec::with_capacity(rows.len());
    let mut prev_score: Option<OfficialScore> = None;
    let mut prev_rank = 0_u32;
    for (i, (candidate_id, score, response_time_us, status, scored)) in rows.into_iter().enumerate()
    {
        let rank = if scored && prev_score == Some(score) {
            prev_rank
        } else {
            u32::try_from(i + 1).unwrap_or(u32::MAX)
        };
        if scored {
            prev_score = Some(score);
            prev_rank = rank;
        } else {
            prev_score = None;
        }
        entries.push(CaseRankEntry { rank, candidate_id, score, response_time_us, status, scored });
    }
    CaseRanking { case_id: case_id.to_owned(), entries }
}

/// Roll one candidate up across the corpus.
fn summarise_candidate(
    candidate_id: &str,
    runs: &[RunRecord],
    baseline_candidate_id: Option<&str>,
) -> CandidateSummary {
    let mine: Vec<&RunRecord> = runs.iter().filter(|r| r.candidate_id == candidate_id).collect();

    let mut wins = 0_u32;
    let mut draws = 0_u32;
    let mut losses = 0_u32;
    let mut not_compared = 0_u32;

    if let Some(baseline) = baseline_candidate_id {
        for r in &mine {
            let other = runs.iter().find(|b| b.case_id == r.case_id && b.candidate_id == baseline);
            match other {
                Some(b) if r.is_scored() && b.is_scored() => {
                    match compare_scores(&r.to_score(), &b.to_score()) {
                        std::cmp::Ordering::Greater => wins += 1,
                        std::cmp::Ordering::Equal => draws += 1,
                        std::cmp::Ordering::Less => losses += 1,
                    }
                }
                _ => not_compared += 1,
            }
        }
    } else {
        not_compared = u32::try_from(mine.len()).unwrap_or(u32::MAX);
    }

    let count = |f: fn(&&RunRecord) -> bool| {
        u32::try_from(mine.iter().filter(|r| f(r)).count()).unwrap_or(u32::MAX)
    };
    let timeouts = count(|r| r.status == RunStatus::Timeout);
    let invalids = count(|r| r.status == RunStatus::Invalid);
    let panics = count(|r| r.status == RunStatus::Panic);
    let scored = count(|r| r.is_scored());

    CandidateSummary {
        candidate_id: candidate_id.to_owned(),
        runs: u32::try_from(mine.len()).unwrap_or(u32::MAX),
        scored,
        wins,
        draws,
        losses,
        not_compared,
        timeouts,
        invalids,
        panics,
        score_summary: ScoreTupleSummary::of(&mine),
    }
}

/// Count of records per status — the cheapest possible health check on a
/// JSONL file.
#[must_use]
pub fn status_histogram(runs: &[RunRecord]) -> BTreeMap<RunStatus, u32> {
    let mut out = BTreeMap::new();
    for r in runs {
        *out.entry(r.status).or_insert(0_u32) += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test convenience: every synthetic record shares a commit, so this
    /// cannot legitimately fail here.
    fn build(runs: Vec<RunRecord>, baseline: Option<&str>) -> AggregateReport {
        AggregateReport::build(runs, baseline).expect("homogeneous synthetic records")
    }

    fn record(candidate: &str, case: &str, v: u32, d: u32, t: u32, rt: u64) -> RunRecord {
        RunRecord {
            candidate_id: candidate.to_owned(),
            case_id: case.to_owned(),
            seed: None,
            generation_parameters: None,
            source_commit_sha: "deadbeef".to_owned(),
            team: "player-a".to_owned(),
            variety: v,
            daily_variety_sum: d,
            total_balls: t,
            response_time_us: rt,
            elapsed_us: rt + 100,
            timeout: false,
            status: RunStatus::Success,
            scored: true,
            status_detail: None,
            last_acquisition_at: None,
            illegal_action_count: 0,
            fuel_short_count: 0,
            rejected_response_count: 0,
            day_budget_exceeded_days: Vec::new(),
            daily_response_us: vec![rt],
            daily_varieties: vec![d],
            agent_kinds: None,
            agent_ball_counts: None,
            oracle_refill: false,
        }
    }

    #[test]
    fn per_case_ranking_is_lexicographic_and_best_first() {
        let runs = vec![
            // Tier 1 dominates: a better tier-2/3 cannot rescue it.
            record("weak", "case-a", 1, 9, 9, 0),
            // Tiers 1-3 equal; tier 4 (response time) breaks the tie and the
            // LOWER value wins.
            record("strong", "case-a", 2, 1, 1, 9_999),
            record("mid", "case-a", 2, 1, 1, 5_000),
        ];
        let agg = build(runs, None);
        let ranking = agg.case("case-a").unwrap();
        let order: Vec<_> = ranking.entries.iter().map(|e| e.candidate_id.as_str()).collect();
        assert_eq!(order, vec!["mid", "strong", "weak"]);
        assert_eq!(ranking.entries[0].rank, 1);
        assert_eq!(ranking.entries[2].rank, 3);
    }

    #[test]
    fn tied_scores_share_a_rank() {
        let runs = vec![
            record("alpha", "case-a", 2, 1, 1, 5_000),
            record("beta", "case-a", 2, 1, 1, 5_000),
            record("gamma", "case-a", 1, 1, 1, 5_000),
        ];
        let agg = build(runs, None);
        let ranking = agg.case("case-a").unwrap();
        // alpha/beta tie on all four tiers -> both rank 1; gamma takes 3.
        assert_eq!(ranking.entries[0].rank, 1);
        assert_eq!(ranking.entries[1].rank, 1);
        assert_eq!(ranking.entries[2].rank, 3);
    }

    #[test]
    fn win_draw_lose_is_measured_against_the_baseline() {
        let runs = vec![
            record("base", "case-a", 1, 1, 1, 0),
            record("better", "case-a", 2, 1, 1, 0),
            record("base", "case-b", 3, 3, 3, 0),
            record("better", "case-b", 3, 3, 3, 0),
            record("base", "case-c", 5, 5, 5, 0),
            record("better", "case-c", 4, 5, 5, 0),
        ];
        let agg = build(runs, Some("base"));
        let better = agg.candidate("better").unwrap();
        assert_eq!((better.wins, better.draws, better.losses), (1, 1, 1));
    }

    #[test]
    fn unscored_runs_are_counted_and_never_compared() {
        let mut timeout = record("base", "case-a", 1, 1, 1, 0);
        timeout.status = RunStatus::Timeout;
        timeout.timeout = true;
        timeout.scored = false;
        let mut panic_run = record("risky", "case-a", 0, 0, 0, 0);
        panic_run.status = RunStatus::Panic;
        panic_run.scored = false;
        let runs = vec![timeout, panic_run];
        let agg = build(runs, Some("base"));
        let risky = agg.candidate("risky").unwrap();
        assert_eq!(risky.panics, 1);
        assert_eq!(risky.scored, 0);
        assert_eq!(risky.not_compared, 1);
        // Unscored runs sort after scored ones on the case ranking.
        let ranking = agg.case("case-a").unwrap();
        assert!(!ranking.entries.iter().any(|e| e.scored));
    }

    #[test]
    fn score_tuple_summary_reports_each_tier() {
        let runs = vec![record("a", "c1", 1, 2, 3, 100), record("a", "c2", 5, 6, 7, 300)];
        let agg = build(runs, None);
        let s = &agg.candidate("a").unwrap().score_summary;
        assert_eq!((s.variety.min, s.variety.max), (1, 5));
        assert_eq!((s.total_balls.min, s.total_balls.max), (3, 7));
        assert_eq!((s.response_time_us.min, s.response_time_us.max), (100, 300));
        assert_eq!(s.variety.n, 2);
    }

    #[test]
    fn aggregate_is_reproducible_for_identical_input() {
        let make = || {
            vec![
                record("x", "c1", 1, 1, 1, 10),
                record("y", "c1", 1, 1, 1, 20),
                record("x", "c2", 2, 2, 2, 10),
                record("y", "c2", 1, 2, 2, 20),
            ]
        };
        let a = build(make(), Some("y"));
        let b = build(make(), Some("y"));
        assert_eq!(a.per_case, b.per_case);
        assert_eq!(a.per_candidate, b.per_candidate);
    }
}
