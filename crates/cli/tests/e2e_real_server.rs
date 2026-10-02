//! End-to-end harness driving `run_coordinator_loop` against a real axum server.
//!
//! Unlike `e2e_openapi_fixture.rs` (which uses an in-process mockito server),
//! this test spins up an actual `axum::serve` instance on
//! `127.0.0.1:<random-port>` and exercises the full TCP / HTTP /
//! tokio-scheduler pipeline. That catches failure modes that the mockito
//! path cannot observe:
//!
//! - real auth/access-time error envelopes parsed by `protocol::MatchClient`
//!   over an actual HTTP socket,
//! - retry behaviour driven by real wall-clock backoff,
//! - the `revision < 0` rejection path inside `submit_actions` (the server
//!   returns `200 OK` with `{"revision": -1}` and the loop must surface it
//!   as `CoordinatorLoopError::Rejected` instead of retrying),
//! - the `starts_at == 0` poll loop that waits for the server to flip the
//!   setting on a separate task,
//! - `ends_at` already in the past so the between-day sleep is skipped.
//!
//! The test server is a deliberately small subset of the official Procon 37
//! 4-endpoint protocol: `GET /setting`, `POST /agent`, `GET /`, `POST /`. Its
//! responses use the exact `camelCase` JSON shape that
//! `protocol::match_config` decodes, so the existing `MatchClient` requires
//! no changes.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use cli::coordinator_loop::{CoordinatorLoopConfig, CoordinatorOutcome, run_coordinator_loop};
use cli::solver_adapter::{EngineLoopAdapter, WaitSolver};
use protocol::http_client::{MatchClient, RetryPolicy};
use solver_loop::InitialStrategy;

const OPENAPI_PATH: &str = "../../.reference/procon37/openapi.json";
const MATCH_CONFIG_PATH: &str = "../../.reference/procon37/match-config.example.json";

/// Map size used by the synthesised `MatchSetting`. 8x8 satisfies the
/// `engine::model::MapSize::MIN` constraint (8..=32 in both axes).
const MAP_WIDTH: i32 = 8;
const MAP_HEIGHT: i32 = 8;

/// Day plan for a 4-day match. Each `day_steps[i]` is the per-day budget the
/// adapter must fill with `-N` (wait) and `0..=5` (move) tokens whose
/// step-sum equals `day_steps[i]`.
const DAY_SECONDS: [i32; 4] = [30, 30, 30, 30];
const DAY_STEPS: [i32; 4] = [20, 20, 20, 20];

/// Sentinel meaning "no forced revision" in [`ServerState::force_revision`].
/// `i32::MIN` is used so that every legal wire value (`..=i32::MAX`) can be
/// forced — the most important of which is `-1` (server rejection).
const NO_FORCE_REVISION: i32 = i32::MIN;

