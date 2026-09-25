//! `procon-eval` — run the canonical offline evaluation batch.
//!
//! This is the **performance** entry point: it ranks candidate solvers on the
//! official competitive score tuple against a fixed corpus. It is unrelated
//! to `procon tournament` (the CLI's real-server wire/protocol smoke
//! harness), which reports *protocol progress* rather than solver quality.
//!
//! ```text
//! # every bundled fixture, three loop variants against the wait baseline
//! procon-eval \
//!     --cases all \
//!     --candidates loop:iterations=32:initial=greedy,loop:iterations=128:initial=greedy,wait \
//!     --baseline wait \
//!     --output-dir target/eval
//!
//! # the candidate axis the registry supports today
//! procon-eval --list-candidates
//! ```
//!
//! Results land in `<output-dir>/runs.jsonl` (one durable record per
//! candidate × case) and `<output-dir>/aggregate.json` (ranking, win/draw/loss
//! vs baseline, timeout/invalid counts, score-tuple summaries).

use std::path::PathBuf;
use std::time::Duration;

use bench::CaseSpec;
use bench::candidate::{CandidateError, CandidateRegistry};
use bench::case::Corpus;
use bench::evaluate::RunRecord;
use bench::tournament::{TournamentConfig, TournamentError, TournamentRunner};
use clap::Parser;
use domain::PlayerId;

#[derive(Debug, Parser)]
#[command(
    name = "procon-eval",
    about = "Canonical offline solver evaluation (official score tuple, lexicographic ranking)",
    long_about = "Evaluate candidate solvers against a fixed case corpus through engine::MatchSim.\n\n\
        The score is the official 4-tier tuple — variety, daily_variety_sum, total_balls, \
        response_time (lower better) — compared lexicographically, never as a weighted sum.\n\n\
        This is NOT `procon tournament`: that command drives the real server and reports \
        protocol progress, not solver quality."
)]
struct Args {
    /// Show the registered candidate algorithms and their parameters, then exit.
    #[arg(long)]
    list_candidates: bool,

    /// Comma-separated case names (`small-8x8,medium-16x16`), or `all` for the
    /// whole bundled fixture corpus.
    #[arg(long, default_value = "all")]
    cases: String,

    /// Comma-separated candidate ids, e.g.
    /// `loop:iterations=32:initial=greedy,greedy,wait`.
    #[arg(long)]
    candidates: Option<String>,

    /// Candidate that win / draw / lose is measured against. Must appear in
    /// `--candidates`.
    #[arg(long)]
    baseline: Option<String>,

    /// Team id every candidate plays as.
    #[arg(long, default_value = "player-a")]
    team: String,

    /// Absolute harness deadline per run, in milliseconds: how long the parent
    /// waits before killing a worker. A safety net for a worker that has
    /// stopped making progress, **not** the official per-day answer window
    /// (`daySeconds`), which the evaluator judges separately per day.
    ///
    /// Omit it to derive the deadline from the case instead: the case's
    /// `sum(daySeconds)` plus `--harness-margin-ms`. Deriving is the default
    /// because a flat absolute value can kill a perfectly legal run —
    /// `daySeconds = [60,60,60]` allows 180 s of answering in total.
    #[arg(long = "harness-timeout-ms", alias = "timeout-ms")]
    harness_timeout_ms: Option<u64>,

    /// Slack added when deriving the deadline from the case, covering process
    /// startup and serialization. Ignored when `--harness-timeout-ms` is set.
    #[arg(long = "harness-margin-ms", default_value_t = 5_000)]
    harness_margin_ms: u64,

    /// Directory for `runs.jsonl` + `aggregate.json`.
    #[arg(long, default_value = "target/eval")]
    output_dir: PathBuf,

    /// Also write the Markdown report to this path.
    #[arg(long)]
    markdown_out: Option<PathBuf>,

    /// Suppress the Markdown report on stdout (artifacts are still written).
    #[arg(long, default_value_t = false)]
    quiet: bool,

