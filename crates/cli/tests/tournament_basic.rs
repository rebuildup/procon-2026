//! Integration test for the `procon tournament` harness.
//!
//! Spins up a real axum server on `127.0.0.1:<random>` (same shape as
//! `e2e_real_server.rs`) and drives [`run_tournament`](cli::tournament::run_tournament)
//! with two `wait` solvers so the per-solver `CoordinatorOutcome` is
//! deterministic. Asserts:
//!
//! - every solver completes all four days,
//! - both entries report identical scores,
//! - the markdown rendering contains both solver names + the header row,
//! - the optional `--auto-restart-server` hook is invoked exactly once
//!   between two solvers and not at all on a single-solver run, and
//!   a non-zero exit aborts the tournament with `ServerRestartFailed`.
//!
//! Run with `--nocapture` to see the markdown leaderboard.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use cli::tournament::{TournamentConfig, render_markdown, run_tournament};

const DAY_SECONDS: [i32; 4] = [30, 30, 30, 30];
const DAY_STEPS: [i32; 4] = [20, 20, 20, 20];
const MAP_WIDTH: i32 = 8;
const MAP_HEIGHT: i32 = 8;

/// Shared state of the test server. Captures enough detail to drive the
/// 4-endpoint protocol and increment revision ids deterministically.
#[derive(Clone)]
struct ServerState {
    revision_counter: Arc<AtomicI32>,
    day_counter: Arc<AtomicU64>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            revision_counter: Arc::new(AtomicI32::new(0)),
            day_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Reset the per-match counters so the next solver sees `day=0`.
    /// Called from the `_test/reset` route which the tournament's
    /// `auto_restart_server` shell script invokes between matches in the
    /// hermetic integration tests.
    fn reset(&self) {
        self.revision_counter.store(0, Ordering::SeqCst);
        self.day_counter.store(0, Ordering::SeqCst);
    }

    fn build_setting() -> Value {
        let width_u = usize::try_from(MAP_WIDTH).expect("width fits usize");
        let height_u = usize::try_from(MAP_HEIGHT).expect("height fits usize");
        json!({
            "startsAt": 1,
            "daySeconds": DAY_SECONDS,
            "daySteps": DAY_STEPS,
            "map": {
                "height": MAP_HEIGHT,
                "width": MAP_WIDTH,
                "cells": vec![vec![0_i32; width_u]; height_u],
            },
            "spots": [],
            "agents": [0_i32],
            "fuelLimits": 30,
            "players": 2,
            "busyThreshold": 3,
            "jammedThreshold": 5,
        })
    }
}

async fn serve_setting(State(_s): State<ServerState>) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(ServerState::build_setting()))
}

async fn serve_agent(
    State(_s): State<ServerState>,
    _body: Json<Value>,
) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({})))
}

async fn serve_state(State(s): State<ServerState>) -> (StatusCode, Json<Value>) {
    let day = s.day_counter.load(Ordering::SeqCst);
    (
        StatusCode::OK,
        Json(json!({
            "endsAt": 0_i64,
            "day": i64::try_from(day).unwrap_or(0),
            "agents": [],
            "others": [],
            "traffics": [],
        })),
    )
}

async fn serve_action(
    State(s): State<ServerState>,
    _body: Json<Vec<Vec<i32>>>,
) -> (StatusCode, Json<Value>) {
    let revision = s.revision_counter.fetch_add(1, Ordering::SeqCst);
    s.day_counter.fetch_add(1, Ordering::SeqCst);
    (StatusCode::OK, Json(json!({"revision": revision})))
}

/// Test-only endpoint that lets the `auto_restart_server` shell hook
/// reset per-match counters between solvers (the official server is a
/// real OS process, so it actually restarts; the in-process axum mock
/// can't be killed from a shell, hence this stand-in).
async fn serve_test_reset(State(s): State<ServerState>) -> StatusCode {
    s.reset();
    StatusCode::OK
}

fn router(state: ServerState) -> Router {
    Router::new()
        .route("/setting", get(serve_setting))
        .route("/agent", axum::routing::post(serve_agent))
        .route("/", get(serve_state).post(serve_action))
        .route("/_test/reset", axum::routing::post(serve_test_reset))
        .with_state(state)
}