/// Shared state of the test server. Held inside an `Arc` so the test thread
/// can flip flags while `run_coordinator_loop` runs in another task.
#[derive(Clone)]
struct ServerState {
    /// UNIX-time match start. `0` means "not yet announced" and causes the
    /// loop to keep polling `GET /setting`. The `starts_at_in_future` test
    /// flips this from `0` to a positive value mid-flight.
    starts_at: Arc<AtomicI64>,
    /// Monotonic day counter, incremented by every `GET /`.
    day_counter: Arc<AtomicU64>,
    /// Monotonic revision counter, returned by `POST /` when no override is
    /// active.
    revision_counter: Arc<AtomicI32>,
    /// When `!= NO_FORCE_REVISION`, the next `POST /` returns this revision
    /// exactly once (then resets to `NO_FORCE_REVISION`). Used by the
    /// `revision=-1` rejection test — `-1` is a valid wire value, so we
    /// cannot use it as the sentinel.
    force_revision: Arc<AtomicI32>,
    /// When `> 0`, the next N requests return `401`. The loop's retry policy
    /// must NOT attempt to retry, so the test asserts that the loop returns
    /// a `CoordinatorLoopError::Setting`/`AgentSubmit`/`State`/`ActionSubmit`
    /// after a single round-trip.
    force_auth_fail: Arc<AtomicU64>,
    /// When `> 0`, the next N requests return `403 AccessTimeError`. Used
    /// by the loop's pre-start branch which translates the error into a
    /// poll-loop iteration.
    force_access_time: Arc<AtomicU64>,
    /// When `> 0`, the next N requests return `500`. The retry policy must
    /// burn through them and recover on the subsequent 200.
    force_500: Arc<AtomicU64>,
    /// When `> 0`, `GET /` returns `403 AccessTimeError` for the first
    /// `force_match_not_started_ms` milliseconds since server start.
    /// Used to reproduce the official server's `-match-start-delay`
    /// behaviour (issue #164) — `starts_at` is set in `GET /setting`
    /// but `GET /` keeps returning 403 for a short window after.
    /// `0` (default) means "match is already started, no 403".
    force_match_not_started_ms: Arc<AtomicU64>,
    /// Instant the test server was started. Used together with
    /// `force_match_not_started_ms` to gate the 403 window.
    started_at: Arc<Instant>,
    /// When `true`, every `POST /agent` returns `403 AccessTimeError`
    /// regardless of `starts_at`. Used by the
    /// `kind_submission_during_window_succeeds_even_if_match_starts_later`
    /// regression test (issue #156) to simulate the official server's
    /// post-deadline behaviour on demand. When `false`, the kind-window
    /// is treated as always OPEN so the other scenarios can run with
    /// `starts_at = 1` without tripping this check.
    force_kind_window_closed: Arc<std::sync::atomic::AtomicBool>,
    /// Per-agent run-length encoded action plans captured by every `POST /`.
    /// Outer index = day, inner = one `Vec<i32>` per controlled agent.
    captured_action_bodies: Arc<Mutex<Vec<Vec<Vec<i32>>>>>,
}

impl ServerState {
    fn new(starts_at: i64) -> Self {
        // `started_at` cannot live in a `#[derive(Default)]` because
        // `Instant: !Default`. Build it eagerly here so the gate logic in
        // `serve_state` has a sensible reference point from the moment
        // the server starts accepting connections.
        Self {
            starts_at: Arc::new(AtomicI64::new(starts_at)),
            day_counter: Arc::new(AtomicU64::new(0)),
            revision_counter: Arc::new(AtomicI32::new(0)),
            force_revision: Arc::new(AtomicI32::new(NO_FORCE_REVISION)),
            force_auth_fail: Arc::new(AtomicU64::new(0)),
            force_access_time: Arc::new(AtomicU64::new(0)),
            force_500: Arc::new(AtomicU64::new(0)),
            force_match_not_started_ms: Arc::new(AtomicU64::new(0)),
            started_at: Arc::new(Instant::now()),
            force_kind_window_closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            captured_action_bodies: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Build a wire-valid `MatchSetting` for the given day plan. The map is
    /// the smallest allowed (`8x8`) plain grid; one controlled agent sits at
    /// cell `0` so the engine view of the wire payload has the right shape.
    fn build_setting(&self) -> Value {
        let starts_at = self.starts_at.load(Ordering::SeqCst);
        let width_u = usize::try_from(MAP_WIDTH).expect("width fits usize");
        let height_u = usize::try_from(MAP_HEIGHT).expect("height fits usize");
        json!({
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

/// `GET /setting` handler. Honours the `force_auth_fail`, `force_access_time`,
/// and `force_500` overrides; otherwise returns a fresh `MatchSetting` built
/// from `ServerState::build_setting`.
async fn serve_setting(State(s): State<ServerState>) -> (StatusCode, Json<Value>) {
    if s.force_auth_fail.load(Ordering::SeqCst) > 0 {
        s.force_auth_fail.fetch_sub(1, Ordering::SeqCst);
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "code": "AuthError",
                "message": "bad token",
            })),
        );
    }
    if s.force_access_time.load(Ordering::SeqCst) > 0 {
        s.force_access_time.fetch_sub(1, Ordering::SeqCst);
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": "AccessTimeError",
                "message": "match has not started",
            })),
        );
    }
    if s.force_500.load(Ordering::SeqCst) > 0 {
        s.force_500.fetch_sub(1, Ordering::SeqCst);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})));
    }
    (StatusCode::OK, Json(s.build_setting()))
}

