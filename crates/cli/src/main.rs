//! `procon` — operator CLI.
//!
//! Subcommands:
//!   - `hello`                     : print the engine's hello payload.
//!   - `replay <path>`             : run a JSON replay file end-to-end.
//!   - `warmup`                    : measure cold / warm / 1-day-smoke timing per solver.
//!   - `coordinator`               : drive the per-match solver loop (in-process transport).
//!   - `version`                   : print package + protocol versions.
//!   - `bundle build <output.zip>` : build a contest deliverable zip.

mod bundle;
mod fixtures;
mod loop_runner;
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
    Coordinator {
        /// Number of days the synthetic match will run for.
        #[arg(long, default_value_t = 3)]
        days: u32,
        /// Player id to register for the synthetic match.
        #[arg(long, default_value = "player-alpha")]
        player: String,
    },
    /// Print package and protocol versions.
    Version,
    /// Build, inspect, and verify contest deliverable bundles.
    Bundle {
        #[command(subcommand)]
        action: BundleCmd,
    },
}

#[derive(Debug, Subcommand)]
enum BundleCmd {
    /// Build a contest deliverable zip at the given output path.
    Build {
        /// Destination path for the produced zip file.
        output: PathBuf,
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
        Cmd::Coordinator { days, player } => {
            run_coordinator(days, &player).await?;
        }
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

/// Drive the per-match solver loop with an in-process transport.
///
/// The CLI does not yet have an HTTP transport; this wires the loop runner to
/// a deterministic in-memory transport so the operator path can be exercised
/// locally. Once `crates/cli/src/coordinator.rs` lands, this stub will be
/// replaced with an HTTP-backed transport.
async fn run_coordinator(days: u32, player: &str) -> Result<()> {
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

/// Placeholder solver that emits a single no-op action per day. The real
/// per-day solver will be plugged in once the engine-level trait lands.
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

/// In-process transport that always acks and advances the day. Real transport
/// is wired in once the network layer lands.
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
}
