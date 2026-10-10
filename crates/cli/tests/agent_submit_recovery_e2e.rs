//! Issue #442: an unanswered `POST /agent` must never be replayed, and the
//! coordinator must tell "accepted, acknowledgement lost" apart from
//! "submission never arrived" by reading the composition back from the
//! server's own published state (`GET /` once the match runs).
//!
//! Unlike `e2e_real_server.rs` (whose server always answers promptly), this
//! harness models the failure the finals actually produced: the kind
//! submission is *taken* by the server while its HTTP answer never reaches
//! the client (`agent_stall_ms` holds the response past the client's
//! timeout), the client classifies the POST as a transport failure, and the
//! loop must recover through the read-back instead of dying on it.
//!
//! The five issue cases map onto the tests as follows:
//!
//! 1. server accepted the kinds, answer timed out → read-back confirms, the
//!    match continues: [`accepted_with_lost_ack_continues_by_readback`];
//! 2. submission never arrived (held unread, nothing recorded) → the
//!    outcome stays unknown, ends as
//!    [`CoordinatorLoopError::AgentSubmitUnconfirmed`], exactly one POST:
//!    [`never_arrived_ends_unconfirmed_without_reposting`];
//! 3. `GET /setting` times out once → the read retry recovers mid-match:
//!    [`setting_timeout_is_ridden_out_and_the_match_completes`] (the pure
//!    HTTP level is pinned in `crates/protocol/tests/http_integration.rs`);
//! 4. the kind deadline passes while the answer is still held → the window
//!    is closed by the time the client notices, and a blind re-POST would
//!    burn the whole budget on `403`: [`ack_lost_at_the_kind_deadline`];
//! 5. 401 / a definitive `403` budget / a server that never comes back:
//!    [`auth_fail_is_not_read_back`],
//!    [`exhausted_access_time_budget_is_not_read_back`],
//!    [`server_that_never_comes_back_ends_unconfirmed`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use cli::coordinator_loop::{
    CoordinatorLoopConfig, CoordinatorLoopError, CoordinatorOutcome, run_coordinator_loop,
};
use cli::solver_adapter::WaitSolver;
use protocol::http_client::{MatchClient, RetryPolicy};

/// Map size: the smallest the engine accepts (`8..=32` per axis).
const MAP_WIDTH: i32 = 8;
const MAP_HEIGHT: i32 = 8;

/// A four-day match plan, one per-day budget each.
const DAY_SECONDS: [i32; 4] = [30, 30, 30, 30];
const DAY_STEPS: [i32; 4] = [20, 20, 20, 20];

/// How long a stalled handler holds its answer. Every client timeout in
/// this harness (`500 ms`) is shorter, so the stalled answer is always lost.
const STALL: Duration = Duration::from_millis(1500);

/// How long `GET /setting` stalls the one poisoned attempt (case 3).
const SETTING_STALL: Duration = Duration::from_millis(1000);

/// How long a blacked-out handler holds its connection: far beyond every
/// patience in the harness, so a blackout is "the server never comes back".
const BLACKOUT: Duration = Duration::from_secs(3600);