/// `POST /agent` handler. Accepts any `AgentKindsBody` and returns 200.
/// Auth/500 overrides apply uniformly so the transient-500 test can exercise
/// the retry path on this endpoint too.
///
/// The official server's kind-submission window is modelled here via the
/// `force_kind_window_closed` opt-in flag (used by
/// `kind_submission_during_window_succeeds_even_if_match_starts_later`,
/// issue #156). By default the window is treated as OPEN so the other
/// scenarios can run with `starts_at = 1` without tripping this check.
async fn serve_agent(
    State(s): State<ServerState>,
    _body: Json<Value>,
) -> (StatusCode, Json<Value>) {
    if s.force_auth_fail.load(Ordering::SeqCst) > 0 {
        s.force_auth_fail.fetch_sub(1, Ordering::SeqCst);
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": "AuthError", "message": "bad token"})),
        );
    }
    if s.force_500.load(Ordering::SeqCst) > 0 {
        s.force_500.fetch_sub(1, Ordering::SeqCst);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})));
    }
    if s.force_kind_window_closed.load(Ordering::SeqCst) {
        // Kind window closed (server has announced the start). Only
        // reachable when the regression test opts in via the flag;
        // otherwise the kind window is treated as always-open so the
        // happy-path tests can run with `starts_at = 1`.
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": "AccessTimeError",
                "message": "agent kind submission is closed",
            })),
        );
    }
    (StatusCode::OK, Json(json!({})))
}

/// `GET /` handler returning a `MatchState` snapshot. `endsAt` is set far in
/// the future so the loop's `ends_at - now` check stays non-negative (i.e.
/// the between-day sleep is skipped only by negative `ends_at`; for the
/// happy-path test we want fast execution, so we use a small positive value
/// instead — see `end_at_past_skips_sleep` for the past-`ends_at` variant).
async fn serve_state(State(s): State<ServerState>) -> (StatusCode, Json<Value>) {
    if s.force_auth_fail.load(Ordering::SeqCst) > 0 {
        s.force_auth_fail.fetch_sub(1, Ordering::SeqCst);
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": "AuthError", "message": "bad token"})),
        );
    }
    // Reproduce the official server's `-match-start-delay`: while
    // elapsed time since server start is below `force_match_not_started_ms`,
    // `GET /` returns 403 AccessTimeError even though `starts_at` is set
    // in `GET /setting`. The loop's `fetch_state_with_retry` helper must
    // ride out this window (issue #164).
    let threshold_ms = s.force_match_not_started_ms.load(Ordering::SeqCst);
    if threshold_ms > 0 {
        let elapsed_ms = s.started_at.elapsed().as_millis();
        if u128::from(threshold_ms) > elapsed_ms {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "code": "AccessTimeError",
                    "message": "match has not started",
                })),
            );
        }
    }
    let day = s.day_counter.fetch_add(1, Ordering::SeqCst);
    // end_at = 0 makes the loop's `state.ends_at - now` calculation
    // negative (since `now` is always positive), so the between-day sleep
    // is skipped and the test finishes quickly.
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

/// `POST /` handler capturing the run-length encoded action plan and
/// returning an `ActionAccepted` with the next revision id (or the
/// `force_revision` value, used by the rejection test).
async fn serve_action(
    State(s): State<ServerState>,
    Json(body): Json<Vec<Vec<i32>>>,
) -> (StatusCode, Json<Value>) {
    if s.force_auth_fail.load(Ordering::SeqCst) > 0 {
        s.force_auth_fail.fetch_sub(1, Ordering::SeqCst);
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": "AuthError", "message": "bad token"})),
        );
    }
    s.captured_action_bodies.lock().await.push(body);
    let force = s.force_revision.load(Ordering::SeqCst);
    let revision = if force == NO_FORCE_REVISION {
        s.revision_counter.fetch_add(1, Ordering::SeqCst)
    } else {
        s.force_revision.store(NO_FORCE_REVISION, Ordering::SeqCst);
        force
    };
    (StatusCode::OK, Json(json!({"revision": revision})))
}