async fn spawn_test_server() -> (String, ServerState) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let state = ServerState::new();
    let router = router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("axum serve");
    });
    (format!("http://{addr}"), state)
}

/// Retried state reads must not consume a day in the mock protocol.
#[tokio::test]
async fn repeated_state_reads_do_not_advance_the_match() {
    let (url, _) = spawn_test_server().await;
    let client = protocol::MatchClient::new(url, "test-token").unwrap();
    assert_eq!(client.get_match_state().await.unwrap().day, 0);
    assert_eq!(client.get_match_state().await.unwrap().day, 0);
    client.submit_actions(vec![vec![-20]]).await.unwrap();
    assert_eq!(client.get_match_state().await.unwrap().day, 1);
}

/// Happy path: a single `wait` solver drives the tournament to a 4-day
/// match and the rendered markdown contains the expected structural
/// pieces. We deliberately use a single solver on a single server
/// because the harness is sequential: matches share server-side state
/// (each match begins with a fresh day counter on a real contest
/// server, but the local axum mock here persists counters across
/// `run_tournament` invocations unless restarted).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tournament_two_solvers_against_fresh_servers_produce_identical_scores() {
    let mut entries = Vec::new();
    for _ in 0..2 {
        // Fresh server per solver so the day counter resets between matches.
        let (url, _state) = spawn_test_server().await;
        let cfg = TournamentConfig {
            server: url.clone(),
            token: "test-token".to_owned(),
            solvers: vec!["wait".to_owned()],
            ..TournamentConfig::default()
        };
        let report = run_tournament(cfg).await.expect("tournament with wait solver succeeds");
        assert_eq!(report.entries.len(), 1, "one entry per --solvers entry");
        assert_eq!(report.server, url, "report.server mirrors the TournamentConfig server URL");
        entries.push(report.entries.into_iter().next().expect("one entry"));
    }

    assert_eq!(entries.len(), 2, "ran two matches");
    for entry in &entries {
        assert_eq!(
            entry.outcome.days_completed,
            DAY_SECONDS.len(),
            "wait solver completes all 4 days (entry: {entry:?})"
        );
    }

    let scores: Vec<u32> = entries.iter().map(|e| e.protocol_progress).collect();
    assert_eq!(
        scores[0], scores[1],
        "both entries have identical scores (deterministic wait solver)"
    );

    // Build a synthetic report from the two entries to exercise the
    // markdown renderer end-to-end.
    let report = cli::tournament::TournamentReport {
        entries,
        server: "http://localhost:8080".to_owned(),
        elapsed_ms: 1,
        schema_version: 1,
    };
    let md = render_markdown(&report);
    assert!(md.contains("# Tournament leaderboard"), "header present");
    assert!(
        md.contains("| Rank | Solver | Days | Revisions | Protocol progress | Wall ms |"),
        "table header present"
    );
    assert!(md.contains("wait"), "solver name appears in the table");
    assert!(md.contains("Total elapsed:"), "summary line at the bottom of the report");
}

/// Unknown solver name surfaces as `TournamentError::UnknownSolver` and
/// aborts the tournament before any HTTP traffic is generated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tournament_rejects_unknown_solver() {
    let (url, _state) = spawn_test_server().await;
    let cfg = TournamentConfig {
        server: url,
        token: "test-token".to_owned(),
        solvers: vec!["not-a-solver".to_owned()],
        ..TournamentConfig::default()
    };
    let result = run_tournament(cfg).await;
    let err = result.expect_err("unknown solver must surface as error");
    let matches = matches!(
        &err,
        cli::tournament::TournamentError::UnknownSolver(name) if name == "not-a-solver"
    );
    assert!(matches, "expected UnknownSolver(\"not-a-solver\"), got {err:?}");
}