/// Shared state of the test server.
#[derive(Clone)]
struct ServerState {
    /// UNIX-time match start; `0` keeps the kind window open.
    starts_at: Arc<AtomicI64>,
    /// `GET /setting` requests still to be answered late (case 3).
    setting_stalls_left: Arc<AtomicU64>,
    setting_get_count: Arc<AtomicU64>,
    /// When `true`, `POST /agent` returns `401` (case 5a).
    agent_auth_fail: Arc<AtomicBool>,
    /// When `true`, `POST /agent` returns `403 AccessTimeError` (case 5b).
    kind_window_closed: Arc<AtomicBool>,
    /// How long `POST /agent` holds its answer before replying.
    agent_stall_ms: Arc<AtomicU64>,
    /// Whether a `POST /agent` is actually processed. `true` records the
    /// submitted composition — the answer is what may be lost; `false`
    /// holds the connection without the body ever reaching the matcher.
    agent_accepts: Arc<AtomicBool>,
    agent_post_count: Arc<AtomicU64>,
    /// The composition the matcher holds, as the wire body (`0 | 1`).
    accepted_kinds: Arc<Mutex<Option<Vec<i32>>>>,
    /// When `true`, `GET /` answers; otherwise `403 AccessTimeError` (the
    /// match has not started / was never started for us).
    state_gate_open: Arc<AtomicBool>,
    /// When `true`, `GET /` holds its connection instead of answering.
    state_blackout: Arc<AtomicBool>,
    state_get_count: Arc<AtomicU64>,
    /// Served day = accepted action submissions: the server stays on a day
    /// until that day's plan is taken.
    action_count: Arc<AtomicU32>,
    revision_counter: Arc<AtomicI32>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            starts_at: Arc::new(AtomicI64::new(0)),
            setting_stalls_left: Arc::new(AtomicU64::new(0)),
            setting_get_count: Arc::new(AtomicU64::new(0)),
            agent_auth_fail: Arc::new(AtomicBool::new(false)),
            kind_window_closed: Arc::new(AtomicBool::new(false)),
            agent_stall_ms: Arc::new(AtomicU64::new(0)),
            agent_accepts: Arc::new(AtomicBool::new(true)),
            agent_post_count: Arc::new(AtomicU64::new(0)),
            accepted_kinds: Arc::new(Mutex::new(None)),
            state_gate_open: Arc::new(AtomicBool::new(false)),
            state_blackout: Arc::new(AtomicBool::new(false)),
            state_get_count: Arc::new(AtomicU64::new(0)),
            action_count: Arc::new(AtomicU32::new(0)),
            revision_counter: Arc::new(AtomicI32::new(0)),
        }
    }

    /// A wire-valid `MatchSetting` for the four-day plan.
    fn build_setting(&self) -> Value {
        let width_u = usize::try_from(MAP_WIDTH).expect("width fits usize");
        let height_u = usize::try_from(MAP_HEIGHT).expect("height fits usize");
        json!({
            "startsAt": self.starts_at.load(Ordering::SeqCst),
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

async fn serve_setting(State(s): State<ServerState>) -> (StatusCode, Json<Value>) {
    s.setting_get_count.fetch_add(1, Ordering::SeqCst);
    if s.setting_stalls_left.load(Ordering::SeqCst) > 0 {
        s.setting_stalls_left.fetch_sub(1, Ordering::SeqCst);
        tokio::time::sleep(SETTING_STALL).await;
    }
    (StatusCode::OK, Json(s.build_setting()))
}

/// `POST /agent`. The submission is taken (recorded) **before** the answer
/// is held, which is exactly the finals failure: the server moved on, the
/// acknowledgement did not survive the trip.
async fn serve_agent(
    State(s): State<ServerState>,
    body: Json<Vec<i32>>,
) -> (StatusCode, Json<Value>) {
    s.agent_post_count.fetch_add(1, Ordering::SeqCst);
    if s.agent_auth_fail.load(Ordering::SeqCst) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": "AuthError", "message": "bad token"})),
        );
    }
    if s.kind_window_closed.load(Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": "AccessTimeError",
                "message": "agent kind submission is closed",
            })),
        );
    }
    if s.agent_accepts.load(Ordering::SeqCst) {
        *s.accepted_kinds.lock().await = Some(body.0);
        let stall = s.agent_stall_ms.load(Ordering::SeqCst);
        if stall > 0 {
            tokio::time::sleep(Duration::from_millis(stall)).await;
        }
        (StatusCode::OK, Json(json!({})))
    } else {
        // The submission never reaches the matcher: the connection is held
        // silently, so the client only ever sees a timeout.
        tokio::time::sleep(BLACKOUT).await;
        (StatusCode::OK, Json(json!({})))
    }
}

async fn serve_state(State(s): State<ServerState>) -> (StatusCode, Json<Value>) {
    s.state_get_count.fetch_add(1, Ordering::SeqCst);
    if s.state_blackout.load(Ordering::SeqCst) {
        tokio::time::sleep(BLACKOUT).await;
        return (StatusCode::OK, Json(json!({})));
    }
    if !s.state_gate_open.load(Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": "AccessTimeError",
                "message": "match has not started",
            })),
        );
    }
    let day = i64::from(s.action_count.load(Ordering::SeqCst));
    let kinds = s.accepted_kinds.lock().await.clone().unwrap_or_default();
    let agents: Vec<Value> =
        kinds.iter().map(|&kind| json!({ "kind": kind, "pos": 0, "fuel": 30 })).collect();
    (
        StatusCode::OK,
        Json(json!({
            "endsAt": 0_i64,
            "day": day,
            "agents": agents,
            "others": [],
            "traffics": [],
        })),
    )
}

async fn serve_action(
    State(s): State<ServerState>,
    _body: Json<Vec<Vec<i32>>>,
) -> (StatusCode, Json<Value>) {
    s.action_count.fetch_add(1, Ordering::SeqCst);
    let revision = s.revision_counter.fetch_add(1, Ordering::SeqCst);
    (StatusCode::OK, Json(json!({ "revision": revision })))
}

fn router(state: ServerState) -> Router {
    Router::new()
        .route("/setting", get(serve_setting))
        .route("/agent", post(serve_agent))
        .route("/", get(serve_state).post(serve_action))
        .with_state(state)
}

async fn spawn_test_server() -> (String, ServerState) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let state = ServerState::new();
    let router = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.expect("axum serve") });
    (format!("http://{addr}"), state)
}