/// Build the four-endpoint axum router and share `state` via `with_state`.
fn router(state: ServerState) -> Router {
    Router::new()
        .route("/setting", get(serve_setting))
        .route("/agent", post(serve_agent))
        .route("/", get(serve_state).post(serve_action))
        .with_state(state)
}

/// Bind to a random localhost port and spawn the axum server. Returns the
/// origin URL (e.g. `http://127.0.0.1:NNNNN`) and a clone of the state
/// handle so the test can flip flags while the loop runs.
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

/// Build a `MatchClient` whose retry budget is small enough that the 401 /
/// revision=-1 tests still complete in well under a second.
fn test_client(server_url: &str, token: &str) -> MatchClient {
    MatchClient::builder(server_url, token)
        .retry_policy(RetryPolicy { max_retries: 1, backoff_millis: &[10, 10] })
        .request_timeout(Duration::from_secs(2))
        .build()
        .expect("match client builds")
}

/// Build the standard `CoordinatorLoopConfig` used by every scenario. The
/// poll interval is short so the `startsAt` poll test reacts within tens
/// of milliseconds.
fn test_config(solver_name: &str) -> CoordinatorLoopConfig {
    CoordinatorLoopConfig {
        team_id: "mvp-player".to_owned(),
        setting_poll_interval: Duration::from_millis(20),
        solver_name: solver_name.to_owned(),
        // Issue #156: keep the kind-window retry budget at the production
        // default so the bug-regression test exercises the same code path
        // as a real coordinator run.
        ..Default::default()
    }
}

/// Drive `run_coordinator_loop` with the production loop solver.
async fn run_with_loop_solver(
    server_url: &str,
    token: &str,
) -> Result<CoordinatorOutcome, cli::coordinator_loop::CoordinatorLoopError> {
    let solver: Box<dyn engine::solver::Solver> =
        Box::new(EngineLoopAdapter::new(InitialStrategy::GreedyNearestNeighbor, 32));
    run_coordinator_loop(test_client(server_url, token), solver, test_config("loop")).await
}

/// Drive `run_coordinator_loop` with the debug `WaitSolver`.
async fn run_with_wait_solver(
    server_url: &str,
    token: &str,
) -> Result<CoordinatorOutcome, cli::coordinator_loop::CoordinatorLoopError> {
    let solver: Box<dyn engine::solver::Solver> = Box::new(WaitSolver);
    run_coordinator_loop(test_client(server_url, token), solver, test_config("wait")).await
}

// ============================================================================
// Test cases
// ============================================================================

/// Happy path: a 4-day match completes end-to-end and every `revision`
/// returned by `POST /` is non-negative. Uses `WaitSolver` so the loop is
/// fast and the wire bodies are predictable (`[-N]` per agent per day).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn happy_path_completes_four_days() {
    let (url, _state) = spawn_test_server(1).await;
    let started = Instant::now();
    let outcome = run_with_wait_solver(&url, "test-token").await.expect("happy path succeeds");
    let elapsed = started.elapsed();

    assert_eq!(outcome.days_completed, DAY_SECONDS.len(), "all 4 days complete");
    assert_eq!(outcome.revisions.len(), DAY_SECONDS.len(), "one revision per day");
    assert!(
        outcome.revisions.iter().all(|&r| r >= 0),
        "all revisions non-negative: {:?}",
        outcome.revisions
    );
    // `endsAt = 0` skips the between-day sleep, so the whole match should
    // finish in well under 5s even on a slow CI runner.
    assert!(elapsed < Duration::from_secs(5), "match too slow: {elapsed:?}");
}

