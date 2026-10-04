//! End-to-end harness covering the `--state-output` JSON snapshot wiring.
//!
//! `--state-output <PATH>` causes `run_coordinator_loop` to atomically
//! rewrite a `MatchStateSnapshot` JSON file after every phase transition
//! (Waiting -> Running -> Completed/Rejected/Error). The frontend Live
//! page (`SolidJS` at `packages/frontend/`) polls this file via the Vite
//! dev server's static-file serving to drive its per-day timeline; the
//! CLI is intentionally agnostic about who consumes the file downstream.
//!
//! Three scenarios, all built on the same `spawn_test_server` axum
//! harness used by `e2e_real_server.rs`:
//!
//! 1. **Happy 4-day run** writes a `status == "completed"` snapshot with
//!    `revisions == [0, 1, 2, 3]` (the harness starts its revision
//!    counter at 0).
//! 2. **Rejection** (`force_revision = -1` on day 0) writes a
//!    `status == "rejected"` snapshot whose `error` field mentions the
//!    rejection.
//! 3. **No `--state-output`** writes no file at all — the snapshot
//!    writer is purely opt-in and never touches disk if `cfg.state_output_path`
//!    is `None`.
//!
//! All scenarios use `WaitSolver` so the test is fast (`endsAt = 0`
//! skips the between-day sleep) and the wire body shape is trivially
//! predictable.

use std::path::PathBuf;
use std::time::Duration;

use cli::coordinator_loop::{CoordinatorLoopConfig, CoordinatorLoopError, run_coordinator_loop};
use cli::solver_adapter::WaitSolver;
use protocol::http_client::{MatchClient, RetryPolicy};
use serde_json::Value;
use tempfile::tempdir;
use tokio::net::TcpListener;

// Pull in the `spawn_test_server` / constants from the sibling E2E
// harness so we do not duplicate the axum router. Re-importing across
// integration-test files is allowed because each `tests/*.rs` is a
// separate crate root, but doing so would re-compile axum etc.; we
// therefore redefine a minimal harness here that mirrors the same
// shape used by `e2e_real_server.rs`.

/// 4-day match plan — identical to `e2e_real_server.rs` so the
/// scenarios in this file are wire-compatible with the existing E2E
/// harness.
const DAY_SECONDS: [i32; 4] = [30, 30, 30, 30];
const DAY_STEPS: [i32; 4] = [20, 20, 20, 20];
const MAP_WIDTH: i32 = 8;
const MAP_HEIGHT: i32 = 8;

/// Sentinel for "no forced revision" — `i32::MIN` so every legal wire
/// value (..=`i32::MAX`) can be forced, including `-1` (server rejection).
const NO_FORCE_REVISION: i32 = i32::MIN;

#[derive(Clone, Default)]
struct ServerState {
    starts_at: std::sync::Arc<std::sync::atomic::AtomicI64>,
    day_counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
    revision_counter: std::sync::Arc<std::sync::atomic::AtomicI32>,
    force_revision: std::sync::Arc<std::sync::atomic::AtomicI32>,
}

impl ServerState {
    fn new(starts_at: i64) -> Self {
        let s = Self::default();
        s.starts_at.store(starts_at, std::sync::atomic::Ordering::SeqCst);
        s.force_revision.store(NO_FORCE_REVISION, std::sync::atomic::Ordering::SeqCst);
        s
    }

    fn build_setting(&self) -> Value {
        let starts_at = self.starts_at.load(std::sync::atomic::Ordering::SeqCst);
        let width_u = usize::try_from(MAP_WIDTH).expect("width fits usize");
        let height_u = usize::try_from(MAP_HEIGHT).expect("height fits usize");
        serde_json::json!({
            "startsAt": starts_at,
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

async fn serve_setting(
    axum::extract::State(s): axum::extract::State<ServerState>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    (axum::http::StatusCode::OK, axum::Json(s.build_setting()))
}

async fn serve_agent(
    axum::extract::State(_s): axum::extract::State<ServerState>,
    _body: axum::Json<Value>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({})))
}

async fn serve_state(
    axum::extract::State(s): axum::extract::State<ServerState>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    let day = s.day_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    (
        axum::http::StatusCode::OK,
        axum::Json(serde_json::json!({
            "endsAt": 0_i64,
            "day": i64::try_from(day).unwrap_or(0),
            "agents": [],
            "others": [],
            "traffics": [],
        })),
    )
}

async fn serve_action(
    axum::extract::State(s): axum::extract::State<ServerState>,
    _body: axum::Json<Vec<Vec<i32>>>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    let force = s.force_revision.load(std::sync::atomic::Ordering::SeqCst);
    let revision = if force == NO_FORCE_REVISION {
        s.revision_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    } else {
        s.force_revision.store(NO_FORCE_REVISION, std::sync::atomic::Ordering::SeqCst);
        force
    };
    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({"revision": revision})))
}

