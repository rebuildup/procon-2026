//! End-to-end checks of the match record (`--record-dir`, Issue #394).
//!
//! A real axum server on `127.0.0.1` plays the official four endpoints and
//! keeps every body it served and received. Each test drives the canonical
//! `run_coordinator_loop` against it and compares the run directory with
//! what actually crossed the wire:
//!
//! - 16×16 / 24×24 / 32×32 multi-day matches: the map and setting are saved
//!   as served, and every agent's plan for every day equals the `POST /`
//!   body the server received;
//! - accepted, rejected, no-verdict-then-resent and unanswered days are told
//!   apart in the record;
//! - recording on and off submit byte-identical plans;
//! - a record directory that stops being writable costs records, not days;
//! - a restarted process continues the match in a new directory that points
//!   at the earlier one, and an unrelated match is not linked;
//! - the token appears in no file.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use cli::arena::Secret;
use cli::coordinator_loop::{
    CoordinatorLoopConfig, CoordinatorLoopError, CoordinatorOutcome, run_coordinator_loop,
};
use cli::match_record::{
    DayStatus, MatchRecorder, RecordEvent, RecorderOptions, SubmitOutcome, load_record,
};
use cli::solver_adapter::EngineLoopAdapter;
use protocol::http_client::{MatchClient, RetryPolicy};
use serde_json::{Value, json};
use solver_loop::InitialStrategy;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

const TOKEN: &str = "tok-394-SECRET-do-not-log";
const FLUSH: Duration = Duration::from_secs(10);

/// What the server does with one `POST /`.
#[derive(Debug, Clone, Copy)]
enum Post {
    /// `200 {"revision": n}` with the next non-negative revision.
    Accept,
    /// `200 {"revision": -1}`.
    Reject,
    /// `500`, and the day does not advance.
    ServerError,
}

#[derive(Clone)]
struct Server {
    size: i32,
    agents: Vec<i32>,
    days: usize,
    /// Seconds each day stays open; `0` serves `endsAt = 0` (no waiting).
    day_seconds: i64,
    starts_at: Arc<AtomicI64>,
    day: Arc<AtomicI32>,
    revision: Arc<AtomicI32>,
    /// Scripted `POST /` behaviour; `Accept` once it runs out.
    script: Arc<Mutex<VecDeque<Post>>>,
    /// `GET /` answers `401` (echoing the token) from this day on.
    auth_fail_from_day: Arc<AtomicI32>,
    served_states: Arc<Mutex<Vec<Value>>>,
    received_plans: Arc<Mutex<Vec<Vec<Vec<i32>>>>>,
    accepted_plans: Arc<Mutex<Vec<Vec<Vec<i32>>>>>,
}

impl Server {
    fn new(size: i32, n_agents: i32, days: usize) -> Self {
        // Agents start on the plain top row, spread out.
        let agents = (0..n_agents).map(|i| 1 + i * 2).collect();
        Self {
            size,
            agents,
            days,
            day_seconds: 0,
            starts_at: Arc::new(AtomicI64::new(1)),
            day: Arc::new(AtomicI32::new(0)),
            revision: Arc::new(AtomicI32::new(0)),
            script: Arc::new(Mutex::new(VecDeque::new())),
            auth_fail_from_day: Arc::new(AtomicI32::new(i32::MAX)),
            served_states: Arc::new(Mutex::new(Vec::new())),
            received_plans: Arc::new(Mutex::new(Vec::new())),
            accepted_plans: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Terrain with all four official codes: plain, a road column, a few
    /// mountains and ponds — so "the codes are stored as served" is a real
    /// claim, not an all-zero coincidence.
    fn cells(&self) -> Vec<Vec<i32>> {
        (0..self.size)
            .map(|r| {
                (0..self.size)
                    .map(|c| {
                        if r == 0 {
                            0
                        } else if c == self.size / 2 {
                            1
                        } else if r % 5 == 3 && c % 7 == 2 {
                            2
                        } else if r % 6 == 4 && c % 9 == 5 {
                            3
                        } else {
                            0
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn setting(&self) -> Value {
        let n = self.size * self.size;
        let spots: Vec<Value> = (0..6)
            .map(|i| {
                let pos = (self.size * (2 + i * 2) + 3 + i) % n;
                json!({ "brand": i % 3, "pos": pos, "stocks": 2 })
            })
            .collect();
        json!({
            "startsAt": self.starts_at.load(Ordering::SeqCst),
            "daySeconds": vec![30; self.days],
            "daySteps": vec![20; self.days],
            "map": { "height": self.size, "width": self.size, "cells": self.cells() },
            "spots": spots,
            "agents": self.agents,
            "fuelLimits": 40,
            "players": 3,
            "busyThreshold": 3,
            "jammedThreshold": 5,
        })
    }
}

fn unauthorized() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "code": "AuthError", "message": format!("bad token {TOKEN}") })),
    )
}

async fn serve_setting(State(s): State<Server>) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(s.setting()))
}

async fn serve_agent(State(_): State<Server>, _body: Json<Value>) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({})))
}

