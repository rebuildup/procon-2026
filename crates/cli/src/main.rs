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
mod bundle;
mod coordinator_loop;
mod fixtures;
mod loop_runner;
mod match_config;
mod openapi_fixture;
mod protocol_to_engine;
mod solver_adapter;
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
    },
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
        Cmd::Coordinator { server, token, solver, dry_run, state_output } => {
            cmd_coordinator(server, token, solver, dry_run, state_output).await?;
        }
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
///   `wait` → `WaitSolver`), hand both to the wired-up
///   `run_coordinator_loop`, and idle-block on the per-day driver.
///   When the real loop completes, we idle-tick forever so the container
///   stays healthy for ops to inspect.
async fn cmd_coordinator(
    server: Option<String>,
    token: Option<String>,
    solver: String,
    dry_run: bool,
    state_output: Option<PathBuf>,
) -> Result<()> {
    if dry_run {
        if server.is_none() {
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
        // dry_run + explicit server: minimal resolved-config dump.
        let base_url = server.unwrap();
        let token = token.unwrap_or_default();
        println!(
            "{}",
            json!({
                "server": base_url,
                "has_token": !token.is_empty(),
                "dry_run": true,
            })
        );
        return Ok(());
    }

    // Default behaviour: drive the real loop against the live server.
    let base_url =
        server.clone().ok_or_else(|| anyhow::anyhow!("PROCON_SERVER (or --server) must be set"))?;
    let token = token.clone().unwrap_or_default();
    let client = MatchClient::builder(base_url.clone(), token)
        .retry_policy(RetryPolicy::default())
        .request_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e: HttpClientError| anyhow::anyhow!("match client: {e}"))?;

    // Build the solver selected by `--solver`. The `value_parser` already
    // restricts the surface to {"loop", "wait"}; the `match` here is
    // exhaustive and the `_` arm is `unreachable!()` defensively.
    let solver_strategy = solver.clone();
    // The loop adapter exposes a setting slot so the coordinator loop
    // can hand the parsed engine setting (real map + agent positions)
    // back to the solver after `protocol_setting_to_engine`. Without
    // this injection the adapter would plan against a stand-in problem
    // and emit moves that fall off the real map, causing the official
    // server to reject day 0 with `revision=-1` (issue #158).
    let engine_setting_slot = match solver_strategy.as_str() {
        "loop" => {
            let adapter =
                EngineLoopAdapter::new(solver_loop::InitialStrategy::GreedyNearestNeighbor, 32);
            let slot = adapter.setting_handle();
            let solver: Box<dyn engine::solver::Solver> = Box::new(adapter);
            (solver, Some(slot))
        }
        "wait" => (Box::new(WaitSolver) as Box<dyn engine::solver::Solver>, None),
        other => {
            unreachable!("--solver value_parser already restricts to {{loop, wait}}, got {other}")
        }
    };
    let solver = engine_setting_slot.0;
    let engine_setting_slot = engine_setting_slot.1;
    tracing::info!(solver = solver.name(), "coordinator loop strategy selected");

    let loop_cfg = CoordinatorLoopConfig {
        solver_name: solver_strategy,
        // `MatchClient::base_url()` is not exposed publicly (we must not
        // modify `crates/protocol`), so we forward the URL we handed to
        // the builder above. The snapshot's `server` field mirrors this
        // value verbatim.
        server_url: Some(base_url.clone()),
        state_output_path: state_output.clone(),
        engine_setting_slot,
        ..Default::default()
    };
    let outcome = run_coordinator_loop(client, solver, loop_cfg)
        .await
        .map_err(|e| anyhow::anyhow!("coordinator loop: {e}"))?;

    tracing::warn!(
        "coordinator loop wired in #135; full E2E driving comes in #136. \
         outcome: {outcome:?}"
    );
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        ticker.tick().await;
    }
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
}