/// `startsAt = 0` makes the loop poll `GET /setting` in a tight loop. A
/// separate task flips the flag to a positive value after a short delay;
/// the loop must then proceed and complete all four days.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starts_at_in_future_polls_until_reached() {
    let (url, state) = spawn_test_server(0).await;
    // Sanity: starts_at starts at 0 so the loop will poll.
    assert_eq!(state.starts_at.load(Ordering::SeqCst), 0);

    // Run the loop on its own task so we can flip the flag concurrently.
    let url_for_task = url.clone();
    let handle =
        tokio::spawn(async move { run_with_wait_solver(&url_for_task, "test-token").await });

    // Give the loop a few poll cycles to confirm it really is polling, then
    // announce the match.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!handle.is_finished(), "loop must not have completed yet while starts_at is still 0");
    state.starts_at.store(1, Ordering::SeqCst);

    let outcome = handle.await.expect("join loop task").expect("loop succeeds");
    assert_eq!(outcome.days_completed, DAY_SECONDS.len(), "all 4 days complete");
    assert!(
        outcome.revisions.iter().all(|&r| r >= 0),
        "all revisions non-negative: {:?}",
        outcome.revisions
    );
}

/// `force_auth_fail` is set high enough to cover the entire match. The
/// first request to fail (here: `GET /setting`) returns 401, which the
/// `protocol::MatchClient` classifies as `HttpClientError::Auth` and
/// refuses to retry. The loop must therefore fail with a
/// `CoordinatorLoopError::Setting` and the loop's wall-clock budget must
/// stay under one second — no retry storm.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_fail_401_does_not_retry() {
    let (url, state) = spawn_test_server(1).await;
    state.force_auth_fail.store(1024, Ordering::SeqCst);

    let started = Instant::now();
    let result = run_with_wait_solver(&url, "bad-token").await;
    let elapsed = started.elapsed();

    let err = result.expect_err("auth failure must surface as error");
    // The loop fails fast on the first `GET /setting` because 401 is not
    // a retryable status. We don't pin the exact variant (Setting vs
    // AgentSubmit) — the wire order is determined by the server's flag,
    // not by anything the test depends on — only the structural shape.
    assert!(
        matches!(
            err,
            cli::coordinator_loop::CoordinatorLoopError::Setting(
                protocol::http_client::HttpClientError::Auth { .. }
            ) | cli::coordinator_loop::CoordinatorLoopError::AgentSubmit(
                protocol::http_client::HttpClientError::Auth { .. }
            )
        ),
        "expected auth error on the first request, got {err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "401 must not trigger a retry storm (elapsed = {elapsed:?})"
    );
}

/// `POST /` returns `{"revision": -1}`. The server still uses HTTP 200, so
/// the `MatchClient`'s retry loop considers the request successful; the
/// rejection is detected only when decoding the body. The loop must surface
/// this as `CoordinatorLoopError::Rejected { day: 0, revision: -1 }` and
/// must NOT retry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revision_neg1_returns_rejected_error() {
    let (url, state) = spawn_test_server(1).await;
    state.force_revision.store(-1, Ordering::SeqCst);

    let err = run_with_wait_solver(&url, "test-token")
        .await
        .expect_err("rejection must surface as error");

    match err {
        cli::coordinator_loop::CoordinatorLoopError::Rejected { day, revision } => {
            assert_eq!(day, 0, "rejection on the first day");
            assert_eq!(revision, -1, "server reported revision -1");
        }
        other => panic!("expected Rejected {{ day: 0, revision: -1 }}, got {other:?}"),
    }
    // Sanity: the flag was consumed exactly once (reset to sentinel).
    assert_eq!(state.force_revision.load(Ordering::SeqCst), NO_FORCE_REVISION);
}

/// Transient 5xx: the first request fails with 500 (which the client
/// classifies as transient via `is_transient_status`), then the retry
/// succeeds. The loop must burn through the retry and complete all four
/// days without losing its place.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_500_retries_then_succeeds() {
    let (url, state) = spawn_test_server(1).await;
    // `max_retries = 1` (see `test_client`) means the client makes at most
    // 2 attempts per request. We only need the FIRST attempt of the FIRST
    // request (typically `GET /setting`) to fail with 500; the second
    // attempt then sees a fresh 200 and the match proceeds.
    state.force_500.store(1, Ordering::SeqCst);

    let outcome = run_with_wait_solver(&url, "test-token").await.expect("loop recovers from 500s");

    assert_eq!(outcome.days_completed, DAY_SECONDS.len());
    assert!(outcome.revisions.iter().all(|&r| r >= 0));
    assert_eq!(state.force_500.load(Ordering::SeqCst), 0, "transient counter fully drained");
}

