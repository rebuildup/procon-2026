//! Fake-arena end-to-end for Issue #179 — **no test here ever contacts
//! `procon37arena.online`**. Everything runs against an in-process axum
//! server on `127.0.0.1:<random>`.
//!
//! Scenarios, each mapped to an AC:
//!
//! 1. registration waits through `403 AccessTimeError` and registers on the
//!    first 200 (AC: 403 during the wait is not an error, reused PR #165
//!    retry semantics; bounded by the window),
//! 2. the SPA HTML shell fails closed after exactly one request
//!    (AC: drift ⇒ fail closed, no retry storm),
//! 3. an all-403 window gives up inside the attempt budget
//!    (AC: no sub-cadence polling spam),
//! 4. a full registration + coordinator match + evidence record
//!    (AC: registration phase, durable evidence, candidate correspondence),
//! 5. the `procon` binary itself runs `--arena` end to end, writes
//!    evidence and exits, without leaking the injected token
//!    (AC: standard entry + token hygiene + offline isolation of the
//!    default path).
//!
//! The default (non-arena) coordinator path is proven unchanged by the
//! pre-existing suites (`e2e_openapi_fixture`, `e2e_real_server`,
//! `coordinator_state_snapshot`) running untouched alongside these.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use cli::arena::{
    self, ArenaProfile, ArenaRunRecord, ArenaRunStatus, MatchWindow, RegistrationOptions, Secret,
};
use cli::coordinator_loop::{CoordinatorLoopConfig, run_coordinator_loop};
use cli::solver_adapter::WaitSolver;
use protocol::http_client::{MatchClient, RetryPolicy};

/// Sentinel token. Must never appear in stdout, stderr, logs, snapshots or
/// evidence — asserted explicitly wherever output is observable.
const SENTINEL: &str = "sentinel-arena-token-9f3c-never-log";

/// One scripted `GET /settings` answer. The last entry repeats forever.
#[derive(Clone)]
struct Scripted {
    status: u16,
    content_type: &'static str,
    body: String,
}

fn status(status: u16, content_type: &'static str, body: String) -> Scripted {
    Scripted { status, content_type, body }
}

fn access_time_403() -> Scripted {
    status(
        403,
        "application/json",
        json!({"code": "AccessTimeError", "message": "試合がまだ開始されていません"}).to_string(),
    )
}

fn registered_200(match_id: &str) -> Scripted {
    status(200, "application/json", json!({"matchId": match_id}).to_string())
}

fn html_shell() -> Scripted {
    status(
        200,
        "text/html; charset=utf-8",
        "<!DOCTYPE html><html><title>ヘキサうどん 非公式練習場</title></html>".to_owned(),
    )
}

#[derive(Clone)]
struct FakeArena {
    settings_script: Arc<Mutex<Vec<Scripted>>>,
    settings_hits: Arc<AtomicU32>,
    saw_sentinel_token: Arc<AtomicBool>,
    day: Arc<AtomicU32>,
    revision: Arc<AtomicU32>,
}

impl FakeArena {
    fn new(script: Vec<Scripted>) -> Self {
        Self {
            settings_script: Arc::new(Mutex::new(script)),
            settings_hits: Arc::new(AtomicU32::new(0)),
            saw_sentinel_token: Arc::new(AtomicBool::new(false)),
            day: Arc::new(AtomicU32::new(0)),
            revision: Arc::new(AtomicU32::new(0)),
        }
    }
}

/// Wire-valid `GET /setting` body: 8x8 board, 4 agents (the engine's
/// 3..=8 team rule), one spot off the agent cells, two days.
fn setting_body() -> Value {
    let mut cells = vec![vec![0_u32; 8]; 8];
    cells[0][4] = 1; // road under the traffic bookkeeping (official: 1 = 道路)
    json!({
        "startsAt": 1,
        "daySeconds": [30, 30],
        "daySteps": [16, 16],
        "map": { "height": 8, "width": 8, "cells": cells },
        "spots": [{ "brand": 0, "pos": 12, "stocks": 4 }],
        "agents": [0, 8, 16, 24],
        "fuelLimits": 30,
        "players": 2,
        "busyThreshold": 1,
        "jammedThreshold": 2,
    })
}

async fn serve_settings(State(arena): State<FakeArena>, headers: HeaderMap) -> Response {
    arena.settings_hits.fetch_add(1, Ordering::SeqCst);
    if headers.get("Procon-Token").and_then(|v| v.to_str().ok()) == Some(SENTINEL) {
        arena.saw_sentinel_token.store(true, Ordering::SeqCst);
    }
    let scripted = {
        let mut script = arena.settings_script.lock().expect("script lock");
        if script.len() > 1 { Some(script.remove(0)) } else { script.first().cloned() }
    };
    let scripted = scripted.unwrap_or_else(|| registered_200("m-default"));
    let status = StatusCode::from_u16(scripted.status).expect("valid status");
    ([(header::CONTENT_TYPE, scripted.content_type)], (status, scripted.body)).into_response()
}

