//! `procon` — operator CLI.
//!
//! Subcommands:
//!   - `hello`                         : print the engine's hello payload.
//!   - `replay <path>`                 : run a JSON replay file end-to-end.
//!   - `warmup`                        : measure cold / warm / 1-day-smoke timing per solver.
//!   - `loop <days>`                   : drive the per-match solver loop (in-process transport).
//!   - `coordinator`                   : contest venue runtime (LAN match-server
//!     auto-detect + /agent POST with retry/timeout).
//!   - `match-config load <path>`      : pretty-print a match config JSON.
//!   - `match-config validate <path>`  : structurally validate a match config.
//!   - `version`                       : print package + protocol versions.
//!   - `bundle build <output.zip>`     : build a contest deliverable zip.

mod action_adapter;
mod arena;
mod arena_import;
mod bundle;
mod candidate_bridge;
mod coordinator_loop;
mod fixtures;
mod initial_strategy;
mod loop_runner;
mod match_config;
mod openapi_fixture;
mod protocol_to_engine;
mod solver_adapter;
mod tournament;
mod warmup;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use domain::{MatchId, PlayerId};
use engine::replay::Replay;
use engine::{EngineServer, EngineServerConfig};
use protocol::{HttpClientError, MatchClient, RetryPolicy};
use serde_json::json;
use solver_loop::InitialStrategy;
use tracing_subscriber::EnvFilter;

use crate::coordinator_loop::{CoordinatorLoopConfig, run_coordinator_loop};
use crate::loop_runner::{
    DayAction, DayPlan, DaySignal, LoopConfig, LoopError, MatchSetting, Solver, Submission,
    SubmissionAck, Transport, run_match_loop,
};
use crate::solver_adapter::{EngineLoopAdapter, WaitSolver};

