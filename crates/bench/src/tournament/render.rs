//! Markdown / JSON renderers for the tournament leaderboard.
//!
//! The Markdown output is meant to be dropped straight into a PR comment or
//! GitHub Projects card. Column widths are derived per-render so the table
//! stays aligned regardless of solver name length.

use super::runner::{Leaderboard, LeaderboardEntry};
use std::fmt::Write;

/// Render the leaderboard as a Markdown table.
#[must_use]
pub fn render_markdown(board: &Leaderboard) -> String {
    let mut out = String::new();
    let title = if board.config.title.is_empty() {
        "Tournament leaderboard".to_owned()
    } else {
        board.config.title.clone()
    };
    writeln!(out, "## {title}").expect("writing to String cannot fail");
    writeln!(out).expect("writing to String cannot fail");
    writeln!(out, "_Elapsed: {} ms · reports: {}_", board.elapsed_ms, board.reports.len())
        .expect("writing to String cannot fail");
    writeln!(out).expect("writing to String cannot fail");

    let headers = [
        "Rank",
        "Solver",
        "Total",
        "Wins/Matches",
        "Win%",
        "Variety",
        "Daily",
        "Balls",
        "Response (ms)",
    ];
    let rows: Vec<[String; 9]> = board
        .entries
        .iter()
        .map(|e| {
            [
                e.rank.to_string(),
                e.solver.clone(),
                e.total_score.to_string(),
                format!("{}/{}", e.wins, e.matches),
                format!("{:.1}", e.win_rate * 100.0),
                e.best.variety.to_string(),
                e.best.daily_variety.to_string(),
                e.best.total.to_string(),
                e.best.response_time_ms.to_string(),
            ]
        })
        .collect();

    if rows.is_empty() {
        writeln!(out, "_No solvers reported a result._").expect("writing to String cannot fail");
        return out;
    }

    let widths = column_widths(&headers, &rows);
    write_row(&mut out, &headers, &widths);
    write_separator(&mut out, &widths);
    for row in &rows {
        write_row_owned(&mut out, row, &widths);
    }
    writeln!(out).expect("writing to String cannot fail");
    out
}

/// Render the leaderboard as pretty JSON.
pub fn render_json(board: &Leaderboard) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(board)
}

/// Convenience: render as a human-friendly Markdown block including each
/// individual [`crate::types::ScoreReport`] below the leaderboard.
#[must_use]
pub fn render_markdown_with_reports(board: &Leaderboard) -> String {
    let mut out = render_markdown(board);
    out.push_str("\n### Per-match reports\n\n");
    out.push_str("| match | solver | variety | daily | total | response_ms | timed_out |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for r in &board.reports {
        writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            r.match_id,
            r.solver,
            r.variety,
            r.daily_variety,
            r.total,
            r.response_time_ms,
            r.timed_out
        )
        .expect("writing to String cannot fail");
    }
    out
}

fn column_widths(headers: &[&str; 9], rows: &[[String; 9]]) -> [usize; 9] {
    let mut widths = [0usize; 9];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = h.chars().count();
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    widths
}

fn write_row(out: &mut String, row: &[&str], widths: &[usize; 9]) {
    out.push('|');
    for (i, cell) in row.iter().enumerate() {
        let w = widths[i];
        let cell_chars = cell.chars().count();
        if cell_chars > w {
            // Truncate at the column boundary.
            let truncated: String = cell.chars().take(w).collect();
            out.push(' ');
            out.push_str(&truncated);
        } else {
            out.push(' ');
            out.push_str(cell);
            for _ in 0..w.saturating_sub(cell_chars) {
                out.push(' ');
            }
        }
        out.push_str(" |");
    }
    out.push('\n');
}

fn write_row_owned(out: &mut String, row: &[String; 9], widths: &[usize; 9]) {
    let refs: [&str; 9] = [
        row[0].as_str(),
        row[1].as_str(),
        row[2].as_str(),
        row[3].as_str(),
        row[4].as_str(),
        row[5].as_str(),
        row[6].as_str(),
        row[7].as_str(),
        row[8].as_str(),
    ];
    write_row(out, &refs, widths);
}

fn write_separator(out: &mut String, widths: &[usize; 9]) {
    out.push('|');
    for w in widths {
        out.push(' ');
        for _ in 0..*w {
            out.push('-');
        }
        out.push_str(" |");
    }
    out.push('\n');
}

#[allow(dead_code)]
fn _ensure_used(_: &LeaderboardEntry) {}

#[cfg(test)]
mod tests {
    use super::super::runner::{Leaderboard, LeaderboardEntry, TournamentConfig};
    use super::super::types::ScoreReport;
    use super::*;
    use std::time::Duration;

    fn entry(
        rank: u32,
        name: &str,
        total: u32,
        wins: u32,
        matches_: u32,
        best: ScoreReport,
    ) -> LeaderboardEntry {
        LeaderboardEntry {
            solver: name.to_owned(),
            rank,
            total_score: total,
            matches: matches_,
            wins,
            win_rate: if matches_ == 0 { 0.0 } else { f64::from(wins) / f64::from(matches_) },
            best,
        }
    }

    fn board(entries: Vec<LeaderboardEntry>) -> Leaderboard {
        Leaderboard {
            config: TournamentConfig { sequential: true, title: "sample".into() },
            entries,
            reports: vec![],
            elapsed_ms: 1234,
        }
    }

    fn best(name: &str) -> ScoreReport {
        ScoreReport::new("m1", name, 5, 7, 30, 100, Duration::from_millis(100), false)
    }

    #[test]
    fn markdown_contains_header_and_rows() {
        let board = board(vec![
            entry(1, "alpha", 60, 2, 2, best("alpha")),
            entry(2, "beta", 40, 1, 2, best("beta")),
        ]);
        let md = render_markdown(&board);
        assert!(md.contains("## sample"));
        assert!(md.contains("| Rank | Solver |"));
        assert!(md.contains("alpha"));
        assert!(md.contains("---"), "separator dashes present");
    }

    #[test]
    fn json_round_trips() {
        let board = board(vec![entry(1, "alpha", 60, 2, 2, best("alpha"))]);
        let s = render_json(&board).unwrap();
        assert!(s.contains("\"alpha\""));
        let back: Leaderboard = serde_json::from_str(&s).unwrap();
        assert_eq!(back, board);
    }

    #[test]
    fn empty_leaderboard_renders_gracefully() {
        let board = board(vec![]);
        let md = render_markdown(&board);
        assert!(md.contains("No solvers reported"));
    }
}
