//! `procon` — operator CLI.
//!
//! Subcommands:
//!   - `hello`                            : print the engine's hello payload.
//!   - `replay <path>`                    : run a JSON replay file end-to-end.
//!   - `warmup`                           : measure cold / warm / 1-day-smoke timing per solver.
//!   - `loop <days>`                      : drive the per-match solver loop (in-process transport).
//!   - `coordinator`                      : contest venue runtime (LAN match-server
//!     auto-detect + /agent POST with retry/timeout).
//!   - `match-config load <path>`         : pretty-print a match config JSON.
//!   - `match-config validate <path>`     : structurally validate a match config.
//!   - `version`                          : print package + protocol versions.
//!   - `bundle build <output.zip>`        : build a contest deliverable zip.

mod bundle;
mod coordinator;
mod fixtures;
mod loop_runner;
mod match_config;
mod warmup;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use domain::{MatchId, PlayerId};
use engine::replay::Replay;
use engine::{EngineServer, EngineServerConfig};
use serde_json::json;
use tracing_subscriber::EnvFilter;

use crate::coordinator::{AgentRequest, CliOverrides, CoordinatorConfig, post_agent_with_retry};
use crate::loop_runner::{
    DayAction, DayPlan, DaySignal, LoopConfig, LoopError, MatchSetting, Solver, Submission,
    SubmissionAck, Transport, run_match_loop,
};

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
    /// (`PROCON_SERVER` or default localhost), POSTs to /agent with retry +
    /// timeout. The actual solver loop body is wired up by #45.
    Coordinator {
        /// Override `PROCON_SERVER` (host:port or http(s)://host:port).
        #[arg(long, env = "PROCON_SERVER")]
        server: Option<String>,

        /// Override `PROCON_AGENT_URL`.
        #[arg(long, env = "PROCON_AGENT_URL")]
        agent_url: Option<String>,

        /// Override `PROCON_SETTING_URL`.
        #[arg(long, env = "PROCON_SETTING_URL")]
        setting_url: Option<String>,

        /// Override `PROCON_TOKEN`.
        #[arg(long, env = "PROCON_TOKEN")]
        token: Option<String>,

        /// agent id to advertise to the match server.
        #[arg(long, default_value = "agent-0")]
        agent_id: String,

        /// Only resolve config + print it. Do not actually POST /agent.
        #[arg(long)]
        dry_run: bool,

        /// Send a single /agent POST then exit. Used by smoke tests.
        #[arg(long)]
        once: bool,
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
        Cmd::Coordinator { server, agent_url, setting_url, token, agent_id, dry_run, once } => {
            run_coordinator(server, agent_url, setting_url, token, agent_id, dry_run, once).await?;
        }
        Cmd::MatchConfig(sub) => match sub {
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
        },
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

/// Drive the `coordinator` subcommand: probe the venue server, optionally
/// send a single agent POST, otherwise idle-block to keep the container
/// healthy for ops inspection (real solver loop arrives with #45).
#[allow(clippy::too_many_lines)]
async fn run_coordinator(
    server: Option<String>,
    agent_url: Option<String>,
    setting_url: Option<String>,
    token: Option<String>,
    agent_id: String,
    dry_run: bool,
    once: bool,
) -> Result<()> {
    let overrides = CliOverrides { server, agent_url, setting_url, token };
    let cfg = CoordinatorConfig::from_env(&overrides)?;
    println!("{}", serde_json::to_string_pretty(&cfg.describe())?);

    if dry_run {
        return Ok(());
    }

    if once {
        let body = AgentRequest { agent_id, day: 0, turn: 0, payload: serde_json::Value::Null };
        let resp = post_agent_with_retry(&cfg, &body)?;
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    // Default behaviour: stay alive, waiting for the real solver loop
    // that #45 will provide. Until then we just block here so the
    // container stays in a healthy state for ops to inspect.
    tracing::warn!(
        "coordinator loop body not yet implemented (see #45); \
         idle-blocking. Use --once to send a single probe POST."
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
}