#[derive(Debug, Parser)]
#[command(name = "procon", about = "procon-2026 operator CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Print the engine's hello payload.
    Hello,
    /// Run a JSON replay file.
    Replay {
        /// Path to a JSON file shaped as `engine::replay::Replay`.
        path: PathBuf,
    },
    /// Time cold/warm/1-day-smoke for every requested solver and emit a JSON report.
    Warmup {
        /// Comma-separated solver names (greedy, astar, loop, mcts). Defaults to all.
        #[arg(long, value_delimiter = ',', default_value = "greedy,astar,loop,mcts")]
        solvers: Vec<String>,
        /// Emit only the JSON report (no human-readable summary).
        #[arg(long)]
        json: bool,
    },
    /// Drive the per-match solver loop end-to-end with an in-process transport.
    Loop {
        /// Number of days the synthetic match will run for.
        #[arg(long, default_value_t = 3)]
        days: u32,
        /// Player id to register for the synthetic match.
        #[arg(long, default_value = "player-alpha")]
        player: String,
    },
    /// Run the contest coordinator. Auto-detects the match server on the LAN
    /// (`PROCON_SERVER` or default localhost), drives the per-day match loop
    /// via `protocol::MatchClient`. The legacy `ureq`-based probe paths were
    /// removed in #137.
    Coordinator {
        /// Override `PROCON_SERVER` (host:port or http(s)://host:port).
        #[arg(long, env = "PROCON_SERVER")]
        server: Option<String>,

        /// Override `PROCON_TOKEN`.
        #[arg(long, env = "PROCON_TOKEN")]
        token: Option<String>,

        /// Solver strategy the coordinator drives:
        /// - `loop` (default) — `crates/solver-loop::LoopSolver`, the
        ///   multi-loop composition strategy from
        ///   `docs/contest/strategy.md`.
        /// - `wait`           — `WaitSolver`, every agent waits every
        ///   step. Useful as a debug smoke test for the wire protocol
        ///   without invoking a real strategy.
        #[arg(long, default_value = "loop", value_parser = ["loop", "wait"])]
        solver: String,

        /// Initial strategy for the `loop` solver — the same label set as
        /// `tournament --loop-initial-strategy`
        /// (`greedy | identity | reverse | sweep-row | sweep-col | random:<seed>`,
        /// `PascalCase` aliases accepted; offline↔live table in
        /// `crates/cli/src/initial_strategy.rs`). Ignored by `--solver wait`.
        /// An unknown label is an error, never a silent fallback.
        #[arg(
            long,
            default_value = "greedy",
            value_parser = initial_strategy::parse_initial_strategy
        )]
        loop_initial_strategy: InitialStrategy,

        /// Iteration cap for the `loop` solver's local search (same flag as
        /// `tournament --loop-iterations`). Ignored by `--solver wait`.
        #[arg(long, default_value_t = 32)]
        loop_iterations: u32,

        /// Only resolve config + print it. Do not actually start the loop.
        #[arg(long)]
        dry_run: bool,

        /// Path to write a JSON snapshot of the per-day match state.
        /// The frontend Live page polls this path via Vite's static-file
        /// serving, so the file must be reachable from the Vite dev
        /// server's filesystem root. Atomic write via tmp file + rename,
        /// so a half-written file is never visible to the frontend.
        #[arg(long, value_name = "PATH", env = "PROCON_STATE_OUTPUT")]
        state_output: Option<PathBuf>,

        /// Exit with status 0 as soon as the match completes instead of
        /// idle-ticking after it (Issue #226 local repro, scripted runs).
        /// The default keeps the post-match idle-tick so the container
        /// stays healthy for ops to inspect.
        #[arg(long, env = "PROCON_EXIT_AFTER_MATCH")]
        exit_after_match: bool,

        /// Run against the unofficial practice arena (Issue #179).
        ///
        /// This is a **profile layer**, not a second coordinator: the same
        /// `protocol::MatchClient` and `run_coordinator_loop` drive the
        /// match. `--arena` only supplies defaults — server from
        /// `PROCON_ARENA_SERVER` (default `https://procon37arena.online`),
        /// token from `PROCON_ARENA_TOKEN` (injected via Infisical; never a
        /// CLI argument on the standard path) — and appends a durable run
        /// record to `target/arena/runs.jsonl`. Explicit `--server` /
        /// `--token` (or `PROCON_SERVER` / `PROCON_TOKEN`) still win.
        #[arg(long)]
        arena: bool,

        /// Wait for the next arena match (`:00/:15/:30/:45` cadence),
        /// register inside the pre-match window by polling `GET /setting`
        /// with the arena token (403 during the wait is not an error), then
        /// play exactly one match and exit after recording evidence.
        /// Requires `--arena`.
        #[arg(long)]
        next: bool,

        /// Offline-selected candidate to play, using the Issue #178
        /// registry id verbatim (e.g. `multiloop:kmeans=4:seed=42`,
        /// `astar:policy=step`). Resolved through
        /// `bench::CandidateRegistry` and adapted onto the canonical
        /// coordinator loop — overrides `--solver` and the loop knobs, and
        /// the id is recorded in the arena evidence for correspondence
        /// with the offline `RunRecord` that selected it.
        #[arg(long)]
        candidate: Option<String>,

        /// JSONL evidence path for `--arena` runs.
        /// Default: `target/arena/runs.jsonl`.
        #[arg(long, value_name = "PATH")]
        arena_evidence: Option<PathBuf>,
    },
    /// Arena utilities that do not touch the network.
    #[command(subcommand)]
    Arena(ArenaCmd),
    /// Inspect or validate a match configuration JSON file.
    #[command(subcommand)]
    MatchConfig(MatchConfigCmd),
    /// Print package and protocol versions.
    Version,
    /// Build, inspect, and verify contest deliverable bundles.
    Bundle {
        #[command(subcommand)]
        action: BundleCmd,
    },
    /// Run a tournament: drive each solver through a full match against the
    /// server, score them, and emit a markdown leaderboard.
    ///
    /// Solver names must be in the same registry as `Cmd::Coordinator`'s
    /// `--solver` (`loop`, `wait`). Matches run sequentially because the
    /// contest server only honours one match per session. Score =
    /// `days_completed * 100 + sum(revisions)`; higher is better.
    Tournament {
        /// Comma-separated solver names (e.g. `loop,wait`). Must be a
        /// non-empty subset of the registry in
        /// `Cmd::Coordinator::--solver`.
        #[arg(long, value_delimiter = ',')]
        solvers: Vec<String>,
        /// Override `PROCON_SERVER`.
        #[arg(long, env = "PROCON_SERVER")]
        server: Option<String>,
        /// Override `PROCON_TOKEN`.
        #[arg(long, env = "PROCON_TOKEN")]
        token: Option<String>,
        /// Initial strategy for the `loop` solver —
        /// `greedy | identity | reverse | sweep-row | sweep-col | random:<seed>`
        /// (`PascalCase` aliases accepted). Each label maps 1:1 onto an
        /// offline `loop:initial=<value>` candidate parameter; see
        /// `crates/cli/src/initial_strategy.rs`. An unknown label is an
        /// error, never a silent `Identity` fallback.
        #[arg(
            long,
            default_value = "greedy",
            value_parser = initial_strategy::parse_initial_strategy
        )]
        loop_initial_strategy: InitialStrategy,
        /// Iteration cap for the `loop` solver's local search.
        #[arg(long, default_value_t = 32)]
        loop_iterations: u32,
        /// Team id baked into the coordinator loop config.
        #[arg(long, default_value = "team-p0")]
        team_id: String,
        /// Directory to write a JSON leaderboard sidecar. When set,
        /// `<DIR>/leaderboard.json` is written atomically (tmp + rename).
        #[arg(long, value_name = "DIR")]
        state_output_dir: Option<PathBuf>,
        /// Optional shell command to invoke between solvers to restart the
        /// match server. The official `procon-server-linux-amd64` only
        /// supports one match per process, so multi-solver tournaments
        /// need a restart hook (e.g.
        /// `./scripts/restart-official-server.sh 8080`). The command is
        /// passed verbatim to `sh -c`, so shell pipelines are honoured.
        /// When unset, every solver shares the same server process.
        #[arg(long, value_name = "CMD", env = "PROCON_AUTO_RESTART_SERVER")]
        auto_restart_server: Option<String>,
    },
}

/// Subcommands under `procon bundle`.
#[derive(Debug, Subcommand)]
enum BundleCmd {
    /// Build a contest deliverable zip at the given output path.
    Build {
        /// Destination path for the produced zip file.
        output: PathBuf,
    },
}

/// Subcommands under `procon match-config`.
#[derive(Debug, Subcommand)]
enum MatchConfigCmd {
    /// Read and pretty-print a match configuration JSON file.
    Load {
        /// Path to a match configuration JSON file.
        path: PathBuf,
    },
    /// Validate a match configuration JSON file (basic structural checks).
    Validate {
        /// Path to a match configuration JSON file.
        path: PathBuf,
    },
}