async fn serve_setting() -> Json<Value> {
    Json(setting_body())
}

async fn serve_agent() -> StatusCode {
    StatusCode::OK
}

async fn serve_state(State(arena): State<FakeArena>) -> Json<Value> {
    let day = arena.day.fetch_add(1, Ordering::SeqCst);
    Json(json!({
        // Far in the past: skips the between-day sleep in tests.
        "endsAt": 1,
        "day": day,
        "agents": [
            { "kind": 0, "pos": 0, "fuel": 30 },
            { "kind": 1, "pos": 8, "fuel": 30 },
            { "kind": 0, "pos": 16, "fuel": 30 },
            { "kind": 1, "pos": 24, "fuel": 30 },
        ],
        "others": [],
        "traffics": [],
    }))
}

async fn serve_action(State(arena): State<FakeArena>) -> Json<Value> {
    let revision = arena.revision.fetch_add(1, Ordering::SeqCst);
    Json(json!({ "revision": revision }))
}

async fn spawn(arena: FakeArena) -> String {
    let app = Router::new()
        .route("/settings", get(serve_settings))
        .route("/setting", get(serve_setting))
        .route("/agent", post(serve_agent))
        .route("/", get(serve_state).post(serve_action))
        .with_state(arena);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

fn profile(server_url: String) -> ArenaProfile {
    ArenaProfile { server_url, token: Secret::new(SENTINEL.to_owned()) }
}

fn client(server_url: &str) -> MatchClient {
    MatchClient::builder(server_url, SENTINEL)
        // No backoff: these tests count requests, not wall time.
        .retry_policy(RetryPolicy { max_retries: 1, backoff_millis: &[0] })
        .request_timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

/// A window that is already open for `hold` milliseconds.
fn open_window(hold_ms: i64) -> MatchWindow {
    let now = Utc::now();
    MatchWindow {
        opens_at: now - chrono::TimeDelta::seconds(2),
        starts_at: now + chrono::TimeDelta::milliseconds(hold_ms),
    }
}

fn options(window: MatchWindow, poll_ms: u64) -> RegistrationOptions {
    RegistrationOptions {
        poll_interval: Duration::from_millis(poll_ms),
        window: Some(window),
        ..RegistrationOptions::default()
    }
}

// ---------------------------------------------------------------------------
// 1. 403 during the wait is not an error.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_waits_through_403_and_registers_on_the_first_200() {
    let hits = {
        let arena =
            FakeArena::new(vec![access_time_403(), access_time_403(), registered_200("m-42")]);
        let url = spawn(arena.clone()).await;
        let outcome = arena::wait_for_registration(
            &client(&url),
            &profile(url),
            &options(open_window(5_000), 10),
            Utc::now(),
        )
        .await
        .expect("registration succeeds after the window opens");
        assert_eq!(outcome.attempts, 3, "two 403s then the registering 200");
        assert_eq!(outcome.arena_match_id.as_deref(), Some("m-42"));
        assert!(arena.saw_sentinel_token.load(Ordering::SeqCst), "token header sent");
        arena.settings_hits.load(Ordering::SeqCst)
    };
    assert_eq!(hits, 3, "exactly one request per attempt — no hidden polling");
}

// ---------------------------------------------------------------------------
// 2. Drift: SPA HTML shell with HTTP 200 ⇒ fail closed after one request.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_fails_closed_on_the_spa_html_shell_after_a_single_request() {
    let arena = FakeArena::new(vec![html_shell()]);
    let url = spawn(arena.clone()).await;
    let err = arena::wait_for_registration(
        &client(&url),
        &profile(url),
        &options(open_window(5_000), 10),
        Utc::now(),
    )
    .await
    .expect_err("HTML where JSON was promised must fail");
    match &err {
        arena::ArenaError::ApiDrift { path, detail } => {
            assert_eq!(path, "/settings");
            assert!(detail.contains("text/html"), "{detail}");
        }
        other => panic!("expected ApiDrift, got {other:?}"),
    }
    assert_eq!(
        arena.settings_hits.load(Ordering::SeqCst),
        1,
        "drift must stop immediately — no retry storm, no re-shaped request"
    );
    assert_eq!(err.status(), ArenaRunStatus::ApiDrift);
}