async fn serve_state(State(s): State<Server>) -> (StatusCode, Json<Value>) {
    let day = s.day.load(Ordering::SeqCst);
    if day >= s.auth_fail_from_day.load(Ordering::SeqCst) {
        return unauthorized();
    }
    let ends_at =
        if s.day_seconds == 0 { 0 } else { chrono::Utc::now().timestamp() + s.day_seconds };
    let agents: Vec<Value> = s
        .agents
        .iter()
        .enumerate()
        .map(|(i, pos)| json!({ "kind": i32::from(i == 0), "pos": pos, "fuel": 40 - day }))
        .collect();
    let state = json!({
        "endsAt": ends_at,
        "day": day,
        "agents": agents,
        "others": [
            { "id": 1, "agents": [{ "kind": 0, "pos": s.size * 3 + 1, "fuel": 31 }] },
            { "id": 2, "agents": [{ "kind": 1, "pos": s.size * 4 + 2, "fuel": 17 }] },
        ],
        "traffics": [
            { "pos": s.size + s.size / 2, "status": 1 },
            { "pos": s.size * 2 + s.size / 2, "status": 2 },
        ],
    });
    let mut served = s.served_states.lock().await;
    if served.last().and_then(|v: &Value| v.get("day")) != Some(&json!(day)) {
        served.push(state.clone());
    }
    (StatusCode::OK, Json(state))
}

async fn serve_action(
    State(s): State<Server>,
    Json(body): Json<Vec<Vec<i32>>>,
) -> (StatusCode, Json<Value>) {
    s.received_plans.lock().await.push(body.clone());
    let behaviour = s.script.lock().await.pop_front().unwrap_or(Post::Accept);
    match behaviour {
        Post::ServerError => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({}))),
        Post::Reject => {
            s.day.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, Json(json!({ "revision": -1 })))
        }
        Post::Accept => {
            s.accepted_plans.lock().await.push(body);
            s.day.fetch_add(1, Ordering::SeqCst);
            let revision = s.revision.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, Json(json!({ "revision": revision })))
        }
    }
}

async fn spawn(server: &Server) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let router = Router::new()
        .route("/setting", get(serve_setting))
        .route("/agent", post(serve_agent))
        .route("/", get(serve_state).post(serve_action))
        .with_state(server.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    format!("http://{addr}")
}

fn recorder(root: &Path) -> MatchRecorder {
    MatchRecorder::start(
        root,
        RecorderOptions {
            manifest: serde_json::Map::new(),
            secrets: vec![Secret::new(TOKEN.to_owned())],
            binary_path: None,
        },
    )
    .expect("recorder starts")
}

/// Run one match with the production `loop` solver.
async fn play(
    url: &str,
    recorder: MatchRecorder,
    resume: bool,
) -> Result<CoordinatorOutcome, CoordinatorLoopError> {
    let client = MatchClient::builder(url, TOKEN)
        // No retries inside the client, so every `POST /` the server sees is
        // one attempt of the loop and the two counts can be compared.
        .retry_policy(RetryPolicy { max_retries: 0, backoff_millis: &[] })
        .request_timeout(Duration::from_secs(2))
        .build()
        .expect("client");
    let adapter = EngineLoopAdapter::new(InitialStrategy::GreedyNearestNeighbor, 32);
    let cfg = CoordinatorLoopConfig {
        team_id: "team-394".to_owned(),
        setting_poll_interval: Duration::from_millis(20),
        solver_name: "loop".to_owned(),
        server_url: Some(url.to_owned()),
        engine_setting_slot: Some(adapter.setting_handle()),
        resume_running_match: resume,
        recorder,
        ..Default::default()
    };
    run_coordinator_loop(client, Box::new(adapter), cfg).await
}