/// Subcommands under `procon arena` (offline helpers — no network).
#[derive(Debug, Subcommand)]
enum ArenaCmd {
    /// Convert the arena's same-board replay JSON into an offline
    /// `CaseSpec` so an arena loss can be re-run through the Issue #178
    /// evaluator (`procon-eval --case-files <output>`).
    ImportCase {
        /// Path to the arena replay JSON.
        #[arg(long, value_name = "PATH")]
        input: PathBuf,
        /// Where to write the `CaseSpec` JSON.
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
        /// Case name (`fixture:<name>`). Defaults to the replay's match
        /// identifier, sanitised to `[A-Za-z0-9._-]`.
        #[arg(long)]
        name: Option<String>,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

// One arm per `Cmd` variant: subcommand dispatch, not a single logic block.
#[allow(clippy::too_many_lines, reason = "one arm per Cmd variant")]
async fn run(cli: Cli) -> Result<()> {
    let server = EngineServer::new(EngineServerConfig::default());
    match cli.cmd {
        Cmd::Hello => {
            let hello = server.hello();
            println!("{}", serde_json::to_string_pretty(&hello)?);
        }
        Cmd::Replay { path } => {
            let raw =
                std::fs::read(&path).with_context(|| format!("read replay {}", path.display()))?;
            let replay: Replay = serde_json::from_slice(&raw)
                .with_context(|| format!("parse replay {}", path.display()))?;
            let runner = engine::replay::ReplayRunner::new();
            let state = runner
                .run(replay.clone())
                .await
                .with_context(|| format!("replay {} failed", path.display()))?;
            println!(
                "{}",
                json!({
                    "match_id": replay.match_id,
                    "final_turn": state.turn,
                    "scores": state.scores,
                })
            );
        }
        Cmd::Warmup { solvers, json } => {
            let setting = fixtures::tiny_setting();
            let names: Vec<&str> = solvers.iter().map(String::as_str).collect();
            let report = warmup::run_warmup(&setting, &names)
                .map_err(|e| anyhow::anyhow!("warmup failed: {e}"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", warmup::format_report_human(&report));
            }
        }
        Cmd::Loop { days, player } => {
            run_loop(days, &player).await?;
        }
        Cmd::Coordinator {
            server,
            token,
            solver,
            loop_initial_strategy,
            loop_iterations,
            dry_run,
            state_output,
            exit_after_match,
            arena,
            next,
            candidate,
            arena_evidence,
        } => {
            cmd_coordinator(
                server,
                token,
                solver,
                loop_initial_strategy,
                loop_iterations,
                dry_run,
                state_output,
                exit_after_match,
                ArenaFlags { arena, next, candidate, arena_evidence },
            )
            .await?;
        }
        Cmd::Arena(sub) => cmd_arena(sub)?,
        Cmd::MatchConfig(sub) => cmd_match_config(sub)?,
        Cmd::Version => {
            println!(
                "{}",
                json!({
                    "package": env!("CARGO_PKG_VERSION"),
                    "protocol": protocol::PROTOCOL_VERSION,
                    "rustc": env!("CARGO_PKG_RUST_VERSION"),
                })
            );
        }
        Cmd::Bundle { action } => match action {
            BundleCmd::Build { output } => {
                bundle::run_build(&output).await?;
            }
        },
        Cmd::Tournament {
            solvers,
            server,
            token,
            loop_initial_strategy,
            loop_iterations,
            team_id,
            state_output_dir,
            auto_restart_server,
        } => {
            cmd_tournament(
                solvers,
                server,
                token,
                loop_initial_strategy,
                loop_iterations,
                team_id,
                state_output_dir,
                auto_restart_server,
            )
            .await?;
        }
    }
    Ok(())
}

/// `match-config` subcommand dispatcher.
fn cmd_match_config(sub: MatchConfigCmd) -> Result<()> {
    match sub {
        MatchConfigCmd::Load { path } => {
            let (_cfg, value) = match_config::load(&path)
                .with_context(|| format!("load match config {}", path.display()))?;
            println!("{}", match_config::pretty_print(&value)?);
        }
        MatchConfigCmd::Validate { path } => {
            let raw = std::fs::read(&path)
                .with_context(|| format!("read match config {}", path.display()))?;
            let value: serde_json::Value = serde_json::from_slice(&raw)
                .with_context(|| format!("parse match config {}", path.display()))?;
            let report = match_config::validate(&value);
            let summary = json!({
                "path": match_config::display_path(&path).display().to_string(),
                "ok": report.ok,
                "errors": report.errors,
            });
            println!("{}", serde_json::to_string_pretty(&summary)?);
            if !report.ok {
                anyhow::bail!("match config validation failed");
            }
        }
    }
    Ok(())
}

/// Arena-mode inputs for [`cmd_coordinator`], bundled so the dispatcher's
/// argument list stays manageable.
struct ArenaFlags {
    /// `--arena` — resolve the arena profile and record run evidence.
    arena: bool,
    /// `--next` — wait for the next `:00/:15/:30/:45` window and register.
    next: bool,
    /// `--candidate` — Issue #178 registry id to play (overrides `--solver`).
    candidate: Option<String>,
    /// `--arena-evidence` — evidence JSONL path override.
    arena_evidence: Option<PathBuf>,
}

/// `coordinator` subcommand dispatcher.
///
/// Behaviour matrix:
///
/// - `--dry-run` with no `--server`: load the `OpenAPI` + `match-config`
///   fixture from `.reference/procon37/`, build a real `protocol::MatchClient`
///   targeting `http://localhost:7878` with the MVP test token, and print
///   the number of days from the loaded `MatchSetting`. The full loop
///   driving the mockito server lands in #136.
/// - `--dry-run` with `--server`: print a minimal resolved-config dump
///   (server URL + `has_token`) and exit.
/// - default: build a `MatchClient`, build the `--solver` strategy
///   (`loop` → `EngineLoopAdapter` wrapping `solver_loop::LoopSolver`,
///   `wait` → `WaitSolver`) — or the `--candidate` bridge when an Issue
///   #178 candidate id is given — hand both to the wired-up
///   `run_coordinator_loop`, and idle-block on the per-day driver.
///   When the real loop completes, we idle-tick forever so the container
///   stays healthy for ops to inspect — unless `--exit-after-match`
///   (Issue #226 local repro / scripted runs) is set, in which case the
///   process returns `Ok(())` and exits 0.
/// - `--arena`: the same path with an env-backed profile
///   (`PROCON_ARENA_SERVER` / `PROCON_ARENA_TOKEN`), a durable run record
///   appended to `target/arena/runs.jsonl`, and **exit after one match**
///   instead of idle-ticking. `--arena --next` additionally waits for the
///   next cadence window and registers via `GET /setting` first
///   (Issue #179, path corrected in Issue #222). No arena-specific
///   protocol client and no second coordinator — the profile only
///   supplies the values the canonical path already takes.
#[allow(clippy::too_many_arguments, reason = "arena flags arrive as a bundle; see ArenaFlags")]
#[allow(clippy::too_many_lines, reason = "one dispatcher, one behaviour matrix")]
async fn cmd_coordinator(
    server: Option<String>,
    token: Option<String>,
    solver: String,
    loop_initial_strategy: InitialStrategy,
    loop_iterations: u32,
    dry_run: bool,
    state_output: Option<PathBuf>,
    exit_after_match: bool,
    arena_flags: ArenaFlags,
) -> Result<()> {
    let ArenaFlags { arena, next, candidate, arena_evidence } = arena_flags;
    if next && !arena {
        anyhow::bail!(
            "--next requires --arena: the :00/:15/:30/:45 cadence and the \
             /setting pre-match registration are arena-specific"
        );
    }

    // Arena profile: env-backed defaults for server/token. Resolved before
    // `--dry-run` so a dry run reports exactly what a real run would use.
    let profile = if arena {
        let env = arena::ArenaEnv::from_env();
        Some(
            arena::resolve_profile(server.clone(), token.clone(), env)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        )
    } else {
        None
    };
    // Explicit `--server` / `--token` (clap already merges PROCON_SERVER /
    // PROCON_TOKEN) still win; the arena profile only fills the defaults.
    let effective_server =
        profile.as_ref().map(|p| p.server_url.clone()).or_else(|| server.clone());
    let effective_token =
        profile.as_ref().map(|p| p.token.expose().to_owned()).or_else(|| token.clone());

    if dry_run {
        if effective_server.is_none() {
            // Load the OpenAPI fixture and build a real MatchClient — but
            // do NOT call the loop yet; full E2E driving belongs to #C.
            let openapi_path = std::path::Path::new(".reference/procon37/openapi.json");
            let match_config_path =
                std::path::Path::new(".reference/procon37/match-config.example.json");
            let fixture =
                crate::openapi_fixture::OpenApiFixture::load(openapi_path, match_config_path)
                    .map_err(|e| anyhow::anyhow!("openapi fixture load: {e}"))?;
            fixture
                .assert_minimal_coverage()
                .map_err(|e| anyhow::anyhow!("openapi coverage: {e}"))?;

            let client = MatchClient::builder("http://localhost:7878", "mvp-test-token")
                .retry_policy(RetryPolicy::default())
                .request_timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e: HttpClientError| anyhow::anyhow!("match client: {e}"))?;
            println!(
                "{}",
                json!({
                    "dry_run": true,
                    "match_config_days": fixture
                        .match_setting
                        .as_ref()
                        .map(|s| s.day_seconds.len())
                        .or_else(|| {
                            fixture.match_config_raw
                                .get("problem")?
                                .get("daySeconds")?
                                .as_array()
                                .map(Vec::len)
                        })
                        .unwrap_or(0),
                    "client_base_url": "http://localhost:7878",
                })
            );
            // Touch the unused-result slot so the linter does not flag it.
            let _ = client;
            return Ok(());
        }
        // dry_run + resolved server: minimal config dump. `has_token` is a
        // boolean precisely so the token itself can never be printed.
        let base_url = effective_server.expect("checked above");
        let token = effective_token.unwrap_or_default();
        println!(
            "{}",
            json!({
                "server": base_url,
                "has_token": !token.is_empty(),
                "dry_run": true,
                "arena": profile.is_some(),
            })
        );
        return Ok(());
    }

    // Default behaviour: drive the real loop against the live server.
    let base_url = effective_server
        .ok_or_else(|| anyhow::anyhow!("PROCON_SERVER (or --server) must be set"))?;
    let token = effective_token.unwrap_or_default();
    let client = MatchClient::builder(base_url.clone(), token)
        .retry_policy(RetryPolicy::default())
        .request_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e: HttpClientError| anyhow::anyhow!("match client: {e}"))?;

    // Pick the solver: an Issue #178 candidate (`--candidate`) or the
    // `--solver` flag. The `value_parser` restricts `--solver` to
    // {"loop", "wait"}; the `match` below is exhaustive with an
    // `unreachable!()` fallback.
    //
    // The loop adapter exposes a setting slot so the coordinator loop
    // can hand the parsed engine setting (real map + agent positions)
    // back to the solver after `protocol_setting_to_engine`. Without
    // this injection the adapter would plan against a stand-in problem
    // and emit moves that fall off the real map, causing the official
    // server to reject day 0 with `revision=-1` (issue #158).
    //
    // `--loop-initial-strategy` / `--loop-iterations` are the same knobs
    // `tournament` exposes (Issue #170): one parser, one surface, no
    // per-variant protocol client.
    let solver_strategy = solver.clone();
    let mut live_state_slot = None;
    let mut candidate_id: Option<String> = None;
    let mut solver_label = solver_strategy.clone();
    let (solver, engine_setting_slot) = if let Some(id) = candidate.as_deref() {
        let bridge = candidate_bridge::CandidateBridge::from_registry_id(id)
            .map_err(|e| anyhow::anyhow!("bad --candidate {id:?}: {e}"))?;
        let canonical = bridge.candidate_id().to_owned();
        tracing::info!(candidate = %canonical, "offline candidate selected for the live run");
        candidate_id = Some(canonical.clone());
        solver_label = format!("candidate:{canonical}");
        let slots = bridge.slots();
        live_state_slot = Some(slots.live_state);
        (Box::new(bridge) as Box<dyn engine::solver::Solver>, Some(slots.setting))
    } else {
        match solver_strategy.as_str() {
            "loop" => {
                tracing::info!(
                    strategy = %initial_strategy::initial_strategy_label(loop_initial_strategy),
                    iterations = loop_iterations,
                    "loop initial strategy selected"
                );
                let adapter = EngineLoopAdapter::new(loop_initial_strategy, loop_iterations);
                let slot = adapter.setting_handle();
                (Box::new(adapter) as Box<dyn engine::solver::Solver>, Some(slot))
            }
            "wait" => (Box::new(WaitSolver) as Box<dyn engine::solver::Solver>, None),
            other => {
                unreachable!(
                    "--solver value_parser already restricts to {{loop, wait}}, got {other}"
                )
            }
        }
    };
    tracing::info!(solver = solver.name(), "coordinator loop strategy selected");

    let loop_cfg = CoordinatorLoopConfig {
        solver_name: solver_label.clone(),
        // `MatchClient::base_url()` is not exposed publicly (we must not
        // modify `crates/protocol`), so we forward the URL we handed to
        // the builder above. The snapshot's `server` field mirrors this
        // value verbatim.
        server_url: Some(base_url.clone()),
        state_output_path: state_output.clone(),
        engine_setting_slot,
        live_state_slot,
        secret_redaction: profile.as_ref().map(|p| p.token.clone()),
        ..Default::default()
    };

    // Arena mode: one match, then evidence, then exit (no idle tick).
    if let Some(profile) = profile.as_ref() {
        let evidence_path = arena_evidence.clone().unwrap_or_else(arena::default_evidence_path);
        let solver_cfg = if let Some(canonical) = candidate_id.as_deref() {
            arena::ArenaSolverConfig {
                mode: "candidate".to_owned(),
                solver: canonical.split(':').next().unwrap_or(canonical).to_owned(),
                loop_initial_strategy: None,
                loop_iterations: None,
            }
        } else {
            arena::ArenaSolverConfig {
                mode: "cli-solver".to_owned(),
                solver: solver_strategy.clone(),
                loop_initial_strategy: Some(initial_strategy::initial_strategy_label(
                    loop_initial_strategy,
                )),
                loop_iterations: Some(loop_iterations),
            }
        };
        return run_arena_once(
            profile,
            client,
            solver,
            loop_cfg,
            next,
            candidate_id,
            solver_cfg,
            &evidence_path,
        )
        .await;
    }

    let outcome = run_coordinator_loop(client, solver, loop_cfg)
        .await
        .map_err(|e| anyhow::anyhow!("coordinator loop: {e}"))?;

    tracing::warn!(
        "coordinator loop wired in #135; full E2E driving comes in #136. \
         outcome: {outcome:?}"
    );
    if exit_after_match {
        // Issue #226 (and scripted runs generally): a completed match is a
        // successful process. Without this the CLI idle-ticks forever and
        // no wrapper can ever observe exit 0.
        tracing::info!(
            "--exit-after-match: {days} day(s) completed, exiting 0",
            days = outcome.days_completed
        );
        return Ok(());
    }
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        ticker.tick().await;
    }
}

