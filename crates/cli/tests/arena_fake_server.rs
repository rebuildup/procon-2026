//! Fake-arena end-to-end for Issue #179 / #222 — **no test here ever
//! contacts `procon37arena.online`**. Everything runs against an in-process
//! axum server on `127.0.0.1:<random>`.
//!
//! Scenarios, each mapped to an AC:
//!
//! 1. registration waits through `403 AccessTimeError` and registers on the
//!    first 200 (AC: 403 during the wait is not an error, reused PR #165
//!    retry semantics; bounded by the window),
//! 2. the SPA HTML shell fails closed after exactly one request — on the
//!    official `/setting` registration path **and** on the plural
//!    `/settings` SPA fallback the live smoke hit (AC: drift ⇒ fail closed,
//!    no retry storm; Issue #222),
//! 3. an all-403 window gives up inside the attempt budget
//!    (AC: no sub-cadence polling spam),
//! 4. a full registration + coordinator match + evidence record
//!    (AC: registration phase, durable evidence, candidate correspondence),
//!    including the Issue #236 per-day evidence — `days[]` carries each
//!    played day's day-start `kind / pos / fuel` and a movement summary of
//!    the plan we submitted (`moves + waits == daySteps` for these
//!    wait-only submissions), with `daySteps` / `daySeconds` recorded once
//!    at the top level,
//! 5. the **live lobby state**: a confirming `403 AccessTimeError`
//!    (`lobby: you are registered for match <id> departing at <unix>`)
//!    stops the poll and records match id + departure time, then the 200
//!    `GET /setting` hands over to the coordinator flow (AC: lobby-403 is
//!    the registration success, Issue #222),
//! 6. the `procon` binary itself runs `--arena` end to end, writes
//!    evidence and exits, without leaking the injected token
//!    (AC: standard entry + token hygiene + offline isolation of the
//!    default path),
//! 7. the same binary run **without** `PROCON_SOURCE_COMMIT_SHA` records
//!    this checkout's git head instead of `"unknown"` (AC: evidence
//!    `source_commit_sha` fix, Issue #222).
//!
//! Registration targets `GET /setting` (singular — the official endpoint,
//! Issue #222). The fake still serves the plural `/settings` route with the
//! arena's HTML SPA shell, because that is what the real arena answers
//! there: a regression that reintroduces the plural path fails closed.
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

/// One scripted `GET /setting` answer. The last entry repeats forever.
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