    // ---- worker mode (internal) -----------------------------------------
    /// Evaluate exactly one (candidate, case) and print its `RunRecord` as
    /// JSON on stdout. This is the process [`bench::evaluate::Isolation`]
    /// spawns so an overrunning run can be killed. Not meant to be invoked
    /// by hand.
    #[arg(long)]
    worker: bool,

    /// Worker mode: the candidate id to build.
    #[arg(long, requires_all = ["worker", "case_json"], default_value = "")]
    candidate_id: String,

    /// Worker mode: the case serialised as JSON.
    #[arg(long, requires_all = ["worker", "candidate_id"], default_value = "")]
    case_json: String,

    /// Worker mode: commit SHA to stamp on the record.
    #[arg(long = "source-commit-sha", requires = "worker", default_value = "unknown")]
    source_commit_sha: String,
}

#[derive(Debug)]
enum Exit {
    Ok,
    Fail(String),
}

fn main() -> std::process::ExitCode {
    match run() {
        Exit::Ok => std::process::ExitCode::SUCCESS,
        Exit::Fail(msg) => {
            eprintln!("error: {msg}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Exit {
    let args = Args::parse();
    if args.worker {
        return run_worker(&args);
    }
    let registry = CandidateRegistry::builtin();

    if args.list_candidates {
        print_catalog(&registry);
        return Exit::Ok;
    }

    if args.candidates.is_none() {
        return Exit::Fail(
            "--candidates is required (or use --list-candidates to see what is available)"
                .to_owned(),
        );
    }

    // Ids themselves are parsed and validated by `build_config`, so the flag
    // → config path is the one the tests cover.
    let corpus = match load_corpus(&args.cases) {
        Ok(c) => c,
        Err(e) => return Exit::Fail(e),
    };

    // Stamp the commit so records are attributable. `just` sets the env var;
    // fall back to asking git so a bare `cargo run` still produces a real SHA.
    // (`std::env::set_var` is unsafe in edition 2024 and this crate denies
    // `unsafe`, so the SHA is threaded through the config instead.)
    let source_commit_sha = std::env::var("PROCON_SOURCE_COMMIT_SHA")
        .ok()
        .or_else(git_head)
        .unwrap_or_else(|| "unknown".to_owned());

    let config = match build_config(&args, source_commit_sha) {
        Ok(c) => c,
        Err(e) => return Exit::Fail(e),
    };

    let runner = TournamentRunner::new(config);
    let outcome = match runner.run(&registry, &corpus) {
        Ok(o) => o,
        Err(e) => return Exit::Fail(format_tournament_error(&e)),
    };

    let md = outcome.to_markdown();
    if !args.quiet {
        println!("{md}");
    }
    if let Some(path) = &args.markdown_out {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    return Exit::Fail(format!("could not create {}: {e}", dir.display()));
                }
            }
        }
        if let Err(e) = std::fs::write(path, &md) {
            return Exit::Fail(format!("could not write {}: {e}", path.display()));
        }
    }

    if let Some(paths) = &outcome.paths {
        eprintln!(
            "wrote {} records to {} and {}",
            outcome.records.len(),
            paths.runs.display(),
            paths.aggregate.display()
        );
    }

    // Non-zero exit when anything failed, so CI can gate on it — while the
    // records are still written and readable.
    let failures = outcome.records.iter().filter(|r| !r.is_scored()).count();
    if failures > 0 {
        // The records are written before we return, so CI gets both the
        // evidence and the failure signal.
        return Exit::Fail(format!(
            "{failures} of {} run(s) did not produce a rankable score; see the failure table",
            outcome.records.len()
        ));
    }
    Exit::Ok
}