/// A client whose single-request timeout is shorter than every stall the
/// harness produces, with a tight retry budget so the waits stay bounded.
fn test_client(server_url: &str) -> MatchClient {
    MatchClient::builder(server_url, "test-token")
        .retry_policy(RetryPolicy { max_retries: 1, backoff_millis: &[10, 10] })
        .request_timeout(Duration::from_millis(500))
        .build()
        .expect("match client builds")
}

/// The loop configuration: a short poll cadence, a two-attempt kind-window
/// budget, and a read-back patience sized per test.
fn test_config(readback_patience: Duration) -> CoordinatorLoopConfig {
    CoordinatorLoopConfig {
        team_id: "mvp-player".to_owned(),
        setting_poll_interval: Duration::from_millis(50),
        solver_name: "wait".to_owned(),
        kind_submit_max_attempts: 2,
        kind_submit_backoff: Duration::from_millis(10),
        match_state_backoff: Duration::from_millis(50),
        submit_readback_patience: readback_patience,
        ..Default::default()
    }
}

/// Drive `run_coordinator_loop` with the debug `WaitSolver` (all-patrol
/// kinds, so the submitted body and the read-back composition are `KIND_0`).
async fn run_wait_solver(
    server_url: &str,
    cfg: CoordinatorLoopConfig,
) -> Result<CoordinatorOutcome, CoordinatorLoopError> {
    let solver: Box<dyn engine::solver::Solver> = Box::new(WaitSolver);
    run_coordinator_loop(test_client(server_url), solver, cfg).await
}

/// The server announces the start (and optionally closes the kind window)
/// after `delay`, while the submission's answer is still being held.
fn announce_start_later(state: &ServerState, delay: Duration, close_kind_window: bool) {
    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        state.starts_at.store(1, Ordering::SeqCst);
        state.state_gate_open.store(true, Ordering::SeqCst);
        if close_kind_window {
            state.kind_window_closed.store(true, Ordering::SeqCst);
        }
    });
}

fn assert_four_days(outcome: &CoordinatorOutcome) {
    assert_eq!(outcome.days_completed, DAY_SECONDS.len(), "all 4 days complete");
    assert_eq!(outcome.revisions, vec![0, 1, 2, 3], "one accepted revision per day, in day order");
}

// ============================================================================
// Test cases
// ============================================================================

/// Case 1: the server takes the kind submission but the acknowledgement is
/// lost (`POST /agent` times out on the client). The loop must not re-post,
/// must read the composition back from `GET /` once the match starts, and
/// must continue the match it would previously have abandoned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_with_lost_ack_continues_by_readback() {
    let (url, state) = spawn_test_server().await;
    state
        .agent_stall_ms
        .store(u64::try_from(STALL.as_millis()).unwrap_or(u64::MAX), Ordering::SeqCst);
    announce_start_later(&state, Duration::from_millis(800), false);

    let outcome = run_wait_solver(&url, test_config(Duration::from_secs(5)))
        .await
        .expect("the lost acknowledgement is recovered by the read-back");

    assert_four_days(&outcome);
    assert_eq!(
        state.agent_post_count.load(Ordering::SeqCst),
        1,
        "an unanswered POST must be sent exactly once"
    );
    let kinds = state.accepted_kinds.lock().await.clone().expect("the submission was taken");
    assert_eq!(kinds, vec![0], "the all-patrol body the wait solver sends");
}

/// Case 4: the kind deadline passes while the answer is still held. By the
/// time the client notices the timeout the window is shut, so a blind
/// re-POST would only ever answer `403` and burn the whole budget — the
/// read-back is what keeps the accepted submission alive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ack_lost_at_the_kind_deadline() {
    let (url, state) = spawn_test_server().await;
    state
        .agent_stall_ms
        .store(u64::try_from(STALL.as_millis()).unwrap_or(u64::MAX), Ordering::SeqCst);
    announce_start_later(&state, Duration::from_millis(800), true);

    let outcome = run_wait_solver(&url, test_config(Duration::from_secs(5)))
        .await
        .expect("the submission accepted before the deadline stands");

    assert_four_days(&outcome);
    assert_eq!(
        state.agent_post_count.load(Ordering::SeqCst),
        1,
        "no re-POST against the closed kind window"
    );
}

