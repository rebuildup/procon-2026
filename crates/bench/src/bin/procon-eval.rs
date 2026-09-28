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
//! # fixtures + seed-fixed generated cases in one batch (Issue #185).
//! # `--cases all` still means "the bundled fixtures only"; generated cases
//! # are opt-in, and every candidate in the batch sees this same case set.
//! procon-eval \
//!     --cases all \
//!     --generated 'open:seed=1,road-heavy:seed=7' \
//!     --candidates 'loop:iterations=32:initial=greedy,wait' \
//!     --baseline wait \
//!     --output-dir target/eval
//!
//! # the fuel-stress axis (Issue #204): `fuel=tight` pins fuelLimits to the
//! # Q14 floor, so fuel-aware and fuel-blind candidates finally differ
//! procon-eval \
//!     --cases all \
//!     --generated 'road-heavy:seed=1:fuel=tight,open:seed=1:fuel=tight' \
//!     --candidates 'loop:iterations=32:initial=greedy:walk=astar,wait' \
//!     --baseline wait \
//!     --output-dir target/eval-tight
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
    ///
    /// `all` (and every other value) selects **fixtures only** — generated
    /// cases are never implied, so existing evidence keeps its meaning. Add
    /// them with `--generated`.
    #[arg(long, default_value = "all")]
    cases: String,

    /// Opt-in generated cases, comma-separated with explicit seeds, e.g.
    /// `open:seed=1,road-heavy:seed=7:size=small:days=3:fuel=tight`.
    ///
    /// Grammar: `<archetype>[:seed=<u64>][:size=small|medium|large]
    /// [:days=1..3][:fuel=comfortable|tight]`, where `seed=` is required —
    /// explicit seeds are what let sibling candidates be compared on one
    /// identical case axis, and `fuel=tight` is the fuel-stress axis (Issue
    /// #204; default `comfortable` keeps the v1 no-fuel-short invariant).
    /// Appended to the `--cases` selection; an empty value selects nothing.
    #[arg(long, default_value = "")]
    generated: String,

    /// Comma-separated paths to `CaseSpec` JSON files to append to the
    /// corpus — the offline re-run half of the Issue #179 import hook
    /// (`procon arena import-case` output). Each file is parsed and
    /// validated **before** any candidate runs, so a malformed import
    /// fails fast instead of surfacing as a poisoned record inside a
    /// worker. Never implied: an empty value changes nothing.
    #[arg(long, default_value = "")]
    case_files: String,

    /// Agent-kind composition pattern (Issue #235): a comma-separated cycle
    /// over agent slots, `P` = patrol and `S` = supply, e.g. `P,S,P,S,P`.
    ///
    /// Slot `i` gets `pattern[i % len]`, so one pattern serves the whole
    /// corpus even though its teams field 3, 4, 5, 6 and 8 agents. Omitted
    /// (or empty) means `P,S` — the historical even-patrol / odd-supply
    /// alternation, byte-for-byte, so pre-#235 evidence keeps its meaning.
    /// The composition each run actually fielded is recorded in
    /// `RunRecord.agent_kinds`.
    ///
    /// Every ratio is legal (the official `POST /agent` schema constrains
    /// only the element count); only a non-kind token, an empty pattern or a
    /// pattern longer than the official 8-agent maximum are rejected.
    #[arg(long, default_value = "")]
    kinds: String,

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

    // A malformed `--kinds` is a pure flag error: reject it before anything
    // is loaded, so the user never waits on corpus construction to be told
    // their pattern has a typo in it.
    if let Err(e) = bench::kinds::parse_kinds_flag(&args.kinds) {
        return Exit::Fail(e);
    }

    // Ids themselves are parsed and validated by `build_config`, so the flag
    // → config path is the one the tests cover. Corpus construction (fixtures
    // + opt-in generated cases) fails fast here, before any candidate runs —
    // a generation failure can never become a run record.
    let corpus = match load_corpus(&args.cases, &args.generated, &args.case_files) {
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
/// Both `--candidates` and `--baseline` are resolved through the builtin
/// registry, so the config holds **canonical** ids (parameters in schema
/// order). The baseline is canonicalised the same way as the batch, because
/// membership checking and `AggregateReport::build` compare ids by string.
///
/// # Errors
/// A `--candidates` / `--baseline` value that does not name a valid candidate
/// id, or a batch whose entries name the same configuration twice.
fn build_config(args: &Args, source_commit_sha: String) -> Result<TournamentConfig, String> {
    // The CLI resolves against exactly one registry, the builtin one — the
    // same registry `run` later builds the batch with.
    let registry = CandidateRegistry::builtin();
    let candidate_ids = split_ids(args.candidates.as_deref().unwrap_or_default(), &registry)?;
    let baseline = match args.baseline.as_deref() {
        Some(baseline) => Some(
            registry
                .build(baseline)
                .map_err(|e| format_candidate_error(&format!("--baseline {baseline:?}"), &e))?
                .id(),
        ),
        None => None,
    };
    // `run` already rejected a malformed value; parsing again here keeps the
    // flag → config path self-contained so the test below proves the pattern
    // actually reaches the runner rather than being dropped in between.
    let kinds = bench::kinds::parse_kinds_flag(&args.kinds)?;
    Ok(TournamentConfig::new(candidate_ids, baseline, args.team.clone(), args.harness_timeout_ms)
        .with_source_commit_sha(source_commit_sha)
        // Without this, `--harness-margin-ms` parsed but never reached the
        // runner, which then used its own default.
        .with_harness_margin_ms(args.harness_margin_ms)
        .with_kinds(kinds)
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
    // The parent always sends `--kinds` (empty = default pattern), so the
    // composition the child fields is the one the batch declared. Parsing
    // here fails the record rather than silently falling back to the default.
    let kinds = bench::kinds::parse_kinds_flag(&args.kinds)?;

    let config = EvalConfig {
        source_commit_sha: args.source_commit_sha.clone(),
        team,
        harness_deadline: worker_deadline(args),
        isolation: Isolation::Thread,
        kinds,
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
    println!("Id syntax: `loop:iterations=32:initial=greedy` parses and re-renders");
    println!("unchanged. Evaluation identity is canonical: at resolve time the");
    println!("parameters are reordered into this schema order, so");
    println!("`loop:initial=greedy:iterations=32` is the same candidate — listing");
    println!("both is a duplicate error, not two runs of one configuration.");
}

/// Split `--candidates` into **canonical** ids, rejecting semantic duplicates.
///
/// Each entry is resolved through `registry`, which reorders parameters into
/// schema order, so `loop:initial=greedy:iterations=32` and
/// `loop:iterations=32:initial=greedy` arrive as one id and the second entry
/// fails the duplicate check instead of being evaluated twice.
///
/// # Errors
/// A bad id (with [`format_candidate_error`]'s wording), a duplicate
/// configuration, or an empty list.
fn split_ids(text: &str, registry: &CandidateRegistry) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for part in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let candidate = registry
            .build(part)
            .map_err(|e| format_candidate_error(&format!("candidate id {part:?}"), &e))?;
        let canonical = candidate.id();
        if let Some(first) = seen.insert(canonical.clone(), out.len()) {
            return Err(format!(
                "duplicate candidate {canonical:?}: entry #{first} names the same algorithm \
                 and parameters (parameter order is ignored)"
            ));
        }
        out.push(canonical);
    }
    if out.is_empty() {
        return Err("--candidates did not name any candidate".to_owned());
    }
    Ok(out)
}

/// Render a [`CandidateError`] for the CLI, keeping the `--list-candidates`
/// hint for the one error a user can act on directly.
fn format_candidate_error(subject: &str, e: &CandidateError) -> String {
    match e {
        CandidateError::UnknownAlgorithm(name) => format!(
            "unknown algorithm {name:?} — run `procon-eval --list-candidates` to see what is registered"
        ),
        other => format!("bad {subject}: {other}"),
    }
}

/// Build the batch corpus: the `--cases` fixture selection, then any
/// `--case-files` imports, then any opt-in `--generated` cases appended in
/// written order.
///
/// `--cases` is untouched by both appenders (`all` still means the bundled
/// fixtures only), so imported or generated cases can never silently
/// change what an existing invocation evaluates. Sibling candidates share
/// this one corpus object, which is what puts them on a single case axis.
///
/// # Errors
/// Fixture load errors, unreadable / non-`CaseSpec` / non-evaluable
/// `--case-files` entries (Issue #179 import hook — fail closed here,
/// before any candidate runs), or a [`bench::GenerationError`] whose
/// message starts with `case generation failure` — distinct from any
/// candidate failure.
fn load_corpus(cases: &str, generated: &str, case_files: &str) -> Result<Corpus, String> {
    let mut fixtures = if cases.trim().eq_ignore_ascii_case("all") {
        Corpus::bundled().map_err(|e| format!("could not load the bundled corpus: {e}"))?
    } else {
        let names: Vec<&str> = cases.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if names.is_empty() {
            return Err("--cases did not name any case".to_owned());
        }
        Corpus::bundled_subset(&names)
            .map_err(|e| format!("could not load cases {names:?}: {e}"))?
    };

    let mut has_case_files = false;
    for path in case_files.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        has_case_files = true;
        let case = load_case_file(path)?;
        fixtures.cases.push(case);
    }

    let params = bench::generator::parse_selection(generated).map_err(|e| e.to_string())?;
    if params.is_empty() {
        if !has_case_files {
            return Ok(fixtures);
        }
        // Re-validate the corpus so a duplicate / malformed import is
        // rejected here rather than inside a worker.
        return Corpus::new(fixtures.cases).map_err(|e| e.to_string());
    }
    let generated = bench::generator::build_corpus(&params).map_err(|e| e.to_string())?;
    fixtures.cases.extend(generated.cases);
    Corpus::new(fixtures.cases).map_err(|e| e.to_string())
}