// ---------------------------------------------------------------------------
// 3. Bounded budget: an all-403 window gives up inside the window.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_gives_up_within_the_window_budget() {
    let arena = FakeArena::new(vec![access_time_403()]);
    let url = spawn(arena.clone()).await;
    // ~100ms window at a 10ms interval ⇒ on the order of ten attempts.
    let err = arena::wait_for_registration(
        &client(&url),
        &profile(url),
        &options(open_window(100), 10),
        Utc::now(),
    )
    .await
    .expect_err("window closes before the arena admits us");
    assert!(matches!(err, arena::ArenaError::NotRegistered { .. }), "got {err:?}");
    let hits = arena.settings_hits.load(Ordering::SeqCst);
    assert!(
        (2..=15).contains(&hits),
        "polling must stay bounded by the window, got {hits} requests"
    );
    assert_eq!(err.status(), ArenaRunStatus::RegistrationFailed);
}

/// `--next` before the window opens: sleep until `opens_at`, then
/// register — never poll during the wait (that would be sub-cadence spam).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_sleeps_until_the_window_opens_without_polling() {
    let arena = FakeArena::new(vec![registered_200("m-sleep")]);
    let url = spawn(arena.clone()).await;
    let now = Utc::now();
    let window = MatchWindow {
        opens_at: now + chrono::TimeDelta::milliseconds(150),
        starts_at: now + chrono::TimeDelta::seconds(5),
    };
    let started = std::time::Instant::now();
    let outcome =
        arena::wait_for_registration(&client(&url), &profile(url), &options(window, 10), now)
            .await
            .expect("registers once the window opens");
    assert!(
        started.elapsed() >= Duration::from_millis(120),
        "must wait for opens_at first, took {:?}",
        started.elapsed()
    );
    assert_eq!(outcome.attempts, 1);
    assert_eq!(
        arena.settings_hits.load(Ordering::SeqCst),
        1,
        "no request may be sent before the window opens"
    );
}

// ---------------------------------------------------------------------------
// 4. Registration + full match + durable evidence (library level).
// ---------------------------------------------------------------------------

#[test]
fn registration_match_and_evidence_record_form_one_coherent_run() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("rt");
    rt.block_on(async {
        let fake = FakeArena::new(vec![registered_200("m-full")]);
        let url = spawn(fake.clone()).await;
        let profile = profile(url.clone());
        let client = client(&url);

        // Phase A: register.
        let registration = arena::wait_for_registration(
            &client,
            &profile,
            &options(open_window(5_000), 10),
            Utc::now(),
        )
        .await
        .expect("registers");

        // Phase B: play the match through the unchanged canonical loop.
        let cfg = CoordinatorLoopConfig {
            team_id: "arena-team".to_owned(),
            setting_poll_interval: Duration::from_millis(20),
            solver_name: "wait".to_owned(),
            server_url: Some(url.clone()),
            ..CoordinatorLoopConfig::default()
        };
        let outcome =
            run_coordinator_loop(client, Box::new(WaitSolver), cfg).await.expect("match completes");
        assert_eq!(outcome.days_completed, 2);

        // Phase C: evidence.
        let (status, error) = arena::classify_loop_result(&Ok(outcome.clone()));
        assert_eq!(status, ArenaRunStatus::Completed);
        let record = ArenaRunRecord::build(arena::ArenaRecordInput {
            profile: &profile,
            candidate_id: Some("loop:iterations=32:initial=greedy:lookahead=crossday"),
            solver: arena::ArenaSolverConfig {
                mode: "candidate".to_owned(),
                solver: "loop".to_owned(),
                loop_initial_strategy: None,
                loop_iterations: None,
            },
            team_id: "arena-team",
            source_commit_sha: "cafebabe".to_owned(),
            window_starts_at_unix: registration.window.starts_at.timestamp(),
            registration: Some(&registration),
            outcome: Some(&outcome),
            status,
            error,
        });

        let dir = std::env::temp_dir().join(format!("procon-arena-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runs.jsonl");
        arena::append_evidence(&path, &record).expect("appends");
        let text = std::fs::read_to_string(&path).expect("readable");

        assert!(!text.contains(SENTINEL), "token leaked into evidence: {text}");
        let parsed: ArenaRunRecord =
            serde_json::from_str(text.trim()).expect("evidence parses as ArenaRunRecord");
        assert_eq!(parsed.record_kind, "arena-run");
        assert_eq!(
            parsed.candidate_id.as_deref(),
            Some("loop:iterations=32:initial=greedy:lookahead=crossday")
        );
        assert_eq!(parsed.source_commit_sha, "cafebabe");
        assert_eq!(parsed.status, ArenaRunStatus::Completed);
        assert_eq!(parsed.days_completed, 2);
        assert_eq!(parsed.registration_attempts, Some(1));
        assert_eq!(parsed.arena_match_id.as_deref(), Some("m-full"));
        assert_eq!(parsed.team_id, "arena-team");
        assert!(!parsed.revisions.is_empty());
        // Offline/arena naming separation: this is emphatically not a
        // bench RunRecord.
        assert_ne!(parsed.record_kind, "run");
        let _ = std::fs::remove_dir_all(&dir);
    });
}