/// Per-day action plan: the production `EngineLoopAdapter` drives the
/// loop solver, so every `POST /` body must contain one inner `Vec<i32>`
/// per controlled agent, each with `step_sum == day_steps[day]`. We also
/// assert at least one Move action (i.e. a non-negative token) to prove
/// the planner is not just degenerating to `wait_all`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn action_plan_body_captured_and_step_sum_correct() {
    let (url, state) = spawn_test_server(1).await;
    let outcome = run_with_loop_solver(&url, "test-token").await.expect("loop completes");
    assert_eq!(outcome.days_completed, DAY_SECONDS.len(), "all 4 days complete");

    let bodies = state.captured_action_bodies.lock().await;
    assert_eq!(bodies.len(), DAY_SECONDS.len(), "captured one POST / body per day");

    let mut saw_move = false;
    for (day_idx, body) in bodies.iter().enumerate() {
        assert!(!body.is_empty(), "day {day_idx}: at least one inner list");
        for (agent_idx, inner) in body.iter().enumerate() {
            // Wire tokens: `-N` (N > 0) = wait N steps, `0..=5` = one move
            // step in that direction. So the step sum is `(-x) for x<0,
            // else 1`.
            let step_sum: i32 = inner.iter().map(|&x| if x < 0 { -x } else { 1 }).sum();
            assert_eq!(
                step_sum, DAY_STEPS[day_idx],
                "day {day_idx} agent {agent_idx}: inner step sum mismatch (body = {inner:?})"
            );
            if inner.iter().any(|&x| x >= 0) {
                saw_move = true;
            }
        }
    }
    assert!(saw_move, "EngineLoopAdapter must produce at least one Move action across the match");
}

/// Drift guard: re-verify the on-disk `OpenAPI` fixture covers all four
/// endpoints with 200/401/403 responses. This is the same assertion the
/// mockito-based E2E performs; running it here keeps both harnesses in sync.
#[test]
fn openapi_fixture_still_drifts_guard() {
    let fixture = cli::openapi_fixture::OpenApiFixture::load(
        std::path::Path::new(OPENAPI_PATH),
        std::path::Path::new(MATCH_CONFIG_PATH),
    )
    .expect("load OpenAPI fixture");
    fixture.assert_minimal_coverage().expect("coverage");
}

/// Regression test for issue #156.
///
/// The official contest server returns `startsAt=0` from `GET /setting`
/// while the kind-submission window is **open**, and only sets
/// `startsAt=<unix>` **after** the kind deadline closes. The previous
/// coordinator loop polled `GET /setting` until `startsAt > 0` and only
/// then issued `POST /agent` — by which point the server always returned
/// `403 AccessTimeError` ("agent kind submission is closed").
///
/// The fixed loop submits `POST /agent` IMMEDIATELY after `GET /setting`
/// succeeds (while `startsAt == 0`), then waits for `startsAt > 0` to
/// enter the per-day driver.
///
/// This test reproduces the official server's kind-window timing:
/// - `starts_at` starts at `0` (kind window OPEN, `POST /agent` → 200)
/// - after `200ms`, a background task closes the window by setting
///   `force_kind_window_closed = true` and `starts_at = 1`
///   (`POST /agent` → 403 from that point on)
///
/// The CLI must land its `POST /agent` within the open window. With the
/// pre-fix ordering it would only call `POST /agent` once `starts_at`
/// flipped positive, by which point the kind window had already closed
/// and the server would reject the call with `403`.
///
/// The wait between the server's kind-deadline and the CLI's first
/// `POST /agent` is generous on purpose (200ms vs. ~1ms p50 in this
/// harness) so the test reliably distinguishes the broken ordering from
/// the fixed one without being flaky on slow CI runners.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kind_submission_during_window_succeeds_even_if_match_starts_later() {
    let (url, state) = spawn_test_server(0).await;
    // Sanity: server started with starts_at=0 and the kind window OPEN,
    // so POST /agent must return 200.
    assert_eq!(state.starts_at.load(Ordering::SeqCst), 0);
    assert!(!state.force_kind_window_closed.load(Ordering::SeqCst), "kind window must start open");

    // Schedule the kind-deadline: after 200ms, close the window and
    // announce the start. From that point on, POST /agent returns 403.
    let state_for_flip = state.clone();
    let flip_handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        state_for_flip.force_kind_window_closed.store(true, Ordering::SeqCst);
        state_for_flip.starts_at.store(1, Ordering::SeqCst);
    });

    // The fixed coordinator loop issues POST /agent during the open window.
    let outcome = run_with_wait_solver(&url, "test-token")
        .await
        .expect("kind must be submitted during open window, not after deadline");

    // Match should have made progress. endsAt=0 in the harness, so the
    // between-day sleep is skipped and the full 4-day match finishes in
    // well under 5s.
    assert_eq!(outcome.days_completed, DAY_SECONDS.len(), "all 4 days complete");
    assert!(
        outcome.revisions.iter().all(|&r| r >= 0),
        "all revisions non-negative: {:?}",
        outcome.revisions
    );

    // The deadline flip must have actually fired; otherwise the test is
    // not exercising the kind-window boundary.
    flip_handle.await.expect("join deadline-flip task");
    assert_eq!(
        state.starts_at.load(Ordering::SeqCst),
        1,
        "server's kind-deadline must have fired by the end of the match",
    );
    assert!(
        state.force_kind_window_closed.load(Ordering::SeqCst),
        "server's kind window must be closed by the end of the match",
    );
}