/// The **live lobby state** (recon 2026-09-27, Issue #222): a
/// `403 AccessTimeError` whose message confirms the registration. This 403
/// IS the success signal — the poll must stop and record match id +
/// departure time.
fn lobby_registered(match_id: &str, departing_at_unix: i64) -> Scripted {
    status(
        403,
        "application/json",
        json!({
            "code": "AccessTimeError",
            "message": format!(
                "lobby: you are registered for match {match_id} departing at {departing_at_unix}"
            ),
        })
        .to_string(),
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

/// The game setting on its own — what the coordinator loop expects from
/// `GET /setting` once registration is over (or never happened).
fn setting_200() -> Scripted {
    status(200, "application/json", setting_body().to_string())
}

/// One answer that satisfies **both** consumers of `GET /setting`: the
/// registration poll (200 JSON, `matchId` picked up when present) and the
/// coordinator's `MatchSetting` parse (the extra field is ignored — the
/// schema does not deny unknown fields). Mirrors the live shape: the
/// official endpoint really is the singular `/setting` (Issue #222).
fn setting_with_match_id(match_id: &str) -> Scripted {
    let mut body = setting_body();
    body["matchId"] = json!(match_id);
    status(200, "application/json", body.to_string())
}

#[derive(Clone)]
struct FakeArena {
    setting_script: Arc<Mutex<Vec<Scripted>>>,
    setting_hits: Arc<AtomicU32>,
    /// Hits on the plural `/settings` SPA-fallback route (live recon 2026-09-27:
    /// the arena answers HTML there, token or not).
    spa_fallback_hits: Arc<AtomicU32>,
    saw_sentinel_token: Arc<AtomicBool>,
    day: Arc<AtomicU32>,
    revision: Arc<AtomicU32>,
}

impl FakeArena {
    fn new(script: Vec<Scripted>) -> Self {
        Self {
            setting_script: Arc::new(Mutex::new(script)),
            setting_hits: Arc::new(AtomicU32::new(0)),
            spa_fallback_hits: Arc::new(AtomicU32::new(0)),
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

fn render(scripted: Scripted) -> Response {
    let status = StatusCode::from_u16(scripted.status).expect("valid status");
    ([(header::CONTENT_TYPE, scripted.content_type)], (status, scripted.body)).into_response()
}

/// Scripted `GET /setting` (singular) — registration and the coordinator's
/// setting probe share this one official endpoint (Issue #222). The last
/// script entry repeats forever.
async fn serve_setting_script(State(arena): State<FakeArena>, headers: HeaderMap) -> Response {
    arena.setting_hits.fetch_add(1, Ordering::SeqCst);
    if headers.get("Procon-Token").and_then(|v| v.to_str().ok()) == Some(SENTINEL) {
        arena.saw_sentinel_token.store(true, Ordering::SeqCst);
    }
    let scripted = {
        let mut script = arena.setting_script.lock().expect("script lock");
        if script.len() > 1 { Some(script.remove(0)) } else { script.first().cloned() }
    };
    render(scripted.unwrap_or_else(|| registered_200("m-default")))
}

/// The plural `/settings`: always the SPA HTML shell, token or not — the
/// live arena's fallback route, confirmed by the Issue #222 recon
/// (`GET /settings` → `200 text/html` even with `Procon-Token`).
async fn serve_spa_fallback(State(arena): State<FakeArena>) -> Response {
    arena.spa_fallback_hits.fetch_add(1, Ordering::SeqCst);
    render(html_shell())
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
        // Official singular endpoint: registration + setting probe.
        .route("/setting", get(serve_setting_script))
        // Plural SPA fallback — HTML on the live arena, HTML here too.
        .route("/settings", get(serve_spa_fallback))
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
        arena.setting_hits.load(Ordering::SeqCst)
    };
    assert_eq!(hits, 3, "exactly one request per attempt — no hidden polling");
}

// ---------------------------------------------------------------------------
// 2. Drift: SPA HTML shell with HTTP 200 ⇒ fail closed after one request —
//    on the official `/setting` registration path *and* on the plural
//    `/settings` SPA fallback (the path the live smoke hit, Issue #222).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_fails_closed_on_the_spa_html_shell_after_a_single_request() {
    let arena = FakeArena::new(vec![html_shell()]);
    let url = spawn(arena.clone()).await;
    let err = arena::wait_for_registration(
        &client(&url),
        &profile(url.clone()),
        &options(open_window(5_000), 10),
        Utc::now(),
    )
    .await
    .expect_err("HTML where JSON was promised must fail");
    match &err {
        arena::ArenaError::ApiDrift { path, detail } => {
            // The official singular endpoint — registering on the plural
            // SPA fallback is what broke the live smoke (Issue #222).
            assert_eq!(path, "/setting");
            assert_eq!(path, arena::ARENA_REGISTRATION_PATH);
            assert!(detail.contains("text/html"), "{detail}");
        }
        other => panic!("expected ApiDrift, got {other:?}"),
    }
    assert_eq!(
        arena.setting_hits.load(Ordering::SeqCst),
        1,
        "drift must stop immediately — no retry storm, no re-shaped request"
    );
    assert_eq!(err.status(), ArenaRunStatus::ApiDrift);

    // The plural fallback keeps its own fail-closed coverage: the live
    // recon saw `GET /settings` answer `200 text/html` even with a token
    // attached, so any request that lands there must be rejected as drift
    // too — and only after that single request.
    let err = client(&url)
        .get_json::<serde_json::Value>("/settings")
        .await
        .expect_err("the plural SPA fallback must never decode as JSON");
    match &err {
        protocol::HttpClientError::ApiDrift { path, detail } => {
            assert_eq!(path, "/settings");
            assert!(detail.contains("text/html"), "{detail}");
        }
        other => panic!("expected ApiDrift, got {other:?}"),
    }
    assert_eq!(
        arena.spa_fallback_hits.load(Ordering::SeqCst),
        1,
        "drift on the plural path must also stop after one request"
    );
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
    let hits = arena.setting_hits.load(Ordering::SeqCst);
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
        arena.setting_hits.load(Ordering::SeqCst),
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
        // One official `GET /setting` answer serves both phases: it
        // registers (the `matchId` is picked up) and then drives the
        // coordinator's setting probe (the extra field is ignored there).
        let fake = FakeArena::new(vec![setting_with_match_id("m-full")]);
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

        // Issue #236 — per-day evidence: the budgets from `/setting`
        // recorded once, plus one summary per played day carrying the
        // day-start state and a movement summary of what we submitted.
        assert_eq!(parsed.day_steps, vec![16, 16], "`daySteps` recorded once");
        assert_eq!(parsed.day_seconds, vec![30, 30], "`daySeconds` recorded once");
        assert_eq!(parsed.days.len(), 2, "one summary per played day");
        for (idx, day) in parsed.days.iter().enumerate() {
            assert_eq!(day.day, u32::try_from(idx).expect("fits"), "day index");
            // Day-start kind / pos / fuel, exactly what the fake's `GET /`
            // answered for that day.
            assert_eq!(day.agents.len(), 4, "day {idx}: four agents");
            let kinds: Vec<i32> = day.agents.iter().map(|a| a.kind.value()).collect();
            assert_eq!(kinds, vec![0, 1, 0, 1], "day {idx}: declared kind composition");
            let pos: Vec<i32> = day.agents.iter().map(|a| a.pos).collect();
            assert_eq!(pos, vec![0, 8, 16, 24], "day {idx}: day-start positions");
            let fuel: Vec<i32> = day.agents.iter().map(|a| a.fuel).collect();
            assert_eq!(fuel, vec![30, 30, 30, 30], "day {idx}: day-start fuel");
            // Submitted movement summary, one entry per agent in
            // submission order. WaitSolver submits an all-wait plan, so
            // moves + waits must reproduce the day's `daySteps` exactly.
            assert_eq!(day.movement.len(), 4, "day {idx}: one summary per agent");
            for (agent, m) in day.movement.iter().enumerate() {
                assert_eq!(m.moves, 0, "day {idx} agent {agent}: wait-only plan");
                assert_eq!(m.runs, 1, "day {idx} agent {agent}: one coalesced wait run");
                assert_eq!(m.waits, 16, "day {idx} agent {agent}: wait steps == daySteps");
                assert_eq!(
                    m.moves + m.waits,
                    u32::try_from(parsed.day_steps[idx]).expect("daySteps is positive"),
                    "day {idx} agent {agent}: moves + waits must equal daySteps"
                );
                assert_eq!(m.dirs, [0; 6], "day {idx} agent {agent}: no directional moves");
            }
        }

        // Offline/arena naming separation: this is emphatically not a
        // bench RunRecord.
        assert_ne!(parsed.record_kind, "run");
        let _ = std::fs::remove_dir_all(&dir);
    });
}

// ---------------------------------------------------------------------------
// 5. Live lobby state: the confirming 403 IS the registration success —
//    the poll stops, match id + departure time land in the evidence, and
//    a 200 `GET /setting` then hands over to the coordinator flow.
// ---------------------------------------------------------------------------

#[test]
fn lobby_403_registration_stops_the_poll_and_records_match_and_departure() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("rt");
    rt.block_on(async {
        let departing = 1_790_001_000_i64;
        // Script: plain 403 (the "not yet" retry path), then the lobby
        // confirmation (registration success), then the game setting the
        // coordinator needs. The last entry repeats forever.
        let fake = FakeArena::new(vec![
            access_time_403(),
            lobby_registered("469", departing),
            setting_200(),
        ]);
        let url = spawn(fake.clone()).await;
        let profile = profile(url.clone());
        let client = client(&url);

        // Phase A: register — one plain 403, then the confirming 403.
        let registration = arena::wait_for_registration(
            &client,
            &profile,
            &options(open_window(5_000), 10),
            Utc::now(),
        )
        .await
        .expect("the lobby confirmation is a successful registration");
        assert_eq!(registration.attempts, 2, "plain 403 first, then the confirming 403");
        assert_eq!(registration.arena_match_id.as_deref(), Some("469"));
        assert_eq!(
            registration.window.starts_at.timestamp(),
            departing,
            "the arena's departure time re-bases the window"
        );
        assert_eq!(
            fake.setting_hits.load(Ordering::SeqCst),
            2,
            "polling must stop at the confirmation — a further request would \
             just repeat the lobby 403"
        );

        // Phase B: at match time `GET /setting` answers 200 ⇒ the
        // unchanged canonical coordinator flow takes over (supervisor
        // recon 2026-09-27: the same endpoint serves the MatchSetting).
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

        // Phase C: the evidence fields the AC calls out.
        let (status, error) = arena::classify_loop_result(&Ok(outcome.clone()));
        let record = ArenaRunRecord::build(arena::ArenaRecordInput {
            profile: &profile,
            candidate_id: Some("wait"),
            solver: arena::ArenaSolverConfig {
                mode: "candidate".to_owned(),
                solver: "wait".to_owned(),
                loop_initial_strategy: None,
                loop_iterations: None,
            },
            team_id: "arena-team",
            source_commit_sha: "lobby-sha".to_owned(),
            window_starts_at_unix: registration.window.starts_at.timestamp(),
            registration: Some(&registration),
            outcome: Some(&outcome),
            status,
            error,
        });
        assert_eq!(record.status, ArenaRunStatus::Completed);
        assert_eq!(record.registration_attempts, Some(2));
        assert!(record.registered_at_unix.is_some(), "registered_at must be recorded");
        assert_eq!(record.arena_match_id.as_deref(), Some("469"));
        assert_eq!(record.window_starts_at_unix, departing, "departure unix recorded");
        assert_eq!(record.days_completed, 2);
        // Issues #238/#243 — the composition this run declared in
        // `POST /agent` (wait, no `--kinds` ⇒ all patrol) is evidence too.
        assert_eq!(
            record.kinds,
            vec![engine::model::AgentKind::Patrol; 4],
            "the submitted agent-kind composition must be recorded"
        );
        // Issue #236 — the lobby path records the same per-day evidence.
        assert_eq!(record.day_steps, vec![16, 16]);
        assert_eq!(record.days.len(), 2, "one summary per played day");
        for day in &record.days {
            assert_eq!(day.agents.len(), 4, "day-start state of every agent");
            assert_eq!(day.movement.len(), 4, "movement summary for every agent");
            assert!(day.movement.iter().all(|m| m.moves + m.waits == 16_u32));
        }
    });
}

