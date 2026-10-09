//! Merge resumable `procon-eval` batches into one ranked evidence set.
//!
//! `procon-eval` aggregates only the batch it just ran. A long campaign is
//! split into many small batches (see `scripts/longrun_eval.py`) precisely so
//! an interrupted process loses one batch instead of the whole run — which
//! means the campaign-level ranking has to be rebuilt from the parts.
//!
//! This example is deliberately *not* a second scorer:
//!
//! * the merged ranking is `bench::AggregateReport::build`, the same struct
//!   `procon-eval` writes, so it refuses mixed commits exactly as upstream;
//! * every pairwise / per-condition win-draw-loss call goes through
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::type_complexity
)]
//!   `domain::compare_scores`, the official lexicographic comparison
//!   (variety → daily variety sum → total balls → response time).
//!
//! `--axes 3` drops the response-time tier before comparing. That tier is
//! wall-clock measured and `procon-eval --jobs > 1` warns that CPU contention
//! makes it unusable for ranking, so axes 1..3 are the primary view for a
//! parallel campaign and the full four-tier view is reported alongside it.
//!
//! Records are keyed on `(candidate, case, agent_kinds)` because the kind
//! pattern (`--kinds`) is itself an evaluation axis and the same board
//! legitimately re-runs under different compositions.
//!
//! ```bash
//! cargo run --release -p bench --example eval_merge -- \
//!     --input target/longrun/A --input target/longrun/B \
//!     --baseline 'coverage:budget=official:park=none' \
//!     --out target/longrun/merged
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bench::aggregate::AggregateReport;
use bench::evaluate::RunRecord;
use bench::report::{read_runs, write_aggregate, write_runs};
use bench::tournament::render::render_markdown;
use bench::{OfficialScore, compare_scores};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "eval_merge",
    about = "Merge procon-eval batches and rank them with the official score tuple"
)]
struct Args {
    /// A `runs.jsonl` file, or a directory searched recursively for one.
    /// Repeatable; batches are concatenated in the order given.
    #[arg(long)]
    input: Vec<PathBuf>,

    /// Candidate that win / draw / lose is measured against. Must be present.
    #[arg(long)]
    baseline: String,

    /// Directory for merged artifacts.
    #[arg(long, default_value = "target/longrun/merged")]
    out: PathBuf,

    /// `4` (default) compares the full official tuple; `3` compares only
    /// variety → daily variety sum → total balls, ignoring response time.
    #[arg(long, default_value_t = 4)]
    axes: u8,
}

/// Board-level comparison unit: one case under one agent-kind composition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
struct Board {
    case_id: String,
    kinds: String,
}