fn router(state: ServerState) -> axum::Router {
    axum::Router::new()
        .route("/setting", axum::routing::get(serve_setting))
        .route("/agent", axum::routing::post(serve_agent))
        .route("/", axum::routing::get(serve_state).post(serve_action))
        .with_state(state)
}

async fn spawn_test_server(starts_at: i64) -> (String, ServerState) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let state = ServerState::new(starts_at);
    let router = router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("axum serve");
    });
    (format!("http://{addr}"), state)
}

fn test_client(server_url: &str, token: &str) -> MatchClient {
    MatchClient::builder(server_url, token)
        .retry_policy(RetryPolicy { max_retries: 1, backoff_millis: &[10, 10] })
        .request_timeout(Duration::from_secs(2))
        .build()
        .expect("match client builds")
}

/// `CoordinatorLoopConfig` for the snapshot tests. `state_output_path`
/// is set per-test via the helper builders below; this base keeps the
/// other knobs identical to `e2e_real_server.rs::test_config` so the
/// two harnesses stay wire-compatible.
fn test_config(server_url: &str, state_output_path: Option<PathBuf>) -> CoordinatorLoopConfig {
    CoordinatorLoopConfig {
        team_id: "mvp-player".to_owned(),
        setting_poll_interval: Duration::from_millis(20),
        solver_name: "wait".to_owned(),
        state_output_path,
        server_url: Some(server_url.to_owned()),
        ..Default::default()
    }
}

async fn run_with_wait_solver(
    server_url: &str,
    state_output_path: Option<PathBuf>,
) -> Result<cli::coordinator_loop::CoordinatorOutcome, CoordinatorLoopError> {
    let solver: Box<dyn engine::solver::Solver> = Box::new(WaitSolver);
    run_coordinator_loop(
        test_client(server_url, "test-token"),
        solver,
        test_config(server_url, state_output_path),
    )
    .await
}

/// Read the snapshot file from disk and parse it as JSON. Used by the
/// post-loop assertions.
fn read_snapshot(path: &std::path::Path) -> Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read snapshot {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse snapshot JSON {}: {e}\nraw:\n{raw}", path.display()))
}

// ============================================================================
// Test cases
// ============================================================================

/// Happy path: 4-day match completes end-to-end. The final snapshot on
/// disk must report `status == "completed"`, one revision per day,
/// `daysTotal == 4`, no error, and `schemaVersion == 1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_completed_after_four_days() {
    let tmp = tempdir().expect("tempdir");
    let snap_path = tmp.path().join("state.json");
    let (url, _state) = spawn_test_server(1).await;

    let outcome =
        run_with_wait_solver(&url, Some(snap_path.clone())).await.expect("happy path succeeds");

    assert_eq!(outcome.days_completed, DAY_SECONDS.len(), "all 4 days completed");
    assert_eq!(outcome.revisions.len(), DAY_SECONDS.len(), "one revision per day");

    let snap = read_snapshot(&snap_path);
    assert_eq!(snap["schemaVersion"], 1, "schemaVersion pinned to 1");
    assert_eq!(snap["status"], "completed", "terminal status is 'completed'");
    assert_eq!(snap["solver"], "wait", "solver name mirrors --solver");
    assert_eq!(snap["server"], url, "server URL mirrors MatchClient base_url");
    assert_eq!(snap["daysTotal"], 4, "daysTotal matches setting.day_steps.len()");
    assert_eq!(snap["error"], Value::Null, "no error on happy path");
    // `day` is `null` after Completed (no current day).
    assert_eq!(snap["day"], Value::Null, "day is null after Completed");
    // `revision` is the last accepted revision (>= 0).
    let rev =
        snap["revision"].as_i64().expect("revision field present and not null after Completed");
    assert!(rev >= 0, "last revision must be non-negative, got {rev}");
    // `revisions` array mirrors `outcome.revisions`.
    let revisions: Vec<i64> = snap["revisions"]
        .as_array()
        .expect("revisions is an array")
        .iter()
        .map(|v| v.as_i64().expect("revision is i64"))
        .collect();
    assert_eq!(revisions.len(), DAY_SECONDS.len(), "revisions array has one entry per day");
    assert!(revisions.iter().all(|&r| r >= 0), "all revisions non-negative: {revisions:?}");
    // `lastUpdateUnix` is a positive unix-seconds value (the test harness
    // runs well after 1970; allow `0` only as a degenerate system-clock
    // fallback which is not exercised here).
    let last_update = snap["lastUpdateUnix"].as_i64().expect("lastUpdateUnix present");
    assert!(last_update > 0, "lastUpdateUnix should be a real timestamp, got {last_update}");

    // The atomic-rename contract: no `.json.tmp` file should remain on
    // disk after the loop completes.
    let tmp_leftover = snap_path.with_extension("json.tmp");
    assert!(
        !tmp_leftover.exists(),
        ".json.tmp leftover detected at {} — atomic rename failed",
        tmp_leftover.display()
    );
}