// ---------------------------------------------------------------------------
// 5. The binary itself: `coordinator --arena …` end to end.
// ---------------------------------------------------------------------------

#[test]
fn binary_arena_run_records_evidence_and_exits_without_leaking_the_token() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("rt");
    let (evidence, state_output, url, dir) = rt.block_on(async {
        let fake = FakeArena::new(vec![registered_200("m-bin")]);
        let url = spawn(fake).await;
        let dir = std::env::temp_dir().join(format!("procon-arena-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        (dir.join("runs.jsonl"), dir.join("state.json"), url, dir)
    });

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_procon"))
        .args([
            "coordinator",
            "--arena",
            "--server",
            &url,
            "--candidate",
            "wait",
            "--arena-evidence",
            evidence.to_str().expect("utf8 evidence path"),
            "--state-output",
            state_output.to_str().expect("utf8 state path"),
        ])
        .env("PROCON_ARENA_TOKEN", SENTINEL)
        .env("PROCON_SOURCE_COMMIT_SHA", "feedbeef-179")
        // Ambient protocol env must not shadow the arena profile under test.
        .env_remove("PROCON_TOKEN")
        .env_remove("PROCON_SERVER")
        .output()
        .expect("spawn procon");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "arena run must succeed\nstdout: {stdout}\nstderr: {stderr}");
    assert!(!stdout.contains(SENTINEL), "token in stdout: {stdout}");
    assert!(!stderr.contains(SENTINEL), "token in stderr: {stderr}");

    let text = std::fs::read_to_string(&evidence).expect("evidence written");
    assert!(!text.contains(SENTINEL), "token in evidence: {text}");
    let record: ArenaRunRecord = serde_json::from_str(text.trim()).expect("parses");
    assert_eq!(record.record_kind, "arena-run");
    assert_eq!(record.candidate_id.as_deref(), Some("wait"));
    assert_eq!(record.solver.mode, "candidate");
    assert_eq!(record.source_commit_sha, "feedbeef-179");
    assert_eq!(record.status, ArenaRunStatus::Completed);
    assert_eq!(record.days_completed, 2, "two-day fake match played out");
    assert!(record.registration_attempts.is_none(), "no --next means no registration phase");

    // The state snapshot (a durable artifact) is token-free too.
    let state = std::fs::read_to_string(&state_output).expect("snapshot written");
    assert!(!state.contains(SENTINEL), "token in state snapshot: {state}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn next_without_arena_is_rejected_before_touching_anything() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_procon"))
        .args(["coordinator", "--next", "--dry-run"])
        .output()
        .expect("spawn procon");
    assert!(!output.status.success(), "--next outside arena mode must fail fast");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--next requires --arena"), "{stderr}");
}

// ---------------------------------------------------------------------------
// Offline isolation: the default path never gains arena behaviour.
// ---------------------------------------------------------------------------

#[test]
fn default_coordinator_config_carries_no_arena_slots() {
    let cfg = CoordinatorLoopConfig::default();
    assert!(
        cfg.live_state_slot.is_none(),
        "the live-state slot must stay opt-in so the default path is byte-identical"
    );
    assert!(cfg.secret_redaction.is_none(), "no redaction outside arena mode");
    assert!(cfg.engine_setting_slot.is_none());
}

#[test]
fn arena_env_defaults_are_opt_in() {
    // Profile resolution only happens under `--arena`; a missing token is
    // an error *there* and nowhere else.
    let err = arena::resolve_profile(None, None, arena::ArenaEnv::default())
        .expect_err("no token, no arena");
    assert!(err.to_string().contains("PROCON_ARENA_TOKEN"));
    // With the env present the profile resolves to the public default URL —
    // no secret involved in the endpoint itself.
    let profile = arena::resolve_profile(
        None,
        None,
        arena::ArenaEnv { server: None, token: Some(SENTINEL.to_owned()) },
    )
    .expect("resolves");
    assert_eq!(profile.server_url, arena::ARENA_SERVER_DEFAULT);
}

/// `--candidate` resolves the shortlist ids the supervisor selected, and
/// the bridge is what plays them (see the unit tests in
/// `candidate_bridge` for the plan conversion itself).
#[test]
fn shortlist_candidate_ids_resolve_through_the_registry() {
    for id in [
        "multiloop:kmeans=4:seed=42",
        "astar:policy=step",
        "loop:iterations=32:initial=greedy:lookahead=crossday",
        "multiloop:kmeans=4:seed=42:walk=astar",
    ] {
        let bridge = cli::candidate_bridge::CandidateBridge::from_registry_id(id)
            .unwrap_or_else(|e| panic!("shortlist id {id} must resolve: {e}"));
        assert_eq!(bridge.candidate_id(), id, "identity is preserved verbatim");
    }
}
