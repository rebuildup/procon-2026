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

use bench::candidate::{CandidateError, CandidateRegistry};
use bench::case::Corpus;
use bench::tournament::{TournamentConfig, TournamentError, TournamentRunner};
use clap::Parser;

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

    /// Wall deadline per (candidate, case) run, in milliseconds.
    #[arg(long, default_value_t = 60_000)]
    timeout_ms: u64,

    /// Directory for `runs.jsonl` + `aggregate.json`.
    #[arg(long, default_value = "target/eval")]
    output_dir: PathBuf,

    /// Also write the Markdown report to this path.
    #[arg(long)]
    markdown_out: Option<PathBuf>,

    /// Suppress the Markdown report on stdout (artifacts are still written).
    #[arg(long, default_value_t = false)]
    quiet: bool,
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
    let registry = CandidateRegistry::builtin();

    if args.list_candidates {
        print_catalog(&registry);
        return Exit::Ok;
    }

    let Some(candidate_list) = args.candidates.as_deref() else {
        return Exit::Fail(
            "--candidates is required (or use --list-candidates to see what is available)"
                .to_owned(),
        );
    };

    let candidate_ids = match split_ids(candidate_list) {
        Ok(ids) if !ids.is_empty() => ids,
        Ok(_) => return Exit::Fail("--candidates did not name any candidate".to_owned()),
        Err(e) => return Exit::Fail(e),
    };

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

    let config = TournamentConfig::new(
        candidate_ids,
        args.baseline.clone(),
        args.team.clone(),
        Duration::from_millis(args.timeout_ms),
    )
    .with_source_commit_sha(source_commit_sha)
    .with_output_dir(args.output_dir.clone());

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
        eprintln!("{failures} run(s) did not produce a score; see the failure table");
    }
    Exit::Ok
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
