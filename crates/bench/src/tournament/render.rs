//! Human-readable rendering of an [`AggregateReport`].
//!
//! Two renderers:
//!
//! - [`render_markdown`] — tables for a PR body, a terminal, or a notebook.
//! - [`render_json`] — pretty JSON, identical to what `aggregate.json` holds.
//!
//! Rendering is pure and deterministic: the same report always produces the
//! same text, so a rendered diff is a meaningful review artifact.

use std::fmt::Write as _;

use crate::aggregate::{AggregateReport, CandidateSummary, Outcome};

/// Render the full report as Markdown.
#[must_use]
pub fn render_markdown(report: &AggregateReport) -> String {
    let mut out = String::new();
    write_header(&mut out, report);
    write_overall_table(&mut out, report);
    write_per_case_tables(&mut out, report);
    write_failure_table(&mut out, report);
    out
}

fn write_header(out: &mut String, report: &AggregateReport) {
    let _ = writeln!(out, "# Evaluation report");
    let _ = writeln!(out);
    if report.source_commit_shas.len() == 1 {
        let _ = writeln!(out, "- commit: `{}`", report.source_commit_shas[0]);
    } else {
        // Not silently collapsed: a batch spanning commits must be obvious,
        // because the numbers underneath are not comparable across them.
        let _ = writeln!(
            out,
            "- **commits: {} ({})** — these runs are not comparable as one evidence set",
            report.source_commit_shas.len(),
            report.source_commit_shas.join(", ")
        );
    }
    match &report.baseline_candidate_id {
        Some(b) => {
            let _ = writeln!(out, "- baseline: `{b}` (win / draw / lose is measured against it)");
        }
        None => {
            let _ = writeln!(out, "- baseline: _none_ (comparisons reported as not-compared)");
        }
    }
    let _ = writeln!(out, "- candidates: {}", report.candidates.len());
    let _ = writeln!(out, "- cases: {}", report.cases.len());
    let _ = writeln!(
        out,
        "- runs: {} ({} scored)",
        report.runs.len(),
        report.runs.iter().filter(|r| r.is_scored()).count()
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Ranking order is the official lexicographic one — **variety → daily variety sum → total balls → response time (lower better)**. It is never collapsed into a weighted score. Response time is measured; wall-clock elapsed is recorded separately and never ranked."
    );
    let _ = writeln!(out);
}

fn write_overall_table(out: &mut String, report: &AggregateReport) {
    let _ = writeln!(out, "## Candidates");
    let _ = writeln!(out);
    if report.per_candidate.is_empty() {
        let _ = writeln!(out, "_no candidates_");
        let _ = writeln!(out);
        return;
    }
    let _ = writeln!(
        out,
        "| candidate | wins | draws | losses | timeouts | invalid | panics | variety (min–max) | daily sum (min–max) | total balls (min–max) | response µs (min–max) |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|---:|---:|---:|---|---|---|---|");
    for c in &report.per_candidate {
        let s = &c.score_summary;
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} | {}–{} | {}–{} | {}–{} | {}–{} |",
            c.candidate_id,
            c.wins,
            c.draws,
            c.losses,
            c.timeouts,
            c.invalids,
            c.panics,
            s.variety.min,
            s.variety.max,
            s.daily_variety_sum.min,
            s.daily_variety_sum.max,
            s.total_balls.min,
            s.total_balls.max,
            s.response_time_us.min,
            s.response_time_us.max,
        );
    }
    let _ = writeln!(out);
    if report.baseline_candidate_id.is_none() {
        let _ = writeln!(out, "_comparisons: n/a — no baseline configured._");
        let _ = writeln!(out);
    } else {
        let _ = writeln!(
            out,
            "Comparison counts cover only cases where **both** the candidate and the baseline produced a score; the rest land in `not compared`."
        );
        let _ = writeln!(out);
    }
}

fn write_per_case_tables(out: &mut String, report: &AggregateReport) {
    let _ = writeln!(out, "## Per-case ranking");
    let _ = writeln!(out);
    for case in &report.per_case {
        let _ = writeln!(out, "### `{}`", case.case_id);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "| rank | candidate | variety | daily sum | total balls | response µs | status |"
        );
        let _ = writeln!(out, "|---:|---|---:|---:|---:|---:|---|");
        for e in &case.entries {
            let _ = writeln!(
                out,
                "| {} | `{}` | {} | {} | {} | {} | {} |",
                e.rank,
                e.candidate_id,
                e.score.variety,
                e.score.daily_variety_sum,
                e.score.total_noodles,
                e.response_time_us,
                e.status,
            );
        }
        let _ = writeln!(out);
    }
}