impl std::fmt::Display for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.kinds == "-" {
            write!(f, "{}", self.case_id)
        } else {
            write!(f, "{} [{}]", self.case_id, self.kinds)
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct PairCell {
    candidate: String,
    opponent: String,
    wins: u32,
    draws: u32,
    losses: u32,
    not_compared: u32,
    /// Boards where the opponent strictly beat this candidate on the compared
    /// axes — the regression list a developer needs.
    losing_boards: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
struct ConditionKey {
    source: String,
    archetype: String,
    size: String,
    days: u32,
    fuel: String,
    kinds: String,
}

#[derive(Debug, serde::Serialize)]
struct ConditionRow {
    condition: ConditionKey,
    candidate: String,
    cases: u32,
    wins: u32,
    draws: u32,
    losses: u32,
    unscored: u32,
    mean_variety: f64,
    mean_daily_variety_sum: f64,
    mean_total_balls: f64,
}

#[derive(Debug, serde::Serialize)]
struct Axis4Row {
    candidate: String,
    runs: u32,
    mean_response_ms: f64,
    p95_response_ms: f64,
    max_response_ms: f64,
    mean_elapsed_ms: f64,
}

#[derive(Debug, serde::Serialize)]
struct Provenance {
    inputs: Vec<String>,
    batch_files: u32,
    records_read: u32,
    records_unique: u32,
    duplicate_keys: u32,
    /// Duplicate `(candidate, case, kinds)` rows that disagreed on tiers 1..3.
    /// Non-empty means the same configuration scored differently across
    /// batches — a determinism or provenance bug, not a tie.
    tier_conflicts: Vec<String>,
    /// Duplicate rows that agreed on tiers 1..3 but not on response time.
    /// Expected whenever batches ran with different `--jobs`: axis 4 is a
    /// measurement, not a guarantee.
    tier4_variances: u32,
    source_commit_shas: Vec<String>,
    axes: u8,
    baseline: String,
}

// Offline aggregation tool: counts fit u32 comfortably; main drives the
// whole merge/report flow (kept in one place by design).
#[allow(clippy::too_many_lines)]
fn main() -> ExitCode {
    let args = Args::parse();
    if !(3..=4).contains(&args.axes) {
        eprintln!("error: --axes must be 3 or 4");
        return ExitCode::FAILURE;
    }
    let files = match discover(&args.input) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if files.is_empty() {
        eprintln!("error: no runs.jsonl found under --input");
        return ExitCode::FAILURE;
    }

    let mut all: Vec<RunRecord> = Vec::new();
    for f in &files {
        match read_runs(f) {
            Ok(mut r) => all.append(&mut r),
            Err(e) => {
                eprintln!("error: reading {}: {e}", f.display());
                return ExitCode::FAILURE;
            }
        }
    }
    let records_read = all.len() as u32;

    // Deduplicate on (candidate, case, agent_kinds). Batches repeat the anchor
    // fixture on purpose, so this key is hit constantly; identical scores are
    // fine and disagreement is reported rather than silently resolved.
    let mut seen: BTreeMap<(String, String, String), RunRecord> = BTreeMap::new();
    let mut duplicate_keys = 0u32;
    let mut tier_conflicts: Vec<String> = Vec::new();
    let mut tier4_variances = 0u32;
    for r in all {
        let key = (r.candidate_id.clone(), r.case_id.clone(), kinds_key(&r));
        match seen.get(&key) {
            None => {
                seen.insert(key, r);
            }
            Some(prev) => {
                duplicate_keys += 1;
                let same_tiers = prev.variety == r.variety
                    && prev.daily_variety_sum == r.daily_variety_sum
                    && prev.total_balls == r.total_balls
                    && prev.is_scored() == r.is_scored();
                if !same_tiers {
                    tier_conflicts.push(format!(
                        "{} × {} [{}]: ({}, {}, {}) vs ({}, {}, {})",
                        key.0,
                        key.1,
                        key.2,
                        prev.variety,
                        prev.daily_variety_sum,
                        prev.total_balls,
                        r.variety,
                        r.daily_variety_sum,
                        r.total_balls
                    ));
                } else if prev.response_time_us != r.response_time_us {
                    tier4_variances += 1;
                }
            }
        }
    }

    let baseline_id = args.baseline.clone();
    let mut merged: Vec<RunRecord> = seen.into_values().collect();
    merged.sort_by(|a, b| {
        a.case_id
            .cmp(&b.case_id)
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });

    if !merged.iter().any(|r| r.candidate_id == baseline_id) {
        eprintln!("error: baseline {baseline_id:?} is not in the merged records");
        return ExitCode::FAILURE;
    }

    let report = match AggregateReport::build(merged.clone(), Some(&baseline_id)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let out = &args.out;
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("error: create {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = write_runs(&out.join("runs.jsonl"), &merged) {
        eprintln!("error: write runs: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = write_aggregate(&out.join("aggregate.json"), &report) {
        eprintln!("error: write aggregate: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(out.join("report.md"), render_markdown(&report)) {
        eprintln!("error: write report: {e}");
        return ExitCode::FAILURE;
    }

    write_json(out, "pairwise.json", &build_pairwise(&merged, &baseline_id, args.axes));
    write_json(out, "by_condition.json", &build_conditions(&merged, &baseline_id, args.axes));
    write_json(out, "axis4.json", &build_axis4(&merged));

    let provenance = Provenance {
        inputs: files.iter().map(|p| p.display().to_string()).collect(),
        batch_files: files.len() as u32,
        records_read,
        records_unique: merged.len() as u32,
        duplicate_keys,
        tier_conflicts: tier_conflicts.clone(),
        tier4_variances,
        source_commit_shas: report.source_commit_shas.clone(),
        axes: args.axes,
        baseline: baseline_id,
    };
    write_json(out, "provenance.json", &provenance);

    println!(
        "merged {} records from {} batch files -> {} ({} unique, {} duplicate keys)",
        records_read,
        files.len(),
        out.display(),
        merged.len(),
        duplicate_keys
    );
    println!(
        "tiers 1..3 conflicts across duplicates: {}, tier-4 variances: {}",
        tier_conflicts.len(),
        tier4_variances
    );
    for s in &report.per_candidate {
        println!(
            "  {:>4}/{:<4}/{}  runs={} scored={} invalid={} timeout={} panic={}  {}",
            s.wins, s.draws, s.losses, s.runs, s.scored, s.invalids, s.timeouts, s.panics,
            s.candidate_id
        );
    }
    println!("(summary above uses the full official tuple including response time;");
    println!(" pairwise.json / by_condition.json use the {}-axis view)", args.axes);

    if !tier_conflicts.is_empty() {
        eprintln!("tier-1..3 conflicts (first 10):");
        for c in tier_conflicts.iter().take(10) {
            eprintln!("  {c}");
        }
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn write_json<T: serde::Serialize>(out: &Path, name: &str, value: &T) {
    let path = out.join(name);
    match serde_json::to_string_pretty(value) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, format!("{s}\n")) {
                eprintln!("error: write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("error: encode {name}: {e}"),
    }
}

fn discover(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for input in inputs {
        if input.is_file() {
            files.push(input.clone());
            continue;
        }
        if !input.is_dir() {
            return Err(format!("{} is neither a file nor a directory", input.display()));
        }
        let mut stack = vec![input.clone()];
        while let Some(dir) = stack.pop() {
            let entries =
                std::fs::read_dir(&dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().and_then(|n| n.to_str()) == Some("runs.jsonl") {
                    files.push(p);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn kinds_key(r: &RunRecord) -> String {
    match &r.agent_kinds {
        Some(kinds) if !kinds.is_empty() => {
            kinds.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>().join(",")
        }
        _ => "-".to_owned(),
    }
}

/// Project a record onto the first `axes` official tiers.
///
/// Axis 4 is inverted (lower is better), so the tuple fed to
/// `compare_scores` is built with the axis-4 value zeroed when only three
/// axes are requested — `compare_scores` then never sees a response-time
/// difference.
fn score_with_axes(r: &RunRecord, axes: u8) -> OfficialScore {
    if axes >= 4 {
        return r.to_score();
    }
    OfficialScore::new(r.variety, r.daily_variety_sum, r.total_balls, 0)
}

fn board_of(r: &RunRecord) -> Board {
    Board { case_id: r.case_id.clone(), kinds: kinds_key(r) }
}

fn build_pairwise(runs: &[RunRecord], baseline: &str, axes: u8) -> Vec<PairCell> {
    let mut candidates: Vec<String> = runs.iter().map(|r| r.candidate_id.clone()).collect();
    candidates.sort();
    candidates.dedup();

    let mut boards: BTreeMap<Board, BTreeMap<String, &RunRecord>> = BTreeMap::new();
    for r in runs {
        boards.entry(board_of(r)).or_default().insert(r.candidate_id.clone(), r);
    }

    let mut out = Vec::new();
    for a in &candidates {
        for b in &candidates {
            if a == b {
                continue;
            }
            let (mut w, mut d, mut l, mut nc) = (0u32, 0u32, 0u32, 0u32);
            let mut losing_boards = Vec::new();
            for (board, per_candidate) in &boards {
                let (Some(ra), Some(rb)) = (per_candidate.get(a), per_candidate.get(b)) else {
                    continue;
                };
                if !ra.is_scored() || !rb.is_scored() {
                    nc += 1;
                    continue;
                }
                match compare_scores(&score_with_axes(ra, axes), &score_with_axes(rb, axes)) {
                    std::cmp::Ordering::Greater => w += 1,
                    std::cmp::Ordering::Equal => d += 1,
                    std::cmp::Ordering::Less => {
                        l += 1;
                        losing_boards.push(board.to_string());
                    }
                }
            }
            out.push(PairCell {
                candidate: a.clone(),
                opponent: b.clone(),
                wins: w,
                draws: d,
                losses: l,
                not_compared: nc,
                losing_boards,
            });
        }
    }
    // Baseline-relative cells first so a reader can scan them.
    out.sort_by(|x, y| {
        let rank = |c: &PairCell| (c.opponent == baseline, c.candidate == baseline);
        rank(x).cmp(&rank(y)).then_with(|| x.candidate.cmp(&y.candidate))
    });
    out
}

/// Condition of a record, read from `generation_parameters` when present.
fn condition_of(r: &RunRecord) -> ConditionKey {
    let empty = serde_json::Value::Null;
    let gp = r.generation_parameters.as_ref().unwrap_or(&empty);
    let s = |k: &str| gp.get(k).and_then(|v| v.as_str()).unwrap_or("-").to_owned();
    let days = gp.get("days").and_then(serde_json::Value::as_u64).unwrap_or(0) as u32;
    ConditionKey {
        source: if r.case_id.starts_with("generated:") {
            "generated".to_owned()
        } else {
            "fixture".to_owned()
        },
        archetype: s("archetype"),
        size: s("size"),
        days,
        fuel: s("fuel"),
        kinds: kinds_key(r),
    }
}

/// Per (condition, candidate) win/draw/loss against the baseline plus the
/// mean official tiers. A condition is one generated archetype × size ×
/// days × fuel combination, or the fixture bucket; `generation_parameters`
/// carries those values explicitly on every generated record.
fn build_conditions(runs: &[RunRecord], baseline: &str, axes: u8) -> Vec<ConditionRow> {
    let mut by_board: BTreeMap<Board, BTreeMap<String, &RunRecord>> = BTreeMap::new();
    for r in runs {
        by_board.entry(board_of(r)).or_default().insert(r.candidate_id.clone(), r);
    }

    // (condition, candidate) -> (cases, wins, draws, losses, unscored, ΣV, ΣD, ΣB)
    let mut acc: BTreeMap<(ConditionKey, String), (u32, u32, u32, u32, u32, f64, f64, f64)> =
        BTreeMap::new();

    for per_candidate in by_board.values() {
        let Some(base) = per_candidate.get(baseline) else { continue };
        let ck = condition_of(base);
        for (cid, rec) in per_candidate {
            if *cid == baseline {
                continue;
            }
            let slot = acc
                .entry((ck.clone(), cid.to_owned()))
                .or_insert((0, 0, 0, 0, 0, 0.0, 0.0, 0.0));
            slot.0 += 1;
            if !rec.is_scored() || !base.is_scored() {
                slot.4 += 1;
                continue;
            }
            match compare_scores(&score_with_axes(rec, axes), &score_with_axes(base, axes)) {
                std::cmp::Ordering::Greater => slot.1 += 1,
                std::cmp::Ordering::Equal => slot.2 += 1,
                std::cmp::Ordering::Less => slot.3 += 1,
            }
            slot.5 += f64::from(rec.variety);
            slot.6 += f64::from(rec.daily_variety_sum);
            slot.7 += f64::from(rec.total_balls);
        }
    }

    let mut rows: Vec<ConditionRow> = acc
        .into_iter()
        .map(|((ck, cid), (n, w, d, l, u, sv, sd, sb))| {
            let scored = f64::from((n - u).max(1));
            ConditionRow {
                condition: ck,
                candidate: cid,
                cases: n,
                wins: w,
                draws: d,
                losses: l,
                unscored: u,
                mean_variety: sv / scored,
                mean_daily_variety_sum: sd / scored,
                mean_total_balls: sb / scored,
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        a.candidate.cmp(&b.candidate).then_with(|| {
            let key = |c: &ConditionKey| {
                (c.source.clone(), c.archetype.clone(), c.size.clone(), c.days, c.fuel.clone())
            };
            key(&a.condition).cmp(&key(&b.condition))
        })
    });
    rows
}

fn build_axis4(runs: &[RunRecord]) -> Vec<Axis4Row> {
    let mut grouped: BTreeMap<&str, Vec<&RunRecord>> = BTreeMap::new();
    for r in runs {
        grouped.entry(r.candidate_id.as_str()).or_default().push(r);
    }
    let mut out = Vec::new();
    for (cid, rs) in grouped {
        let mut response: Vec<f64> =
            rs.iter().map(|r| r.response_time_us as f64 / 1000.0).collect();
        response.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = response.len().max(1);
        let p95 = response[((n as f64) * 0.95).floor() as usize % n];
        out.push(Axis4Row {
            candidate: cid.to_owned(),
            runs: rs.len() as u32,
            mean_response_ms: response.iter().sum::<f64>() / n as f64,
            p95_response_ms: p95,
            max_response_ms: *response.last().unwrap_or(&0.0),
            mean_elapsed_ms: rs.iter().map(|r| r.elapsed_us as f64 / 1000.0).sum::<f64>()
                / rs.len().max(1) as f64,
        });
    }
    out
}