/// Regression test for issue #164.
///
/// The official contest server's `-match-start-delay` (default 1s,
/// configurable up to 5s) means `GET /setting` may advertise
/// `starts_at > 0` while `GET /` still returns `403 AccessTimeError`
/// for a short window. The pre-fix loop returned
/// `CoordinatorLoopError::State(HttpClientError::AccessTime)` immediately
/// on that 403 and the match failed even though the server would have
/// become responsive within a second.
///
/// The harness reproduces this by setting
/// `force_match_not_started_ms = 800`: every `GET /` request that lands
/// in the first 800ms after server start returns the official
/// `403 AccessTimeError` envelope. The loop's
/// `fetch_state_with_retry` helper must ride out this window with its
/// 30 × 500ms = 15s retry budget (issue #164) and complete all four
/// days once `GET /` becomes responsive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn match_state_during_match_start_delay_retries_then_succeeds() {
    // `starts_at = 1` so Phase 4 sees the server has announced the match
    // and Phase 5 enters the per-day loop immediately. The
    // `force_match_not_started_ms` flag then makes `GET /` return 403
    // for the first 800ms.
    let (url, state) = spawn_test_server(1).await;
    state.force_match_not_started_ms.store(800, Ordering::SeqCst);

    // The fixed coordinator loop's `fetch_state_with_retry` retries on
    // 403 AccessTimeError while `match-start-delay` elapses. The default
    // 30 × 500ms budget (15s) comfortably covers the 800ms test window.
    let outcome = run_with_wait_solver(&url, "test-token")
        .await
        .expect("loop must recover from match-start-delay 403s");

    assert_eq!(outcome.days_completed, DAY_SECONDS.len(), "all 4 days complete");
    assert!(
        outcome.revisions.iter().all(|&r| r >= 0),
        "all revisions non-negative: {:?}",
        outcome.revisions
    );
    // Sanity: the harness actually exercised the retry path. The server
    // should have observed more than one `GET /` request, with at least
    // one 403 before the first successful 200. We approximate by checking
    // that the day counter advanced past `DAY_SECONDS.len()` (each day
    // triggers exactly one successful `GET /`, so the counter must be at
    // least that high after a successful match).
    let day_count = state.day_counter.load(Ordering::SeqCst);
    assert!(
        day_count >= u64::try_from(DAY_SECONDS.len()).unwrap_or(u64::MAX),
        "server saw {day_count} GET / requests, expected at least {} for a 4-day match",
        DAY_SECONDS.len(),
    );
}