/// Worker mode: evaluate one `(candidate, case)` and print its `RunRecord`.
///
/// Deliberately forced onto in-thread execution: this process already *is*
/// the isolation boundary, and a nested child would only add latency. If the
/// parent kills us mid-run, that is exactly the timeout behaviour it asked
/// for.
fn run_worker(args: &Args) -> Exit {
    // The worker ALWAYS writes exactly one `RunRecord` JSON line to stdout,
    // even when it cannot evaluate anything. A silent non-zero exit would be
    // indistinguishable from a crash, and the parent would have to guess at
    // the cause. The only way to produce no output is to be killed.
    let record = match worker_record(args) {
        Ok(r) => r,
        // The real case id is re-stamped by the parent from its own `CaseSpec`,
        // so the placeholder here only needs to be a valid value.
        Err(detail) => RunRecord::failed(
            args.candidate_id.clone(),
            "<unreadable>",
            args.source_commit_sha.clone(),
            detail,
        ),
    };

    match serde_json::to_string(&record) {
        Ok(json) => {
            println!("{json}");
            Exit::Ok
        }
        Err(e) => Exit::Fail(format!("could not encode the record: {e}")),
    }
}

/// Turn parsed CLI flags into a [`TournamentConfig`].
///
/// Kept apart from `run` so the flag → config mapping is directly
/// testable: a flag that parses but is dropped here would otherwise be
/// silently ignored (`--harness-margin-ms` was).
///
/// # Errors
/// A `--candidates` value that does not name a valid candidate id.
fn build_config(args: &Args, source_commit_sha: String) -> Result<TournamentConfig, String> {
    let candidate_ids = split_ids(args.candidates.as_deref().unwrap_or_default())?;
    Ok(TournamentConfig::new(
        candidate_ids,
        args.baseline.clone(),
        args.team.clone(),
        args.harness_timeout_ms,
    )
    .with_source_commit_sha(source_commit_sha)
    // Without this, `--harness-margin-ms` parsed but never reached the
    // runner, which then used its own default.
    .with_harness_margin_ms(args.harness_margin_ms)
    .with_output_dir(args.output_dir.clone()))
}

/// The deadline policy for this invocation.
///
/// The parent always sends the deadline it already resolved for *this case*,
/// so the worker inherits it verbatim rather than re-deriving it against
/// whatever case body happens to be in the argument.
fn worker_deadline(args: &Args) -> bench::evaluate::HarnessDeadline {
    use bench::evaluate::HarnessDeadline;
    match args.harness_timeout_ms {
        Some(ms) => HarnessDeadline::Fixed(Duration::from_millis(ms)),
        None => {
            HarnessDeadline::CaseWindow { margin: Duration::from_millis(args.harness_margin_ms) }
        }
    }
}

/// Build and evaluate one run for worker mode.
fn worker_record(args: &Args) -> Result<RunRecord, String> {
    use bench::evaluate::{EvalConfig, Evaluator, Isolation};

    if args.candidate_id.is_empty() || args.case_json.is_empty() {
        return Err("--worker requires --candidate-id and --case-json".to_owned());
    }

    let registry = CandidateRegistry::builtin();
    let candidate = registry
        .build(&args.candidate_id)
        .map_err(|e| format!("bad candidate {:?}: {e}", args.candidate_id))?;
    let case: CaseSpec =
        serde_json::from_str(&args.case_json).map_err(|e| format!("bad --case-json: {e}"))?;
    // Same `--team` flag the parent passes. It was previously read from a
    // separate `--worker-team` that nothing sent, so any non-default team
    // silently fell back to `player-a` under process isolation.
    let team = PlayerId::new(args.team.as_str()).map_err(|e| format!("bad team: {e}"))?;

    // This process already is the isolation boundary; a nested child would
    // only add latency.
    let config = EvalConfig {
        source_commit_sha: args.source_commit_sha.clone(),
        team,
        harness_deadline: worker_deadline(args),
        isolation: Isolation::Thread,
    };
    Ok(Evaluator::new(config).evaluate_one(&candidate, &case))
}

