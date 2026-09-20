//! Integration test for the `procon tournament` harness.
//!
//! Spins up a real axum server on `127.0.0.1:<random>` (same shape as
//! `e2e_real_server.rs`) and drives [`run_tournament`](cli::tournament::run_tournament)
//! with two `wait` solvers so the per-solver `CoordinatorOutcome` is
//! deterministic. Asserts:
//!
//! - every solver completes all four days,
//! - both entries report identical scores,
//! - the markdown rendering contains both solver names + the header row.
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
    let day = s.day_counter.fetch_add(1, Ordering::SeqCst);
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
    (StatusCode::OK, Json(json!({"revision": revision})))
}

fn router(state: ServerState) -> Router {
    Router::new()
        .route("/setting", get(serve_setting))
        .route("/agent", axum::routing::post(serve_agent))
        .route("/", get(serve_state).post(serve_action))
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

    let scores: Vec<u32> = entries.iter().map(|e| e.score).collect();
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
        md.contains("| Rank | Solver | Days | Revisions | Score | Wall ms |"),
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