/// `force_revision = -1` makes the server return `{"revision": -1}` on
/// the very first `POST /`. The loop surfaces this as
/// `CoordinatorLoopError::Rejected { day: 0, revision: -1 }`, and the
/// snapshot writer must record `status == "rejected"` with the
/// rejection summary in `error`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_rejected_when_server_returns_neg1_revision() {
    let tmp = tempdir().expect("tempdir");
    let snap_path = tmp.path().join("state.json");
    let (url, state) = spawn_test_server(1).await;
    state.force_revision.store(-1, std::sync::atomic::Ordering::SeqCst);

    let err = run_with_wait_solver(&url, Some(snap_path.clone()))
        .await
        .expect_err("rejection must surface as error");

    match err {
        CoordinatorLoopError::Rejected { day, revision } => {
            assert_eq!(day, 0, "rejection on the first day");
            assert_eq!(revision, -1, "server reported revision -1");
        }
        other => panic!("expected Rejected {{ day: 0, revision: -1 }}, got {other:?}"),
    }

    let snap = read_snapshot(&snap_path);
    assert_eq!(snap["status"], "rejected", "terminal status is 'rejected'");
    assert_eq!(snap["day"], Value::from(0_u32), "day reflects the day of rejection");
    assert_eq!(snap["revision"], Value::from(-1_i32), "revision reflects the rejection value");
    let error_msg = snap["error"].as_str().expect("error is a non-null string on rejection");
    assert!(
        error_msg.contains("rejected"),
        "error message should mention 'rejected', got {error_msg:?}"
    );
    // `revisions` includes the rejected revision per the snapshot
    // semantics: the server DID return it, just negatively.
    let revisions = snap["revisions"].as_array().expect("revisions is an array");
    assert!(
        revisions.iter().any(|v| v.as_i64() == Some(-1)),
        "revisions must include the rejected value, got {revisions:?}"
    );
}

/// When `state_output_path = None` the snapshot writer is a complete
/// no-op: nothing should be written to disk under any path. We verify
/// by listing the tempdir after the loop completes — no `.json` or
/// `.json.tmp` files should appear.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_opt_out_writes_nothing_to_disk() {
    let tmp = tempdir().expect("tempdir");
    let (url, _state) = spawn_test_server(1).await;

    // Pre-condition: tempdir is empty (other than the OS-level book-keeping
    // files the tempdir crate writes, which never match `.json*`).
    for entry in std::fs::read_dir(tmp.path()).expect("read tempdir") {
        let entry = entry.expect("entry");
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        assert!(
            !name_str.ends_with(".json") && !name_str.ends_with(".json.tmp"),
            "tempdir unexpectedly contained a JSON file before the run: {name_str}"
        );
    }

    let outcome = run_with_wait_solver(&url, None)
        .await
        .expect("happy path succeeds even without snapshot output");
    assert_eq!(outcome.days_completed, DAY_SECONDS.len());

    // Post-condition: still no JSON files in the tempdir.
    for entry in std::fs::read_dir(tmp.path()).expect("read tempdir") {
        let entry = entry.expect("entry");
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        assert!(
            !name_str.ends_with(".json") && !name_str.ends_with(".json.tmp"),
            "snapshot writer leaked a file when state_output_path = None: {name_str}"
        );
    }
}

/// Phase 4 (`starts_at == 0` wait) must be bounded: the loop polls
/// `GET /setting`, which returns promptly on `403 AccessTimeError` without
/// retrying, so a server that never announces a start (cancelled match,
/// drifted API) used to hang an unattended run forever with no log line.
#[tokio::test]
async fn start_wait_gives_up_when_the_match_never_flips() {
    // `starts_at = 0` and nothing ever flips it.
    let (server_url, _state) = spawn_test_server(0).await;
    let mut cfg = test_config(&server_url, None);
    cfg.setting_poll_max_attempts = 3;
    cfg.setting_poll_interval = Duration::from_millis(5);

    let solver: Box<dyn engine::solver::Solver> = Box::new(WaitSolver);
    let err = run_coordinator_loop(test_client(&server_url, "test-token"), solver, cfg)
        .await
        .expect_err("a never-starting match must not hang the loop");

    assert!(
        matches!(
            err,
            CoordinatorLoopError::SettingPollTimeout { phase: "phase-4 start wait", attempts: 3 }
        ),
        "expected a bounded Phase 4 wait, got {err:?}"
    );
}