fn print_catalog(registry: &CandidateRegistry) {
    println!("Registered candidate algorithms:\n");
    for entry in registry.catalog().algorithms {
        println!("  {}", entry.algorithm);
        println!("      {}", entry.describe);
        if entry.params.is_empty() {
            println!("      (no parameters)");
        }
        for p in entry.params {
            let req = if p.required { "required" } else { "optional" };
            println!("      :{0}=…  ({1}) {2}", p.key, req, p.doc);
        }
        for ex in &entry.examples {
            println!("      e.g. {ex}");
        }
        println!();
    }
    println!("Candidate ids round-trip: `loop:iterations=32:initial=greedy` parses and");
    println!("re-renders unchanged. Parameter order is preserved as written.");
}

fn split_ids(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match bench::candidate::CandidateSpec::parse(part) {
            Ok(_) => out.push(part.to_owned()),
            Err(e) => return Err(format!("bad candidate id {part:?}: {e}")),
        }
    }
    if out.is_empty() {
        return Err("--candidates did not name any candidate".to_owned());
    }
    Ok(out)
}

fn load_corpus(spec: &str) -> Result<Corpus, String> {
    if spec.trim().eq_ignore_ascii_case("all") {
        return Corpus::bundled().map_err(|e| format!("could not load the bundled corpus: {e}"));
    }
    let names: Vec<&str> = spec.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if names.is_empty() {
        return Err("--cases did not name any case".to_owned());
    }
    Corpus::bundled_subset(&names).map_err(|e| format!("could not load cases {names:?}: {e}"))
}

fn git_head() -> Option<String> {
    let out = std::process::Command::new("git").args(["rev-parse", "HEAD"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}

fn format_tournament_error(e: &TournamentError) -> String {
    match e {
        TournamentError::Candidate(ce) => match ce {
            CandidateError::UnknownAlgorithm(name) => format!(
                "unknown algorithm {name:?} — run `procon-eval --list-candidates` to see what is registered"
            ),
            other => format!("bad candidate: {other}"),
        },
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(extra: &[&str]) -> Args {
        let mut argv = vec!["procon-eval", "--candidates", "wait", "--cases", "small-8x8"];
        argv.extend_from_slice(extra);
        Args::parse_from(argv)
    }

    /// `--harness-margin-ms` used to parse and then be dropped: the runner
    /// kept its own default. This asserts the flag actually reaches the
    /// config, which is the only path from CLI to evaluation.
    #[test]
    fn cli_flags_reach_the_tournament_config() {
        let args = parse(&[
            "--harness-margin-ms",
            "1234",
            "--harness-timeout-ms",
            "4321",
            "--team",
            "cli-team",
        ]);
        let config = build_config(&args, "test-sha".to_owned()).expect("flags are valid");

        assert_eq!(config.harness_margin_ms, 1234, "--harness-margin-ms was dropped");
        assert_eq!(config.harness_timeout_ms, Some(4321), "--harness-timeout-ms was dropped");
        assert_eq!(config.team, "cli-team", "--team was dropped");
        assert_eq!(config.source_commit_sha, "test-sha");
    }

    /// The margin default must stay a default, not become an override.
    #[test]
    fn an_unspecified_margin_keeps_the_default() {
        let config = build_config(&parse(&[]), "s".to_owned()).expect("flags are valid");
        assert_eq!(config.harness_margin_ms, 5_000);
        assert_eq!(
            config.harness_timeout_ms, None,
            "omitting --harness-timeout-ms must mean 'derive from the case'"
        );
    }

    /// The derived deadline must be `sum(daySeconds) + margin`, so the margin
    /// is observable end to end rather than only at the flag boundary.
    #[test]
    fn the_configured_margin_is_used_when_resolving_the_deadline() {
        use std::time::Duration;

        let config = build_config(&parse(&["--harness-margin-ms", "1234"]), "s".to_owned())
            .expect("flags are valid");
        let runner = bench::tournament::TournamentRunner::new(config);
        let case = bench::CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");

        // `small-8x8` declares daySeconds = [60].
        let deadline = runner.resolved_harness_deadline(&case);
        assert_eq!(deadline, Duration::from_secs(60) + Duration::from_millis(1234));
    }
}