fn assert_no_token(root: &Path) {
    let mut stack = vec![root.to_path_buf()];
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files += 1;
                let bytes = std::fs::read(&path).expect("read file");
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains(TOKEN), "{} contains the token", path.display());
            }
        }
    }
    assert!(files > 0, "nothing was recorded under {}", root.display());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_matches_on_16_24_32_boards_are_recorded_as_played() {
    for (size, n_agents, days) in [(16, 4, 5), (24, 5, 4), (32, 8, 6)] {
        let server = Server::new(size, n_agents, days);
        let url = spawn(&server).await;
        let root = tempfile::tempdir().expect("tempdir");
        let rec = recorder(root.path());

        let outcome = play(&url, rec.clone(), false).await.expect("match completes");
        assert_eq!(outcome.days_completed, days, "{size}x{size}");
        assert!(rec.flush(FLUSH));
        assert_eq!(rec.health().dropped + rec.health().write_errors, 0);

        let dir = rec.dir().expect("dir");
        for name in
            ["manifest.json", "setting.json", "events.jsonl", "coordinator.log", "state.json"]
        {
            // coordinator.log only exists when a logger is attached (the
            // binary's job); every other file must be there.
            assert!(
                dir.join(name).is_file() || name == "coordinator.log",
                "{size}x{size}: {name} is missing"
            );
        }

        // setting.json is the served body: same terrain codes, same spots.
        let saved: Value =
            serde_json::from_slice(&std::fs::read(dir.join("setting.json")).expect("setting"))
                .expect("setting parses");
        assert_eq!(saved, server.setting(), "{size}x{size}: setting.json differs from the wire");
        let cells = saved["map"]["cells"].as_array().expect("cells");
        assert_eq!(cells.len(), usize::try_from(size).expect("size"));
        let codes: std::collections::BTreeSet<i64> = cells
            .iter()
            .flat_map(|row| row.as_array().expect("row"))
            .filter_map(Value::as_i64)
            .collect();
        assert_eq!(codes.into_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3]);

        let loaded = load_record(dir).expect("record loads");
        assert_eq!(loaded.unparsed_lines, 0);
        assert_eq!(loaded.missing_seq(), 0);

        // Every day, every agent: the recorded plan is the body the server got.
        let received = server.received_plans.lock().await.clone();
        let plans = loaded.plans();
        assert_eq!(plans.len(), days);
        for (day, body) in received.iter().enumerate() {
            let recorded = plans[&u32::try_from(day).expect("day")];
            assert_eq!(recorded, body, "{size}x{size} day {day}: plan differs from the POST body");
            assert_eq!(recorded.len(), usize::try_from(n_agents).expect("agents"));
        }

        // Every day's observation is the GET / body, others and traffic included.
        let served = server.served_states.lock().await.clone();
        let observed = loaded.observed_days();
        assert_eq!(observed.len(), days);
        for state in &served {
            let day = i32::try_from(state["day"].as_i64().expect("day")).expect("day fits");
            assert_eq!(&serde_json::to_value(observed[&day]).expect("to_value"), state);
        }

        // Verdicts: accepted with the revision the server returned.
        let results = loaded.day_results();
        for day in 0..days {
            let day = u32::try_from(day).expect("day");
            let revision = i32::try_from(day).expect("revision");
            assert_eq!(results[&day], (DayStatus::Accepted, Some(revision)));
        }

        let manifest = &loaded.manifest;
        assert_eq!(manifest["match"]["map_width"], json!(size));
        assert_eq!(manifest["match"]["days"], json!(days));
        assert_eq!(
            manifest["agent_kinds"]["kinds"].as_array().expect("kinds").len(),
            outcome.kinds.len()
        );
        assert_eq!(manifest["result"]["status"], json!("completed"));
        assert_eq!(manifest["os"], json!(std::env::consts::OS));

        // state.json keeps the existing `--state-output` schema.
        let state: Value =
            serde_json::from_slice(&std::fs::read(dir.join("state.json")).expect("state"))
                .expect("state parses");
        assert_eq!(state["schemaVersion"], json!(1));
        assert_eq!(state["status"], json!("completed"));
        assert_eq!(state["revisions"].as_array().expect("revisions").len(), days);

        // The saved setting goes straight into the offline evaluator.
        let case = cli::arena_import::case_from_replay_file(&dir.join("setting.json"), Some("rec"))
            .expect("setting.json imports as an evaluator case");
        assert_eq!(case.setting.map.width, size);

        let summary = loaded.summary();
        assert_eq!(summary["days"].as_array().expect("days").len(), days);
        assert_no_token(root.path());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recording_on_and_off_submit_identical_plans() {
    let mut bodies = Vec::new();
    for record in [false, true] {
        let server = Server::new(24, 5, 5);
        let url = spawn(&server).await;
        let root = tempfile::tempdir().expect("tempdir");
        let rec = if record { recorder(root.path()) } else { MatchRecorder::disabled() };
        let outcome = play(&url, rec.clone(), false).await.expect("match completes");
        assert!(rec.flush(FLUSH));
        if !record {
            assert_eq!(std::fs::read_dir(root.path()).expect("read_dir").count(), 0);
        }
        bodies.push((server.received_plans.lock().await.clone(), outcome));
    }
    assert_eq!(bodies[0].0, bodies[1].0, "recording changed what was submitted");
    assert_eq!(bodies[0].1, bodies[1].1, "recording changed the outcome");
    assert_eq!(bodies[0].0.len(), 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resends_rejections_and_verdicts_are_told_apart() {
    let mut server = Server::new(16, 4, 3);
    server.day_seconds = 4;
    // Day 0: two 500s, then accepted. Day 1: rejected. Day 2: accepted.
    *server.script.lock().await =
        VecDeque::from([Post::ServerError, Post::ServerError, Post::Accept, Post::Reject]);
    let url = spawn(&server).await;
    let root = tempfile::tempdir().expect("tempdir");
    let rec = recorder(root.path());

    let result = play(&url, rec.clone(), false).await;
    assert!(
        matches!(result, Err(CoordinatorLoopError::Rejected { day: 1, revision: -1 })),
        "got {result:?}"
    );
    assert!(rec.flush(FLUSH));
    let loaded = load_record(rec.dir().expect("dir")).expect("load");

    let outcomes = |day: u32| -> Vec<(u32, SubmitOutcome, Option<i32>)> {
        loaded
            .submit_attempts(day)
            .into_iter()
            .filter_map(|event| match event {
                RecordEvent::SubmitAttempt { attempt, outcome, revision, .. } => {
                    Some((*attempt, *outcome, *revision))
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        outcomes(0),
        vec![
            (1, SubmitOutcome::NoVerdict, None),
            (2, SubmitOutcome::NoVerdict, None),
            (3, SubmitOutcome::Accepted, Some(0)),
        ]
    );
    assert_eq!(outcomes(1), vec![(1, SubmitOutcome::Rejected, Some(-1))]);
    assert_eq!(outcomes(2), vec![(1, SubmitOutcome::Accepted, Some(1))]);

    let results = loaded.day_results();
    assert_eq!(results[&0], (DayStatus::Accepted, Some(0)));
    assert_eq!(results[&1], (DayStatus::Rejected, Some(-1)));
    assert_eq!(results[&2], (DayStatus::Accepted, Some(1)));

    // The resent plan is the same plan: one `plan` record, three deliveries.
    let received = server.received_plans.lock().await.clone();
    assert_eq!(received.len(), 5);
    assert_eq!(received[0], received[1]);
    assert_eq!(received[1], received[2]);
    assert_eq!(loaded.plans()[&0], &received[0]);
    // The rejected plan is kept in full too.
    assert_eq!(loaded.plans()[&1], &received[3]);
    assert_eq!(loaded.manifest["result"]["status"], json!("rejected"));
    assert_no_token(root.path());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unwritable_record_directory_costs_records_not_days() {
    let baseline = Server::new(16, 4, 4);
    let url = spawn(&baseline).await;
    play(&url, MatchRecorder::disabled(), false).await.expect("baseline completes");
    let expected = baseline.received_plans.lock().await.clone();

    let server = Server::new(16, 4, 4);
    let url = spawn(&server).await;
    let root = tempfile::tempdir().expect("tempdir");
    let rec = recorder(root.path());
    let dir = rec.dir().expect("dir").to_path_buf();
    assert!(rec.flush(FLUSH));
    // The run directory becomes a file: every write from here on fails.
    std::fs::remove_dir_all(&dir).expect("remove");
    std::fs::write(&dir, b"disk gone").expect("block");

    let outcome = play(&url, rec.clone(), false).await.expect("the match still completes");
    assert_eq!(outcome.days_completed, 4);
    assert_eq!(server.received_plans.lock().await.clone(), expected);
    assert!(rec.flush(FLUSH), "the writer survives the failures");
    assert!(rec.health().write_errors > 0, "the failure must be counted, not hidden");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restarted_process_continues_the_match_in_a_linked_directory() {
    let root = tempfile::tempdir().expect("tempdir");

    // An unrelated earlier match in the same record root (another board).
    let other = Server::new(24, 5, 2);
    let other_url = spawn(&other).await;
    let other_rec = recorder(root.path());
    play(&other_url, other_rec.clone(), false).await.expect("other match completes");
    assert!(other_rec.flush(FLUSH));

    // First process: plays days 0 and 1, then dies (the server starts
    // answering 401, echoing the token, which ends the loop).
    let server = Server::new(16, 4, 5);
    server.starts_at.store(1_800_000_000, Ordering::SeqCst);
    server.auth_fail_from_day.store(2, Ordering::SeqCst);
    let url = spawn(&server).await;
    let first = recorder(root.path());
    let result = play(&url, first.clone(), true).await;
    assert!(matches!(result, Err(CoordinatorLoopError::State(_))), "got {result:?}");
    assert!(first.flush(FLUSH));

    // Second process: same server, match now on day 2.
    server.auth_fail_from_day.store(i32::MAX, Ordering::SeqCst);
    let second = recorder(root.path());
    let outcome = play(&url, second.clone(), true).await.expect("resumed match completes");
    assert_eq!(outcome.days_completed, 3);
    assert!(second.flush(FLUSH));

    let a = load_record(first.dir().expect("dir")).expect("first loads");
    let b = load_record(second.dir().expect("dir")).expect("second loads");
    assert_ne!(first.run_id(), second.run_id(), "each start gets its own directory");
    assert_eq!(a.plans().keys().copied().collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(b.plans().keys().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
    assert_eq!(a.manifest["result"]["status"], json!("error"));
    assert!(a.manifest.get("continues_run_id").is_none());
    assert_eq!(b.manifest["resumed_at_day"], json!(2));
    assert_eq!(b.manifest["continues_run_id"], json!(first.run_id().expect("run id")));
    assert!(b.lines.iter().any(|l| matches!(
        &l.event,
        RecordEvent::Continues { previous_run_id, matched_by }
            if Some(previous_run_id.as_str()) == first.run_id() && matched_by == "starts_at"
    )));

    // Together the two directories hold the whole match, in order.
    let mut all: Vec<Vec<Vec<i32>>> = a.plans().values().map(|p| (*p).clone()).collect();
    all.extend(b.plans().values().map(|p| (*p).clone()));
    assert_eq!(all, server.accepted_plans.lock().await.clone());

    // The unrelated match stays unrelated.
    let c = load_record(other_rec.dir().expect("dir")).expect("other loads");
    assert!(c.manifest.get("continues_run_id").is_none());
    assert_eq!(c.plans().len(), 2);

    // The 401 body carried the token; no file may.
    assert_no_token(root.path());
}