/// Read one `--case-files` entry as a validated [`CaseSpec`].
///
/// Both evaluator-side validators run up front (thresholds and the
/// wire→engine board conversion), so an import that could not produce a
/// real score is rejected before the batch starts.
fn load_case_file(path: &str) -> Result<CaseSpec, String> {
    let raw =
        std::fs::read(path).map_err(|e| format!("could not read --case-files {path}: {e}"))?;
    let text =
        String::from_utf8(raw).map_err(|e| format!("--case-files {path} is not UTF-8: {e}"))?;
    let case: CaseSpec = serde_json::from_str(&text)
        .map_err(|e| format!("--case-files {path} is not a CaseSpec JSON: {e}"))?;
    case.sim_config().map_err(|e| format!("--case-files {path} is not evaluable: {e}"))?;
    let team = domain::PlayerId::new("case-files-check")
        .map_err(|e| format!("--case-files {path}: team id: {e}"))?;
    case.to_engine_setting(&team)
        .map_err(|e| format!("--case-files {path} is not evaluable: {e}"))?;
    Ok(case)
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
        TournamentError::Candidate(ce) => format_candidate_error("candidate", ce),
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

    /// An id written in a shuffled parameter order reaches the config — and
    /// therefore `RunRecord.candidate_id` — in canonical schema order.
    #[test]
    fn split_ids_yields_canonical_ids() {
        let registry = CandidateRegistry::builtin();
        let ids = split_ids("loop:initial=greedy:iterations=32,wait", &registry)
            .expect("a shuffled id is still a valid id");
        assert_eq!(ids, vec!["loop:iterations=32:initial=greedy", "wait"]);
    }

    /// Two spellings of one configuration must fail loudly rather than run
    /// the same setting twice in one batch.
    #[test]
    fn split_ids_rejects_a_shuffled_semantic_duplicate() {
        let registry = CandidateRegistry::builtin();
        let err = split_ids(
            "loop:iterations=32:initial=greedy,loop:initial=greedy:iterations=32",
            &registry,
        )
        .expect_err("one configuration cannot be a batch of two");
        assert!(err.contains("duplicate candidate"), "{err}");
        assert!(
            err.contains("loop:iterations=32:initial=greedy"),
            "the error must name the canonical id: {err}"
        );
    }

    /// The whole flag → config path, including `--baseline`, which is
    /// canonicalised so its membership check and the aggregate's W/D/L
    /// comparison match the (canonical) records.
    #[test]
    fn shuffled_candidates_and_baseline_reach_the_config_canonically() {
        let args = Args::parse_from([
            "procon-eval",
            "--cases",
            "small-8x8",
            "--candidates",
            "loop:initial=greedy:iterations=32,wait",
            "--baseline",
            "loop:iterations=32:initial=greedy",
        ]);
        let config = build_config(&args, "test-sha".to_owned()).expect("flags are valid");
        assert_eq!(config.candidate_ids, vec!["loop:iterations=32:initial=greedy", "wait"]);
        assert_eq!(
            config.baseline_candidate_id.as_deref(),
            Some("loop:iterations=32:initial=greedy")
        );
    }

    /// The same duplicate check, but observed through `build_config` — the
    /// only path from CLI flags to an evaluation.
    #[test]
    fn build_config_fails_on_a_shuffled_semantic_duplicate() {
        let args = Args::parse_from([
            "procon-eval",
            "--cases",
            "small-8x8",
            "--candidates",
            "loop:iterations=32:initial=greedy,loop:initial=greedy:iterations=32",
        ]);
        let err = build_config(&args, "test-sha".to_owned()).expect_err("duplicate configuration");
        assert!(err.contains("duplicate candidate"), "{err}");
    }

    /// Issue #235 AC: `--kinds` parses and reaches the batch config, an
    /// omitted flag stays `None` (= the historical alternation), and a
    /// malformed pattern is refused instead of silently falling back.
    #[test]
    fn the_kinds_flag_reaches_the_tournament_config() {
        use engine::model::AgentKind;

        let args = parse(&["--kinds", "P,P,P,P,S"]);
        let config = build_config(&args, "test-sha".to_owned()).expect("a valid pattern parses");
        assert_eq!(
            config.kinds,
            Some(vec![
                AgentKind::Patrol,
                AgentKind::Patrol,
                AgentKind::Patrol,
                AgentKind::Patrol,
                AgentKind::Supply
            ]),
            "--kinds was parsed but never reached the config"
        );

        let omitted = build_config(&parse(&[]), "s".to_owned()).expect("flags are valid");
        assert_eq!(omitted.kinds, None, "omitted --kinds must mean 'today's default'");

        for bad in ["3p2s", "X", "P,S,"] {
            let err = build_config(&parse(&["--kinds", bad]), "s".to_owned())
                .expect_err("a malformed pattern must be rejected");
            assert!(err.contains("--kinds"), "{bad} → {err}");
        }
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
    fn a_margin_flag_selects_a_case_window_with_that_margin() {
        use bench::evaluate::HarnessDeadline;
        use std::time::Duration;

        let config = build_config(&parse(&["--harness-margin-ms", "1234"]), "s".to_owned())
            .expect("flags are valid");
        let runner = bench::tournament::TournamentRunner::new(config);

        // The policy itself: the flag must choose the variant, not just the
        // number, or an explicit timeout would keep silently winning.
        assert_eq!(
            runner.harness_deadline(),
            HarnessDeadline::CaseWindow { margin: Duration::from_millis(1234) },
            "--harness-margin-ms was parsed and then dropped"
        );

        // And it is the margin actually used when resolving against a case.
        // `small-8x8` declares daySeconds = [60].
        let case = bench::CaseSpec::from_bundled_fixture("small-8x8").expect("fixture loads");
        assert_eq!(
            runner.resolved_harness_deadline(&case),
            Duration::from_secs(60) + Duration::from_millis(1234)
        );
    }

    /// Number of bundled fixtures — the `--cases all` meaning that must not
    /// drift when generated cases are added.
    fn bundled_count() -> usize {
        bench::fixture::MatchFixtureLoader::bundled_fixtures().len()
    }

    /// The `--generated` flag parses and actually reaches corpus selection.
    #[test]
    fn the_generated_flag_selects_generated_cases() {
        // `days=2` is deliberately off road-heavy's default (3), so the id
        // must spell both overrides out — two specs that differ only there
        // must not collide.
        let args = parse(&["--generated", "open:seed=1,road-heavy:seed=7:size=small:days=2"]);
        assert_eq!(args.generated, "open:seed=1,road-heavy:seed=7:size=small:days=2");

        let corpus = load_corpus(&args.cases, &args.generated, "").expect("selection loads");
        // `--cases small-8x8` (the parse helper's default) + 2 generated.
        assert_eq!(corpus.len(), 3);
        assert!(corpus.case_ids().contains(&"fixture:small-8x8"));
        assert!(corpus.case_ids().contains(&"generated:v2:open:seed=1"));
        assert!(corpus.case_ids().contains(&"generated:v2:road-heavy:seed=7:size=small:days=2"));

        // The generated cases carry full provenance into the records' source.
        for case in &corpus.cases {
            if case.case_id.starts_with("generated:") {
                assert!(case.seed.is_some(), "{} must carry a seed", case.case_id);
                let params = case.generation_parameters.as_ref().expect("params recorded");
                assert_eq!(params["generator_version"], bench::GENERATOR_VERSION);
            }
        }
    }

    /// Issue #179 import hook, offline half: a `CaseSpec` JSON produced by
    /// `procon arena import-case` joins the corpus through `--case-files`,
    /// and a malformed file fails closed **before** any candidate runs.
    #[test]
    fn case_files_append_imported_cases_and_fail_closed() {
        let dir =
            std::env::temp_dir().join(format!("procon-eval-case-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("arena-import.json");
        // Fixture-shaped CaseSpec: identity + the flattened wire setting.
        let case_json = r#"{
            "case_id": "fixture:arena-imported",
            "seed": null,
            "generation_parameters": null,
            "source": {"kind": "fixture", "name": "arena-imported"},
            "startsAt": 1790000100,
            "daySeconds": [30],
            "daySteps": [16],
            "map": {
                "height": 8, "width": 8,
                "cells": [[0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],
                          [0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],
                          [0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0]]
            },
            "spots": [],
            "agents": [0, 8, 16],
            "fuelLimits": 30,
            "players": 2,
            "busyThreshold": 1,
            "jammedThreshold": 2
        }"#;
        std::fs::write(&path, case_json).expect("write case file");
        let path_str = path.to_str().expect("utf8 path");

        // Flag → corpus: `--cases small-8x8` (parse helper default) + 1 import.
        let args = parse(&["--case-files", path_str]);
        assert_eq!(args.case_files, path_str);
        let corpus = load_corpus(&args.cases, &args.generated, &args.case_files)
            .expect("imported case loads");
        assert_eq!(corpus.len(), 2);
        assert!(corpus.case_ids().contains(&"fixture:arena-imported"));

        // Empty value selects nothing — the default corpus is untouched.
        let untouched = load_corpus("small-8x8", "", "").expect("default loads");
        assert_eq!(untouched.len(), 1);

        // Garbage (an HTML shell, say) fails closed with a clear message.
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "<!DOCTYPE html><html></html>").expect("write garbage");
        let err = load_corpus("small-8x8", "", bad.to_str().expect("utf8 path"))
            .expect_err("garbage must not load");
        assert!(err.contains("not a CaseSpec JSON"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fuel axis (Issue #204) survives flag → corpus → recorded
    /// parameters, the default spelling adds no bytes to the id, and a
    /// nonsense mode dies as a case generation failure — never a run record.
    #[test]
    fn the_generated_flag_carries_the_fuel_axis() {
        let args = parse(&["--generated", "open:seed=1:fuel=tight"]);
        assert_eq!(args.generated, "open:seed=1:fuel=tight");
        let corpus = load_corpus(&args.cases, &args.generated, "").expect("tight selection loads");
        let case =
            corpus.cases.iter().find(|c| c.case_id.starts_with("generated:")).expect("generated");
        assert_eq!(case.case_id, "generated:v2:open:seed=1:fuel=tight");
        let params = case.generation_parameters.as_ref().expect("params recorded");
        assert_eq!(params["fuel"], "tight");

        // The default spelling is byte-identical to the pre-axis corpus.
        let default = load_corpus("all", "open:seed=1", "").expect("default loads");
        assert!(default.case_ids().contains(&"generated:v2:open:seed=1"));
        let case =
            default.cases.iter().find(|c| c.case_id.starts_with("generated:")).expect("generated");
        assert_eq!(
            case.generation_parameters.as_ref().expect("params recorded")["fuel"],
            "comfortable"
        );

        // A nonsense mode is a selection-time generation failure.
        let err = load_corpus("all", "open:seed=1:fuel=extreme", "").expect_err("bad mode");
        assert!(err.starts_with("case generation failure"), "{err}");
    }

    /// Generated cases are opt-in: `--cases all` alone still evaluates exactly
    /// the bundled fixtures, so no existing evidence changes meaning.
    #[test]
    fn cases_all_still_means_the_bundled_fixtures_only() {
        let fixtures_only = load_corpus("all", "", "").expect("bundled corpus loads");
        assert_eq!(fixtures_only.len(), bundled_count());
        assert!(fixtures_only.case_ids().iter().all(|id| id.starts_with("fixture:")));

        let mixed = load_corpus("all", "open:seed=1,road-heavy:seed=7", "").expect("mixed loads");
        assert_eq!(mixed.len(), bundled_count() + 2);
        // Fixtures first, generated appended — order is stable per selection.
        assert_eq!(mixed.case_ids()[0], "fixture:openapi-sample");
        assert_eq!(mixed.case_ids()[bundled_count()], "generated:v2:open:seed=1");
        assert_eq!(mixed.case_ids()[bundled_count() + 1], "generated:v2:road-heavy:seed=7");
    }

    /// One corpus object per batch: every sibling candidate runs against this
    /// exact case set, which is what keeps comparisons on one case axis.
    #[test]
    fn one_selection_is_one_shared_case_set() {
        let corpus = load_corpus("all", "many-series:seed=3,road-heavy:seed=3", "")
            .expect("selection loads");
        let ids = corpus.case_ids();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "case ids must be unique");
        // The two generated siblings share one seed per archetype: comparing
        // candidates on this corpus varies the candidate, never the cases.
        assert!(ids.contains(&"generated:v2:many-series:seed=3"));
        assert!(ids.contains(&"generated:v2:road-heavy:seed=3"));
    }

    /// A rejected selection surfaces as a *case generation failure* — a
    /// corpus-build error — not as a run record that could look like a
    /// candidate failing.
    #[test]
    fn a_bad_generated_selection_is_a_case_generation_failure() {
        let err = load_corpus("all", "open", "").expect_err("seed= is required");
        assert!(err.starts_with("case generation failure"), "{err}");
        assert!(err.contains("seed= is required"), "{err}");

        let err = load_corpus("all", "bogus:seed=1", "").expect_err("unknown archetype");
        assert!(err.starts_with("case generation failure"), "{err}");

        let err = load_corpus("all", "open:seed=1,open:seed=1", "").expect_err("duplicate ids");
        assert!(err.starts_with("case generation failure"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }
}