/// Case 2: the submission never arrives (the server holds the connection
/// without the body ever being processed) and the match never starts for
/// us. The outcome is genuinely unknown: exactly one POST was sent, the
/// read-back runs for its whole patience, and the run ends classified as
/// unconfirmed instead of guessing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_arrived_ends_unconfirmed_without_reposting() {
    let (url, state) = spawn_test_server().await;
    state.agent_accepts.store(false, Ordering::SeqCst);

    let started = Instant::now();
    let err = run_wait_solver(&url, test_config(Duration::from_millis(1200)))
        .await
        .expect_err("an unconfirmable submission must end the run");

    assert!(
        matches!(err, CoordinatorLoopError::AgentSubmitUnconfirmed { .. }),
        "expected AgentSubmitUnconfirmed, got {err:?}"
    );
    assert_eq!(state.agent_post_count.load(Ordering::SeqCst), 1, "no blind re-POST");
    assert!(
        state.state_get_count.load(Ordering::SeqCst) > 0,
        "the read-back polled GET / before classifying the outcome"
    );
    assert!(started.elapsed() < Duration::from_secs(5), "the failure is bounded");
}

/// Case 3, loop level: one `GET /setting` answer is held past the request
/// timeout; the client's read retry recovers and the match completes. The
/// pure HTTP level of the same behaviour is pinned in
/// `crates/protocol/tests/http_integration.rs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn setting_timeout_is_ridden_out_and_the_match_completes() {
    let (url, state) = spawn_test_server().await;
    state.setting_stalls_left.store(1, Ordering::SeqCst);
    state.starts_at.store(1, Ordering::SeqCst);
    state.state_gate_open.store(true, Ordering::SeqCst);
    // The kind window never shuts in this scenario: POST /agent is answered
    // promptly and the match runs.
    state.agent_stall_ms.store(0, Ordering::SeqCst);

    let outcome = run_wait_solver(&url, test_config(Duration::from_secs(5)))
        .await
        .expect("the stalled setting read is retried");

    assert_four_days(&outcome);
    assert!(state.setting_get_count.load(Ordering::SeqCst) >= 2, "the timed-out read was repeated");
}

/// Case 5a: a `401` on `POST /agent` is definitive. It propagates as
/// `AgentSubmit(Auth)` at once — no read-back, no second POST.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_fail_is_not_read_back() {
    let (url, state) = spawn_test_server().await;
    state.agent_auth_fail.store(true, Ordering::SeqCst);

    let started = Instant::now();
    let err = run_wait_solver(&url, test_config(Duration::from_secs(5)))
        .await
        .expect_err("a 401 must end the run");

    match err {
        CoordinatorLoopError::AgentSubmit(protocol::http_client::HttpClientError::Auth {
            ..
        }) => {}
        other => panic!("expected AgentSubmit(Auth), got {other:?}"),
    }
    assert_eq!(state.agent_post_count.load(Ordering::SeqCst), 1);
    assert_eq!(state.state_get_count.load(Ordering::SeqCst), 0, "no read-back after a 401");
    assert!(started.elapsed() < Duration::from_secs(2), "the failure is immediate");
}

/// Case 5b: a definitive `403 AccessTimeError` (the kind window is shut and
/// stays shut) exhausts its bounded budget and surfaces as
/// `AgentSubmit(AccessTime)` — no read-back masquerading the refusal as an
/// unknown outcome.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_access_time_budget_is_not_read_back() {
    let (url, state) = spawn_test_server().await;
    state.kind_window_closed.store(true, Ordering::SeqCst);

    let err = run_wait_solver(&url, test_config(Duration::from_secs(5)))
        .await
        .expect_err("a closed kind window must end the run");

    match err {
        CoordinatorLoopError::AgentSubmit(protocol::http_client::HttpClientError::AccessTime {
            ..
        }) => {}
        other => panic!("expected AgentSubmit(AccessTime), got {other:?}"),
    }
    assert_eq!(
        state.agent_post_count.load(Ordering::SeqCst),
        2,
        "the kind-window budget is burned, nothing more"
    );
    assert_eq!(state.state_get_count.load(Ordering::SeqCst), 0, "no read-back after a refusal");
}

/// Case 5c: the submission was taken, but the server never comes back —
/// every read-back answer is held past the timeout. The run ends classified
/// as unconfirmed after the patience, with exactly one POST on the wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_that_never_comes_back_ends_unconfirmed() {
    let (url, state) = spawn_test_server().await;
    state
        .agent_stall_ms
        .store(u64::try_from(STALL.as_millis()).unwrap_or(u64::MAX), Ordering::SeqCst);
    let black = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        black.state_blackout.store(true, Ordering::SeqCst);
    });

    let started = Instant::now();
    let err = run_wait_solver(&url, test_config(Duration::from_millis(1200)))
        .await
        .expect_err("an unreadable server state must end the run");

    assert!(
        matches!(err, CoordinatorLoopError::AgentSubmitUnconfirmed { .. }),
        "expected AgentSubmitUnconfirmed, got {err:?}"
    );
    assert_eq!(state.agent_post_count.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < Duration::from_secs(10), "the failure is bounded");
}