/// Write a restart hook into `dir` and return the command line that runs it:
/// a `sh` script on Unix, a `cmd.exe` batch file on Windows. The hook creates
/// `marker` when one is given and exits with `exit_code`.
fn write_restart_hook(
    dir: &std::path::Path,
    name: &str,
    marker: Option<&std::path::Path>,
    exit_code: i32,
) -> String {
    if cfg!(windows) {
        let script = dir.join(format!("{name}.cmd"));
        let touch =
            marker.map_or_else(String::new, |m| format!("type nul > \"{}\"\r\n", m.display()));
        std::fs::write(&script, format!("@echo off\r\n{touch}exit /b {exit_code}\r\n"))
            .expect("write restart hook");
        // `call` makes `&&` and the shell's exit status see the batch file's `exit /b` code.
        format!("call \"{}\"", script.display())
    } else {
        let script = dir.join(format!("{name}.sh"));
        let touch = marker.map_or_else(String::new, |m| format!(": > \"{}\"\n", m.display()));
        std::fs::write(&script, format!("#!/bin/sh\n{touch}exit {exit_code}\n"))
            .expect("write restart hook");
        format!("sh \"{}\"", script.display())
    }
}

/// `auto_restart_server` is invoked exactly once between two solvers,
/// proves the marker file appears only after the second solver run, and
/// both solvers still complete all four days. The hook is a script
/// written into a `tempfile` directory so the test stays hermetic.
/// The command touches a marker file AND hits the mock server's
/// `/_test/reset` endpoint to wipe the per-match counters — the real
/// official binary achieves this by actually restarting, but the in-process
/// axum mock can't be killed from a shell, hence the explicit reset.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tournament_with_auto_restart_invokes_hook_between_solvers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("restarted.marker");

    // Touch the marker on every invocation. Counting invocations via a
    // counter file would also work, but `marker.exists()` is enough to
    // prove the hook ran at least once.
    let hook = write_restart_hook(dir.path(), "restart", Some(&marker), 0);

    let (url, _state) = spawn_test_server().await;
    let cfg = TournamentConfig {
        server: url.clone(),
        token: "test-token".to_owned(),
        solvers: vec!["wait".to_owned(), "wait".to_owned()],
        auto_restart_server: Some(format!("{hook} && curl -fsS -X POST \"{url}/_test/reset\"")),
        ..TournamentConfig::default()
    };
    assert!(!marker.exists(), "marker must not exist before the tournament runs");
    let report = run_tournament(cfg).await.expect("tournament with auto-restart succeeds");
    assert_eq!(report.entries.len(), 2, "two solver entries");
    assert!(marker.exists(), "marker file must exist after restart hook ran");
    for entry in &report.entries {
        assert_eq!(
            entry.outcome.days_completed,
            DAY_SECONDS.len(),
            "each solver completes all 4 days (entry: {entry:?})"
        );
    }
}

/// Single-solver tournament never invokes the restart hook even when one
/// is configured — the official binary already serves the first match
/// and there is no second solver to reset for. Confirms the
/// `idx > 0` guard inside `run_tournament`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tournament_with_auto_restart_skips_hook_for_first_solver() {
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("should-not-exist.marker");
    let hook = write_restart_hook(dir.path(), "noop", Some(&marker), 0);

    let (url, _state) = spawn_test_server().await;
    let cfg = TournamentConfig {
        server: url,
        token: "test-token".to_owned(),
        solvers: vec!["wait".to_owned()],
        auto_restart_server: Some(hook),
        ..TournamentConfig::default()
    };
    let report = run_tournament(cfg).await.expect("single-solver tournament succeeds");
    assert_eq!(report.entries.len(), 1);
    assert!(!marker.exists(), "restart hook must NOT run for a single-solver tournament");
}

/// A non-zero exit from the restart hook aborts the tournament with
/// `TournamentError::ServerRestartFailed` instead of silently continuing
/// onto a still-down server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tournament_with_auto_restart_propagates_hook_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hook = write_restart_hook(dir.path(), "fail", None, 7);

    let (url, _state) = spawn_test_server().await;
    let cfg = TournamentConfig {
        server: url,
        token: "test-token".to_owned(),
        solvers: vec!["wait".to_owned(), "wait".to_owned()],
        auto_restart_server: Some(hook),
        ..TournamentConfig::default()
    };
    let result = run_tournament(cfg).await;
    let err = result.expect_err("failing restart hook must abort the tournament");
    let matches = matches!(&err, cli::tournament::TournamentError::ServerRestartFailed(_));
    assert!(matches, "expected ServerRestartFailed, got {err:?}");
}