/// Run exactly one arena match through the canonical coordinator loop and
/// record durable evidence for it (Issue #179).
///
/// Sequence: optional pre-match registration (`--next`) →
/// `run_coordinator_loop` → append an [`arena::ArenaRunRecord`] to
/// `evidence_path` → return. Every failure path also records evidence, so
/// a failed join or an API drift is just as durable as a completed match.
/// All error text is redacted through the profile token before it reaches
/// the caller (and therefore stderr).
#[allow(clippy::too_many_arguments, reason = "one call site, explicit inputs")]
async fn run_arena_once(
    profile: &arena::ArenaProfile,
    client: protocol::MatchClient,
    solver: Box<dyn engine::solver::Solver>,
    loop_cfg: CoordinatorLoopConfig,
    next: bool,
    candidate_id: Option<String>,
    solver_cfg: arena::ArenaSolverConfig,
    evidence_path: &std::path::Path,
) -> Result<()> {
    let team_id = loop_cfg.team_id.clone();
    // Env override → `git rev-parse HEAD` → "unknown": the live smoke ran
    // `cargo run` without the `just`-injected env var and recorded
    // "unknown" (Issue #222).
    let source_commit_sha = arena::source_commit_sha();

    // Phase A: pre-match registration (only with `--next`; that is the
    // documented standard entry point `just arena-next`).
    let mut registration = None;
    let mut early: Option<(arena::ArenaRunStatus, Option<String>)> = None;
    if next {
        match arena::wait_for_registration(
            &client,
            profile,
            &arena::RegistrationOptions::default(),
            chrono::Utc::now(),
        )
        .await
        {
            Ok(outcome) => {
                tracing::info!(
                    attempts = outcome.attempts,
                    registered_at = %outcome.registered_at,
                    match_id = ?outcome.arena_match_id,
                    "arena registration complete"
                );
                registration = Some(outcome);
            }
            Err(e) => {
                tracing::error!(error = %e, "arena registration failed");
                early = Some((e.status(), Some(e.to_string())));
            }
        }
    } else {
        tracing::info!(
            "arena: pre-match registration skipped (use --next to wait for \
             the next :00/:15/:30/:45 window and register automatically)"
        );
    }

    // Phase B: play through the unchanged canonical loop.
    let mut outcome = None;
    let (status, error) = if let Some(early) = early {
        early
    } else {
        let result = run_coordinator_loop(client, solver, loop_cfg).await;
        let (status, error) = arena::classify_loop_result(&result);
        if let Ok(o) = &result {
            outcome = Some(o.clone());
        }
        (status, error)
    };

    // Phase C: durable evidence — for success *and* failure alike.
    let window_starts_at_unix = registration.as_ref().map_or_else(
        || {
            let lead = std::time::Duration::from_secs(
                u64::try_from(arena::ARENA_PRE_MATCH_LEAD_SECS).unwrap_or(180),
            );
            arena::next_match_window(chrono::Utc::now(), lead).starts_at.timestamp()
        },
        |r| r.window.starts_at.timestamp(),
    );
    let record = arena::ArenaRunRecord::build(arena::ArenaRecordInput {
        profile,
        candidate_id: candidate_id.as_deref(),
        solver: solver_cfg,
        team_id: &team_id,
        source_commit_sha,
        window_starts_at_unix,
        registration: registration.as_ref(),
        outcome: outcome.as_ref(),
        status,
        error: error.clone(),
    });
    arena::append_evidence(evidence_path, &record)
        .map_err(|e| anyhow::anyhow!("arena evidence could not be written: {e}"))?;
    tracing::info!(
        status = %status,
        path = %evidence_path.display(),
        candidate = candidate_id.as_deref().unwrap_or("-"),
        "arena run recorded"
    );

    if status == arena::ArenaRunStatus::Completed {
        return Ok(());
    }
    let text = error.unwrap_or_else(|| format!("arena run ended with status {status}"));
    Err(anyhow::anyhow!("{}", profile.token.redact(&text)))
}