// ---------------------------------------------------------------------------
// 6. The binary itself: `coordinator --arena …` end to end.
// ---------------------------------------------------------------------------

#[test]
fn binary_arena_run_records_evidence_and_exits_without_leaking_the_token() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("rt");
    let (evidence, state_output, url, dir) = rt.block_on(async {
        let fake = FakeArena::new(vec![setting_200()]);
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
            // Issues #238/#243: the flag has to survive the whole path —
            // clap → loop → `POST /agent` → the evidence record below.
            "--kinds",
            "S",
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
    // Issues #238/#243 — `--kinds S` on a 4-agent team must show up in the
    // durable evidence as the composition actually submitted.
    assert_eq!(
        record.kinds,
        vec![engine::model::AgentKind::Supply; 4],
        "--kinds must reach the evidence record: {:?}",
        record.kinds
    );
    assert!(record.registration_attempts.is_none(), "no --next means no registration phase");
    // Issue #236 — the binary path writes the per-day evidence too.
    assert_eq!(record.day_steps, vec![16, 16]);
    assert_eq!(record.day_seconds, vec![30, 30]);
    assert_eq!(record.days.len(), 2, "one summary per played day");
    assert_eq!(record.days[0].day, 0);
    assert_eq!(record.days[1].day, 1);
    assert!(
        record.days.iter().all(|d| d.agents.len() == 4 && d.movement.len() == 4),
        "kind/pos/fuel + movement summary for every agent of every day: {:?}",
        record.days
    );

    // The state snapshot (a durable artifact) is token-free too.
    let state = std::fs::read_to_string(&state_output).expect("snapshot written");
    assert!(!state.contains(SENTINEL), "token in state snapshot: {state}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The `source_commit_sha` half of the fix: without the `just`-injected
/// `PROCON_SOURCE_COMMIT_SHA` (the live smoke ran `cargo run` directly and
/// recorded `"unknown"`, Issue #222) the evidence carries this checkout's
/// git head instead.
#[test]
fn binary_arena_evidence_falls_back_to_the_git_head_without_the_env_var() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("rt");
    let (evidence, state_output, url, dir) = rt.block_on(async {
        let fake = FakeArena::new(vec![setting_200()]);
        let url = spawn(fake).await;
        let dir = std::env::temp_dir().join(format!("procon-arena-sha-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        (dir.join("runs.jsonl"), dir.join("state.json"), url, dir)
    });

    // The expected head, resolved exactly the way the binary resolves it:
    // `git rev-parse HEAD` from the crate directory inside the worktree.
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git available in the dev/CI environment");
    assert!(out.status.success(), "tests must run inside the git worktree");
    let expected = String::from_utf8(out.stdout).expect("utf8").trim().to_owned();

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
        // Deliberately absent: the git fallback must supply the sha.
        .env_remove("PROCON_SOURCE_COMMIT_SHA")
        .env_remove("PROCON_TOKEN")
        .env_remove("PROCON_SERVER")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("spawn procon");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "arena run must succeed\nstderr: {stderr}");

    let text = std::fs::read_to_string(&evidence).expect("evidence written");
    let record: ArenaRunRecord = serde_json::from_str(text.trim()).expect("parses");
    assert_eq!(record.source_commit_sha, expected, "evidence must record the git head");
    assert_ne!(record.source_commit_sha, "unknown");
    assert!(!text.contains(SENTINEL), "token in evidence: {text}");

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