fn write_failure_table(out: &mut String, report: &AggregateReport) {
    let failures: Vec<_> =
        report.runs.iter().filter(|r| !r.is_scored() || r.status_detail.is_some()).collect();
    if failures.is_empty() {
        return;
    }
    let _ = writeln!(out, "## Failures and warnings");
    let _ = writeln!(out);
    let _ = writeln!(out, "| candidate | case | status | detail |");
    let _ = writeln!(out, "|---|---|---|---|");
    for r in failures {
        let detail = r.status_detail.as_deref().unwrap_or("—");
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | {} |",
            r.candidate_id,
            r.case_id,
            r.status,
            escape_cell(detail),
        );
    }
    let _ = writeln!(out);
}

fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Render the report as pretty JSON.
///
/// # Errors
/// Propagates `serde_json` errors. In practice this cannot fail for
/// `AggregateReport`, which is plain data.
pub fn render_json(report: &AggregateReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

/// One-line-per-candidate summary, for a terminal.
#[must_use]
pub fn render_summary_line(c: &CandidateSummary) -> String {
    format!(
        "{:<40} W/D/L {}/{}/{}  timeouts {}  invalid {}  panics {}",
        c.candidate_id, c.wins, c.draws, c.losses, c.timeouts, c.invalids, c.panics
    )
}

/// Render the outcome of one comparison pair, for logs.
#[must_use]
pub fn render_outcome(o: Outcome) -> &'static str {
    match o {
        Outcome::Win => "win",
        Outcome::Draw => "draw",
        Outcome::Lose => "lose",
        Outcome::NotCompared => "not-compared",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::AggregateReport;
    use crate::evaluate::{RunRecord, RunStatus};

    fn record(candidate: &str, case: &str, v: u32, status: RunStatus) -> RunRecord {
        RunRecord {
            candidate_id: candidate.to_owned(),
            case_id: case.to_owned(),
            seed: None,
            generation_parameters: None,
            source_commit_sha: "abc123".to_owned(),
            team: "player-a".to_owned(),
            variety: v,
            daily_variety_sum: v * 2,
            total_balls: v * 3,
            response_time_us: 1_000,
            elapsed_us: 2_000,
            timeout: status == RunStatus::Timeout,
            status,
            scored: status != RunStatus::Timeout && status != RunStatus::Panic,
            status_detail: if status == RunStatus::Success {
                None
            } else {
                Some("something went wrong | with a pipe".to_owned())
            },
            last_acquisition_at: None,
            illegal_action_count: 0,
            fuel_short_count: 0,
            rejected_response_count: 0,
            day_budget_exceeded_days: Vec::new(),
            daily_response_us: vec![1_000],
            daily_varieties: vec![v],
            agent_kinds: None,
            agent_ball_counts: None,
            oracle_refill: false,
        }
    }

    fn sample() -> AggregateReport {
        AggregateReport::build(
            vec![
                record(
                    "loop:iterations=32:initial=greedy",
                    "fixture:small-8x8",
                    3,
                    RunStatus::Success,
                ),
                record("greedy", "fixture:small-8x8", 1, RunStatus::Success),
                record(
                    "loop:iterations=32:initial=greedy",
                    "fixture:series-few",
                    2,
                    RunStatus::Success,
                ),
                record("greedy", "fixture:series-few", 0, RunStatus::Timeout),
            ],
            Some("greedy"),
        )
        .expect("homogeneous synthetic records")
    }

    #[test]
    fn markdown_contains_the_headline_tables() {
        let md = render_markdown(&sample());
        assert!(md.contains("# Evaluation report"));
        assert!(md.contains("`loop:iterations=32:initial=greedy`"));
        assert!(md.contains("`fixture:small-8x8`"));
        assert!(md.contains("## Candidates"));
        assert!(md.contains("## Per-case ranking"));
        assert!(md.contains("## Failures and warnings"));
        // The ranking order is stated so a reader cannot mistake it for a
        // weighted score.
        assert!(md.contains("lexicographic"));
    }

    #[test]
    fn markdown_escapes_pipes_in_failure_details() {
        let md = render_markdown(&sample());
        assert!(md.contains("with a pipe"));
        assert!(!md.contains("wrong | with"));
    }

    #[test]
    fn render_is_deterministic() {
        let r = sample();
        assert_eq!(render_markdown(&r), render_markdown(&r));
    }

    #[test]
    fn json_render_matches_struct() {
        let r = sample();
        let json = render_json(&r).unwrap();
        let back: AggregateReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.candidates, r.candidates);
    }
}