/// `arena` subcommand dispatcher (offline helpers, no network).
fn cmd_arena(sub: ArenaCmd) -> Result<()> {
    match sub {
        ArenaCmd::ImportCase { input, output, name } => {
            let case = arena_import::case_from_replay_file(&input, name.as_deref())
                .map_err(|e| anyhow::anyhow!("arena import: {e}"))?;
            arena_import::write_case(&output, &case)
                .map_err(|e| anyhow::anyhow!("arena import: {e}"))?;
            println!(
                "{}",
                json!({
                    "case_id": case.case_id,
                    "output": output.display().to_string(),
                    "days": case.setting.day_steps.len(),
                })
            );
        }
    }
    Ok(())
}

/// `tournament` subcommand dispatcher.
///
/// Resolves the env / flag fallback for `--server` and `--token`, builds a
/// [`TournamentConfig`], runs every solver sequentially through a real
/// match against the server, prints the markdown leaderboard to stdout,
/// and optionally writes `<DIR>/leaderboard.json` (atomic tmp + rename).
#[allow(clippy::too_many_arguments)]
async fn cmd_tournament(
    solvers: Vec<String>,
    server: Option<String>,
    token: Option<String>,
    loop_initial_strategy: InitialStrategy,
    loop_iterations: u32,
    team_id: String,
    state_output_dir: Option<PathBuf>,
    auto_restart_server: Option<String>,
) -> Result<()> {
    use crate::tournament::{TournamentConfig, render_markdown, run_tournament};

    let cfg = TournamentConfig {
        server: server
            .or_else(|| std::env::var("PROCON_SERVER").ok())
            .unwrap_or_else(|| "http://localhost:8080".to_owned()),
        token: token
            .or_else(|| std::env::var("PROCON_TOKEN").ok())
            .unwrap_or_else(|| "token-p0".to_owned()),
        solvers,
        loop_initial_strategy,
        loop_iterations,
        team_id,
        state_output_dir,
        auto_restart_server,
    };
    tracing::info!(
        strategy = %initial_strategy::initial_strategy_label(cfg.loop_initial_strategy),
        iterations = cfg.loop_iterations,
        "tournament loop initial strategy selected"
    );
    let report = run_tournament(cfg).await.map_err(|e| anyhow::anyhow!("tournament: {e}"))?;
    print!("{}", render_markdown(&report));
    Ok(())
}

/// Drive the per-match solver loop with an in-process transport.
async fn run_loop(days: u32, player: &str) -> Result<()> {
    let match_id =
        MatchId::new("match-cli-1").map_err(|e| anyhow::anyhow!("build match id: {e}"))?;
    let player_id = PlayerId::new(player).map_err(|e| anyhow::anyhow!("build player id: {e}"))?;
    let setting = MatchSetting::new(match_id.clone(), days, vec![player_id.clone()], 60);

    let solver = EchoSolver { player: player_id };
    let transport = StubTransport;
    let config = LoopConfig::default();

    let score = run_match_loop(setting, &solver, &transport, &config)
        .await
        .map_err(|e| anyhow::anyhow!("coordinator loop failed: {e}"))?;

    println!(
        "{}",
        json!({
            "match_id": match_id,
            "final_score": score.0,
            "days_requested": days,
        })
    );
    Ok(())
}

/// Placeholder solver that emits a single no-op action per day.
struct EchoSolver {
    player: PlayerId,
}

impl Solver for EchoSolver {
    fn decide(
        &self,
        _setting: &MatchSetting,
        day: u32,
    ) -> Result<DayPlan, crate::loop_runner::LoopError> {
        let action_index = u16::try_from(day).unwrap_or(u16::MAX);
        Ok(DayPlan::new(
            day,
            vec![DayAction {
                agent_id: self.player.clone(),
                action_index,
                payload: serde_json::json!({ "noop": true, "day": day }),
            }],
        ))
    }
}

/// In-process transport that always acks and advances the day.
struct StubTransport;

impl Transport for StubTransport {
    async fn post_submission(&self, _submission: &Submission) -> Result<SubmissionAck, LoopError> {
        tokio::task::yield_now().await;
        Ok(SubmissionAck { accepted: true, reason: None })
    }

    async fn wait_for_next_day(&self, current_day: u32) -> Result<DaySignal, LoopError> {
        tokio::task::yield_now().await;
        Ok(DaySignal::Continue { next_day: current_day + 1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_version() {
        let cli = Cli::try_parse_from(["procon", "version"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Version));
    }

    #[test]
    fn parses_bundle_build() {
        let cli =
            Cli::try_parse_from(["procon", "bundle", "build", "/tmp/deliverable.zip"]).unwrap();
        match cli.cmd {
            Cmd::Bundle { action } => match action {
                BundleCmd::Build { output } => {
                    assert_eq!(output, PathBuf::from("/tmp/deliverable.zip"));
                }
            },
            _ => panic!("expected Bundle"),
        }
    }

    #[test]
    fn parses_match_config_load() {
        let cli = Cli::try_parse_from(["procon", "match-config", "load", "/tmp/foo.json"]).unwrap();
        match cli.cmd {
            Cmd::MatchConfig(MatchConfigCmd::Load { path }) => {
                assert_eq!(path.to_str(), Some("/tmp/foo.json"));
            }
            _ => panic!("expected match-config load"),
        }
    }

    #[test]
    fn parses_match_config_validate() {
        let cli =
            Cli::try_parse_from(["procon", "match-config", "validate", "/tmp/foo.json"]).unwrap();
        match cli.cmd {
            Cmd::MatchConfig(MatchConfigCmd::Validate { path }) => {
                assert_eq!(path.to_str(), Some("/tmp/foo.json"));
            }
            _ => panic!("expected match-config validate"),
        }
    }

    #[test]
    fn parses_coordinator_dry_run() {
        let cli = Cli::try_parse_from(["procon", "coordinator", "--dry-run"]).unwrap();
        match cli.cmd {
            Cmd::Coordinator { dry_run, .. } => assert!(dry_run),
            _ => panic!("expected Coordinator variant"),
        }
    }

    #[test]
    fn parses_coordinator_state_output() {
        let cli =
            Cli::try_parse_from(["procon", "coordinator", "--state-output", "/tmp/state.json"])
                .unwrap();
        match cli.cmd {
            Cmd::Coordinator { state_output, .. } => {
                assert_eq!(state_output, Some(PathBuf::from("/tmp/state.json")));
            }
            _ => panic!("expected Coordinator variant"),
        }
    }

    #[test]
    fn parses_coordinator_state_output_defaults_none() {
        let cli = Cli::try_parse_from(["procon", "coordinator"]).unwrap();
        match cli.cmd {
            Cmd::Coordinator { state_output, .. } => assert_eq!(state_output, None),
            _ => panic!("expected Coordinator variant"),
        }
    }

    /// Issue #179: the arena profile, cadence wait, candidate selection and
    /// evidence path all parse — and only under explicit flags.
    #[test]
    fn parses_arena_coordinator_flags() {
        let cli = Cli::try_parse_from([
            "procon",
            "coordinator",
            "--arena",
            "--next",
            "--candidate",
            "multiloop:kmeans=4:seed=42",
            "--arena-evidence",
            "/tmp/arena-runs.jsonl",
        ])
        .expect("arena invocation parses");
        match cli.cmd {
            Cmd::Coordinator { arena, next, candidate, arena_evidence, .. } => {
                assert!(arena, "--arena");
                assert!(next, "--next");
                assert_eq!(candidate.as_deref(), Some("multiloop:kmeans=4:seed=42"));
                assert_eq!(arena_evidence, Some(PathBuf::from("/tmp/arena-runs.jsonl")));
            }
            _ => panic!("expected Coordinator variant"),
        }
    }

    /// Offline isolation (AC): every arena behaviour is opt-in. A plain
    /// `procon coordinator` parses with the arena flags at their inert
    /// defaults, so the default path cannot have changed meaning.
    #[test]
    fn arena_flags_are_opt_in() {
        let cli = Cli::try_parse_from(["procon", "coordinator"]).expect("parses");
        match cli.cmd {
            Cmd::Coordinator { arena, next, candidate, arena_evidence, .. } => {
                assert!(!arena, "arena must be off by default");
                assert!(!next, "--next must be off by default");
                assert!(candidate.is_none(), "no candidate implied");
                assert!(arena_evidence.is_none(), "evidence path only in arena mode");
            }
            _ => panic!("expected Coordinator variant"),
        }
    }

    /// Token hygiene (AC): `PROCON_ARENA_TOKEN` is read from the
    /// environment in code — there is no flag that could carry it (the
    /// standard flow never prints or types a token).
    #[test]
    fn no_flag_carries_the_arena_token() {
        let err = Cli::try_parse_from(["procon", "coordinator", "--arena-token", "shhh"])
            .expect_err("there is deliberately no --arena-token flag");
        assert!(err.to_string().contains("arena-token"), "{err}");
    }

    /// The import hook is an offline subcommand of `procon arena`.
    #[test]
    fn parses_arena_import_case() {
        let cli = Cli::try_parse_from([
            "procon",
            "arena",
            "import-case",
            "--input",
            "replay.json",
            "--output",
            "case.json",
            "--name",
            "loss-001",
        ])
        .expect("import-case parses");
        match cli.cmd {
            Cmd::Arena(ArenaCmd::ImportCase { input, output, name }) => {
                assert_eq!(input, PathBuf::from("replay.json"));
                assert_eq!(output, PathBuf::from("case.json"));
                assert_eq!(name.as_deref(), Some("loss-001"));
            }
            _ => panic!("expected Arena(ImportCase)"),
        }
    }

    /// Feed `--loop-initial-strategy <label>` to `tournament` and return the
    /// parsed strategy, or the (clap-wrapped) parser error text.
    fn tournament_initial_label(label: &str) -> Result<InitialStrategy, String> {
        Cli::try_parse_from(["procon", "tournament", "--loop-initial-strategy", label])
            .map_err(|e| e.to_string())
            .map(|cli| match cli.cmd {
                Cmd::Tournament { loop_initial_strategy, .. } => loop_initial_strategy,
                other => panic!("expected Tournament, got {other:?}"),
            })
    }

    /// Same for `coordinator` — the two subcommands share one parser.
    fn coordinator_initial_label(label: &str) -> Result<InitialStrategy, String> {
        Cli::try_parse_from(["procon", "coordinator", "--loop-initial-strategy", label])
            .map_err(|e| e.to_string())
            .map(|cli| match cli.cmd {
                Cmd::Coordinator { loop_initial_strategy, .. } => loop_initial_strategy,
                other => panic!("expected Coordinator, got {other:?}"),
            })
    }

    /// Issue #170: `tournament` **and** `coordinator` parse the whole live
    /// label set (`greedy`, `identity`, `reverse`, `sweep-row`, `sweep-col`,
    /// `random:<seed>`), and label → strategy → label round-trips.
    #[test]
    fn both_subcommands_parse_every_initial_strategy_label() {
        for (label, expected) in [
            ("greedy", InitialStrategy::GreedyNearestNeighbor),
            ("identity", InitialStrategy::Identity),
            ("reverse", InitialStrategy::Reverse),
            ("sweep-row", InitialStrategy::SweepRow),
            ("sweep-col", InitialStrategy::SweepCol),
            ("random:7", InitialStrategy::Random { seed: 7 }),
            ("random:0", InitialStrategy::Random { seed: 0 }),
            // Legacy PascalCase aliases stay accepted.
            ("GreedyNearestNeighbor", InitialStrategy::GreedyNearestNeighbor),
            ("Identity", InitialStrategy::Identity),
            ("Reverse", InitialStrategy::Reverse),
            ("SweepRow", InitialStrategy::SweepRow),
            ("SweepCol", InitialStrategy::SweepCol),
        ] {
            assert_eq!(tournament_initial_label(label), Ok(expected), "tournament {label}");
            assert_eq!(coordinator_initial_label(label), Ok(expected), "coordinator {label}");
            // Round-trip through the canonical label both flags report.
            let canonical = initial_strategy::initial_strategy_label(expected);
            assert_eq!(
                tournament_initial_label(&canonical),
                Ok(expected),
                "tournament round-trip of {canonical}"
            );
            assert_eq!(
                coordinator_initial_label(&canonical),
                Ok(expected),
                "coordinator round-trip of {canonical}"
            );
        }
    }

    /// Both flags default to the same strategy as the offline
    /// `loop:initial` default (`greedy`).
    #[test]
    fn initial_strategy_flags_default_to_greedy() {
        let cli = Cli::try_parse_from(["procon", "tournament"]).unwrap();
        match cli.cmd {
            Cmd::Tournament { loop_initial_strategy, .. } => {
                assert_eq!(loop_initial_strategy, InitialStrategy::GreedyNearestNeighbor);
            }
            other => panic!("expected Tournament, got {other:?}"),
        }
        let cli = Cli::try_parse_from(["procon", "coordinator"]).unwrap();
        match cli.cmd {
            Cmd::Coordinator { loop_initial_strategy, loop_iterations, .. } => {
                assert_eq!(loop_initial_strategy, InitialStrategy::GreedyNearestNeighbor);
                assert_eq!(loop_iterations, 32);
            }
            other => panic!("expected Coordinator, got {other:?}"),
        }
    }

    /// The core Issue #170 bug: an unknown label must fail loudly on both
    /// subcommands instead of silently becoming `Identity`.
    #[test]
    fn both_subcommands_reject_unknown_initial_strategy_labels() {
        for label in ["GreedyZoneCoverage", "reverse-2", "SWEEP-ROW", "randomx"] {
            for (side, err) in [
                ("tournament", tournament_initial_label(label).expect_err("must be rejected")),
                ("coordinator", coordinator_initial_label(label).expect_err("must be rejected")),
            ] {
                assert!(err.contains("unknown --loop-initial-strategy"), "{side} {label}: {err}");
                assert!(
                    err.contains("greedy | identity | reverse | sweep-row | sweep-col"),
                    "{side} {label}: {err}"
                );
                assert!(!err.contains("Identity"), "{side} {label} must not fall back: {err}");
            }
        }
        // `random` without a seed is rejected too (seed passthrough is
        // `random:<seed>`, mirroring offline `initial=random:seed=N`).
        let err = coordinator_initial_label("random").expect_err("seed required");
        assert!(err.contains("needs a seed"), "{err}");
        assert!(tournament_initial_label("random:abc").is_err());
    }

    /// The parsed strategy reaches `TournamentConfig` unchanged.
    #[test]
    fn tournament_config_receives_the_flag_value() {
        let cli =
            Cli::try_parse_from(["procon", "tournament", "--loop-initial-strategy", "reverse"])
                .unwrap();
        let Cmd::Tournament { loop_initial_strategy, .. } = cli.cmd else {
            panic!("expected Tournament");
        };
        let cfg = tournament::TournamentConfig {
            loop_initial_strategy,
            ..tournament::TournamentConfig::default()
        };
        assert_eq!(cfg.loop_initial_strategy, InitialStrategy::Reverse);
        assert_eq!(initial_strategy::initial_strategy_label(cfg.loop_initial_strategy), "reverse");
    }
}
