//! End-to-end checks of the HTTP log (`--http-log-dir`, Issue #396).
//!
//! A real axum server answers with the statuses a contest day can produce —
//! 200, 400, 401, 500, a stalled response, an HTML page where JSON is due —
//! and a closed port stands in for a dead link. Every test reads back what
//! the log printed (the console sink) and what it stored (`http.jsonl`) and
//! compares both with what the server actually saw.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use cli::arena::Secret;
use cli::coordinator_loop::{CoordinatorLoopConfig, CoordinatorOutcome, run_coordinator_loop};
use cli::http_log::{HttpLog, HttpLogEntry, load_http_log, render};
use cli::solver_adapter::EngineLoopAdapter;
use protocol::http_client::{HttpClientError, MatchClient, RetryPolicy};
use serde_json::{Value, json};
use solver_loop::InitialStrategy;
use tokio::net::TcpListener;

const TOKEN: &str = "tok-396-SECRET-do-not-log";
const FLUSH: Duration = Duration::from_secs(10);

/// Console sink the test can read back.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("sink").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("sink")).into_owned()
    }
}

#[derive(Clone, Default)]
struct Seen {
    /// Every request the server received: method, path, body.
    requests: Arc<Mutex<Vec<(String, String, String)>>>,
    day: Arc<Mutex<i32>>,
}

impl Seen {
    fn note(&self, method: &str, path: &str, body: &str) {
        self.requests.lock().expect("seen").push((
            method.to_owned(),
            path.to_owned(),
            body.to_owned(),
        ));
    }
}

fn setting() -> Value {
    json!({
        "startsAt": 1,
        "daySeconds": [30, 30, 30],
        "daySteps": [20, 20, 20],
        "map": { "height": 16, "width": 16, "cells": vec![vec![0; 16]; 16] },
        "spots": [{ "brand": 0, "pos": 40, "stocks": 2 }, { "brand": 1, "pos": 99, "stocks": 2 }],
        "agents": [1, 3, 5, 7],
        "fuelLimits": 40,
        "players": 2,
        "busyThreshold": 3,
        "jammedThreshold": 5,
    })
}

async fn serve_setting(State(s): State<Seen>) -> Json<Value> {
    s.note("GET", "/setting", "");
    Json(setting())
}

async fn serve_agent(State(s): State<Seen>, body: String) -> Json<Value> {
    s.note("POST", "/agent", &body);
    Json(json!({}))
}

async fn serve_state(State(s): State<Seen>) -> Json<Value> {
    s.note("GET", "/", "");
    let day = *s.day.lock().expect("day");
    let agents: Vec<Value> =
        [1, 3, 5, 7].iter().map(|pos| json!({ "kind": 0, "pos": pos, "fuel": 40 })).collect();
    Json(json!({ "endsAt": 0, "day": day, "agents": agents, "others": [], "traffics": [] }))
}

async fn serve_action(State(s): State<Seen>, body: String) -> Json<Value> {
    s.note("POST", "/", &body);
    let mut day = s.day.lock().expect("day");
    *day += 1;
    Json(json!({ "revision": *day }))
}

async fn bad(State(s): State<Seen>) -> impl IntoResponse {
    s.note("GET", "/bad", "");
    (StatusCode::BAD_REQUEST, Json(json!({ "message": "malformed" })))
}

/// 401 whose body and headers repeat the token the client sent.
async fn unauth(State(s): State<Seen>, headers: HeaderMap) -> impl IntoResponse {
    s.note("GET", "/unauth", "");
    let token = headers.get("Procon-Token").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::SET_COOKIE, format!("session={token}")),
            (header::HeaderName::from_static("x-echo"), format!("you sent {token}")),
        ],
        Json(json!({ "code": "AuthError", "message": format!("bad token {token}") })),
    )
}

async fn boom(State(s): State<Seen>) -> impl IntoResponse {
    s.note("GET", "/boom", "");
    (StatusCode::INTERNAL_SERVER_ERROR, "database on fire")
}

async fn slow(State(s): State<Seen>) -> impl IntoResponse {
    s.note("GET", "/slow", "");
    tokio::time::sleep(Duration::from_secs(3)).await;
    Json(json!({}))
}

async fn html(State(s): State<Seen>) -> impl IntoResponse {
    s.note("GET", "/html", "");
    ([(header::CONTENT_TYPE, "application/json")], "<html>502 Bad Gateway</html>")
}

async fn spawn(seen: &Seen) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let router = Router::new()
        .route("/setting", get(serve_setting))
        .route("/agent", post(serve_agent))
        .route("/", get(serve_state).post(serve_action))
        .route("/bad", get(bad))
        .route("/unauth", get(unauth))
        .route("/boom", get(boom))
        .route("/slow", get(slow))
        .route("/html", get(html))
        .with_state(seen.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    format!("http://{addr}")
}

struct Logged {
    log: Arc<HttpLog>,
    sink: Sink,
    _root: tempfile::TempDir,
}

fn start_log() -> Logged {
    let root = tempfile::tempdir().expect("tempdir");
    let sink = Sink::default();
    let log = HttpLog::start(
        &root.path().join("logs"),
        "run-396",
        vec![Secret::new(TOKEN.to_owned())],
        Box::new(sink.clone()),
    )
    .expect("log starts");
    Logged { log, sink, _root: root }
}

fn client(url: &str, logged: Option<&Logged>, retries: usize) -> MatchClient {
    let mut builder = MatchClient::builder(url, TOKEN)
        .retry_policy(RetryPolicy { max_retries: retries, backoff_millis: &[1, 1, 1, 1] })
        .request_timeout(Duration::from_millis(400));
    if let Some(logged) = logged {
        builder = builder.observer(logged.log.clone());
    }
    builder.build().expect("client")
}

fn entries(logged: &Logged) -> Vec<HttpLogEntry> {
    assert!(logged.log.flush(FLUSH));
    assert_eq!(logged.log.lost(), (0, 0));
    let (entries, unparsed) = load_http_log(logged.log.path()).expect("http.jsonl loads");
    assert_eq!(unparsed, 0);
    entries
}

/// The console shows exactly the stored entries, in order, in full.
fn assert_console_matches_file(logged: &Logged, entries: &[HttpLogEntry]) {
    let expected: String = entries.iter().map(render).collect();
    assert_eq!(logged.sink.text(), expected, "console and http.jsonl differ");
}

fn assert_no_token(logged: &Logged) {
    let file = std::fs::read_to_string(logged.log.path()).expect("read log");
    for (what, text) in [("http.jsonl", file), ("console", logged.sink.text())] {
        assert!(!text.contains(TOKEN), "{what} contains the token");
        assert!(!text.contains("SECRET"), "{what} contains part of the token");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_status_and_failure_is_logged_as_it_happened() {
    let seen = Seen::default();
    let url = spawn(&seen).await;
    let logged = start_log();
    let c = client(&url, Some(&logged), 2);

    // 200
    let ok: Value = c.get_json("/setting").await.expect("200");
    assert_eq!(ok, setting());
    // 400: not transient, one request
    assert!(matches!(
        c.get_json::<Value>("/bad").await,
        Err(HttpClientError::Http { status: 400, .. })
    ));
    // 401
    assert!(matches!(c.get_json::<Value>("/unauth").await, Err(HttpClientError::Auth { .. })));
    // 500: transient, repeated twice, then surfaced
    assert!(matches!(
        c.get_json::<Value>("/boom").await,
        Err(HttpClientError::Http { status: 500, .. })
    ));
    // timeout on a server that stalls (no retries, to keep the test short)
    let c0 = client(&url, Some(&logged), 0);
    assert!(matches!(c0.get_json::<Value>("/slow").await, Err(HttpClientError::Transport(_))));
    // 200 with a body that is not JSON
    assert!(matches!(c.get_json::<Value>("/html").await, Err(HttpClientError::ApiDrift { .. })));
    // nothing listening
    let dead = {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        format!("http://{}", listener.local_addr().expect("addr"))
    };
    let cd = client(&dead, Some(&logged), 0);
    assert!(matches!(cd.get_json::<Value>("/setting").await, Err(HttpClientError::Transport(_))));

    let all = entries(&logged);
    let brief: Vec<(String, Option<u16>, Option<String>, u32)> =
        all.iter().map(|e| (e.kind.clone(), e.status, e.failure.clone(), e.attempt)).collect();
    let exchange = |status: u16, attempt: u32| ("exchange".to_owned(), Some(status), None, attempt);
    assert_eq!(
        brief,
        vec![
            exchange(200, 1),
            exchange(400, 1),
            exchange(401, 1),
            exchange(500, 1),
            exchange(500, 2),
            exchange(500, 3),
            ("exchange".to_owned(), None, Some("timeout".to_owned()), 1),
            exchange(200, 1),
            ("decode_error".to_owned(), Some(200), Some("decode".to_owned()), 1),
            ("exchange".to_owned(), None, Some("connect".to_owned()), 1),
        ]
    );
    // Request numbers count requests; the decode error points at its request.
    assert_eq!(all.iter().map(|e| e.n).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6, 7, 8, 8, 9]);

    let text = |i: usize| all[i].status_text.clone().unwrap_or_default();
    assert_eq!(text(0), "OK");
    assert_eq!(text(1), "Bad Request");
    assert_eq!(text(2), "Unauthorized");
    assert_eq!(text(3), "Internal Server Error");

    // Bodies are what the server sent, whole.
    assert_eq!(
        serde_json::from_str::<Value>(all[0].response_body.as_deref().expect("body"))
            .expect("json"),
        setting()
    );
    assert_eq!(all[1].response_body.as_deref(), Some(r#"{"message":"malformed"}"#));
    assert_eq!(all[3].response_body.as_deref(), Some("database on fire"));
    assert_eq!(all[7].response_body.as_deref(), Some("<html>502 Bad Gateway</html>"));
    // A failure has no invented status or body.
    assert_eq!(all[6].result, "failed");
    assert!(all[6].response_body.is_none() && all[6].error.is_some());
    assert_eq!(all[9].result, "failed");
    assert!(all[9].url.starts_with(&dead));
    assert!(all[8].error.as_deref().unwrap_or("").contains("API drift"));
    // The log has one exchange per request the server received (the dead
    // port received none).
    assert_eq!(seen.requests.lock().expect("seen").len(), 8);

    let console = logged.sink.text();
    for needle in [
        "-> 200 OK",
        "-> 400 Bad Request",
        "-> 401 Unauthorized",
        "-> 500 Internal Server Error",
        "-> FAILED timeout",
        "-> FAILED connect",
        "DECODE ERROR",
        "attempt=3",
        "> Procon-Token: [MASKED]",
    ] {
        assert!(console.contains(needle), "console lacks {needle:?}:\n{console}");
    }
    assert_console_matches_file(&logged, &all);

    // The 401 echoed the token in a cookie, a header and the body.
    assert_eq!(
        all[2].response_headers.iter().find(|(n, _)| n == "set-cookie").map(|(_, v)| v.as_str()),
        Some("[MASKED]")
    );
    assert!(all[2].response_body.as_deref().expect("body").contains("[REDACTED]"));
    assert_no_token(&logged);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_large_request_body_is_logged_whole() {
    let seen = Seen::default();
    let url = spawn(&seen).await;
    let logged = start_log();
    let plan: Vec<Vec<i32>> =
        (0..8).map(|agent| (0..400).map(|step| (agent + step) % 6).collect()).collect();
    let accepted = client(&url, Some(&logged), 0).submit_actions(plan.clone()).await.expect("POST");
    assert_eq!(accepted.revision, 1);

    let all = entries(&logged);
    assert_eq!(all.len(), 1);
    let sent = serde_json::to_string(&plan).expect("json");
    assert_eq!(all[0].method, "POST");
    assert_eq!(all[0].request_body.as_deref(), Some(sent.as_str()));
    // …and it is what the server received.
    assert_eq!(seen.requests.lock().expect("seen")[0].2, sent);
    assert_eq!(all[0].response_body.as_deref(), Some(r#"{"revision":1}"#));
    assert!(logged.sink.text().contains(&format!("  > body: {sent}\n")));
    assert_console_matches_file(&logged, &all);
    assert_no_token(&logged);
}

async fn play(url: &str, logged: Option<&Logged>) -> CoordinatorOutcome {
    let adapter = EngineLoopAdapter::new(InitialStrategy::GreedyNearestNeighbor, 32);
    let cfg = CoordinatorLoopConfig {
        team_id: "team-396".to_owned(),
        setting_poll_interval: Duration::from_millis(20),
        solver_name: "loop".to_owned(),
        engine_setting_slot: Some(adapter.setting_handle()),
        ..Default::default()
    };
    run_coordinator_loop(client(url, logged, 1), Box::new(adapter), cfg)
        .await
        .expect("match completes")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_match_is_the_same_with_and_without_the_log() {
    let plain = Seen::default();
    let plain_url = spawn(&plain).await;
    let outcome_plain = play(&plain_url, None).await;

    let seen = Seen::default();
    let url = spawn(&seen).await;
    let logged = start_log();
    let outcome_logged = play(&url, Some(&logged)).await;

    assert_eq!(outcome_plain, outcome_logged, "the log changed the outcome");
    let requests = seen.requests.lock().expect("seen").clone();
    assert_eq!(*plain.requests.lock().expect("seen"), requests, "the log changed the traffic");

    // One entry per request, same method, path and body, in order.
    let all = entries(&logged);
    assert_eq!(all.len(), requests.len());
    for (entry, (method, path, body)) in all.iter().zip(&requests) {
        assert_eq!(&entry.method, method);
        assert_eq!(entry.url, format!("{url}{}", if path == "/" { "/" } else { path }));
        assert_eq!(entry.request_body.clone().unwrap_or_default(), *body);
        assert_eq!(entry.status, Some(200));
        assert_eq!(entry.run_id, "run-396");
    }
    // Requests are tied to the day they belong to.
    let posts: Vec<Option<i64>> =
        all.iter().filter(|e| e.method == "POST" && e.url.ends_with('/')).map(|e| e.day).collect();
    assert_eq!(posts, vec![Some(0), Some(1), Some(2)]);
    assert_eq!(all[0].day, None, "GET /setting comes before any day");
    assert_console_matches_file(&logged, &all);
    assert_no_token(&logged);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_log_that_cannot_be_written_does_not_disturb_requests() {
    let seen = Seen::default();
    let url = spawn(&seen).await;
    let logged = start_log();
    let path: &Path = logged.log.path();
    let dir = path.parent().expect("dir").to_path_buf();
    std::fs::remove_dir_all(&dir).expect("remove");
    std::fs::write(&dir, b"in the way").expect("block");

    let c = client(&url, Some(&logged), 0);
    for _ in 0..3 {
        let ok: Value = c.get_json("/setting").await.expect("requests still work");
        assert_eq!(ok, setting());
    }
    assert!(logged.log.flush(FLUSH));
    assert!(logged.log.lost().1 >= 1, "failed writes are counted");
    // The console still shows every exchange, and says the file failed.
    let console = logged.sink.text();
    assert_eq!(console.matches("-> 200 OK").count(), 3);
    assert!(console.contains("http-log: writing"));
}

/// A server that sends a status line and headers, promises more body than it
/// delivers, and hangs up. Returns its URL and the number of connections it
/// has accepted.
async fn spawn_truncating(status_line: &'static str) -> (String, Arc<Mutex<u32>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let connections = Arc::new(Mutex::new(0_u32));
    let counter = connections.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            *counter.lock().expect("counter") += 1;
            tokio::spawn(async move {
                let mut request = [0_u8; 4096];
                let _ = socket.read(&mut request).await;
                let head = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: 500\r\n\r\n{{\"day\":"
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (url, connections)
}

/// A response body that breaks off must be handled the same with and without
/// the log: the log reads bodies earlier than the decoders do, and that must
/// not turn into extra requests or a different verdict.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_that_breaks_off_is_handled_the_same_with_and_without_the_log() {
    // 200 on a JSON endpoint: a transport error, and no second request.
    for with_log in [false, true] {
        let (url, connections) = spawn_truncating("HTTP/1.1 200 OK").await;
        let logged = with_log.then(start_log);
        let result = client(&url, logged.as_ref(), 3).get_match_state().await;
        assert!(matches!(result, Err(HttpClientError::Transport(_))), "log={with_log}: {result:?}");
        assert_eq!(
            *connections.lock().expect("counter"),
            1,
            "log={with_log}: the GET was repeated"
        );
        if let Some(logged) = logged.as_ref() {
            let all = entries(logged);
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].result, "failed");
            assert_eq!(all[0].failure.as_deref(), Some("body"));
            assert_eq!(all[0].status, Some(200), "the status that did arrive is kept");
            assert!(logged.sink.text().contains("-> FAILED body after status 200 OK"));
        }
    }

    // 200 on POST /agent, whose body nobody reads: still a success.
    for with_log in [false, true] {
        let (url, connections) = spawn_truncating("HTTP/1.1 200 OK").await;
        let logged = with_log.then(start_log);
        let kinds = vec![protocol::match_config::AgentKind::KIND_0; 4];
        let result = client(&url, logged.as_ref(), 3).submit_agent(kinds).await;
        assert!(result.is_ok(), "log={with_log}: {result:?}");
        assert_eq!(*connections.lock().expect("counter"), 1);
    }

    // 503: the status drives the retries, as it does without the log.
    for with_log in [false, true] {
        let (url, connections) = spawn_truncating("HTTP/1.1 503 Service Unavailable").await;
        let logged = with_log.then(start_log);
        let result = client(&url, logged.as_ref(), 2).get_match_state().await;
        assert!(
            matches!(result, Err(HttpClientError::Http { status: 503, .. })),
            "log={with_log}: {result:?}"
        );
        assert_eq!(*connections.lock().expect("counter"), 3, "log={with_log}: 1 try + 2 retries");
    }
}

/// A console that stops accepting output must not stop the file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stuck_console_does_not_stop_the_file() {
    struct Stuck(Arc<std::sync::atomic::AtomicBool>);
    impl Write for Stuck {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            while self.0.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let seen = Seen::default();
    let url = spawn(&seen).await;
    let root = tempfile::tempdir().expect("tempdir");
    let blocked = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let log = HttpLog::start(
        &root.path().join("logs"),
        "run-stuck",
        vec![Secret::new(TOKEN.to_owned())],
        Box::new(Stuck(blocked.clone())),
    )
    .expect("log starts");

    let c = MatchClient::builder(&url, TOKEN)
        .retry_policy(RetryPolicy { max_retries: 0, backoff_millis: &[] })
        .request_timeout(Duration::from_secs(2))
        .observer(log.clone())
        .build()
        .expect("client");
    for _ in 0..5 {
        let ok: Value = c.get_json("/setting").await.expect("requests are not held up");
        assert_eq!(ok, setting());
    }
    // The console is still blocked; the file must fill up regardless.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (entries, _) = load_http_log(log.path()).expect("http.jsonl");
        if entries.len() == 5 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the file waited for the console");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!log.flush(Duration::from_millis(100)), "flush reports the stuck console");
    blocked.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(log.flush(FLUSH), "and completes once the console is back");
}

/// A decode failure names its exchange by id, so it lands on the right
/// request even when another request was logged in between, and on none when
/// its own exchange never made it into the log.
#[test]
fn a_decode_error_is_attached_to_its_own_exchange() {
    use protocol::http_client::{HttpExchange, HttpObserver, HttpOutcome};

    let exchange = |id: u64, path: &str| HttpExchange {
        id,
        method: "GET".to_owned(),
        url: format!("http://server{path}"),
        request_headers: Vec::new(),
        request_body: None,
        attempt: 1,
        started_at: std::time::SystemTime::now(),
        elapsed: Duration::from_millis(3),
        outcome: HttpOutcome::Response {
            status: 200,
            headers: Vec::new(),
            body: b"<html>".to_vec(),
        },
    };
    let logged = start_log();
    logged.log.exchange(exchange(1, "/a"));
    logged.log.exchange(exchange(2, "/b"));
    logged.log.decode_failed(1, "not JSON");
    logged.log.decode_failed(99, "its exchange was never logged");
    assert!(logged.log.flush(FLUSH));

    let (all, unparsed) = load_http_log(logged.log.path()).expect("load");
    assert_eq!(unparsed, 0);
    assert_eq!(all.len(), 3);
    assert_eq!(
        (all[2].kind.as_str(), all[2].n, all[2].url.as_str()),
        ("decode_error", 1, "http://server/a")
    );
    assert_eq!(all[2].error.as_deref(), Some("not JSON"));
    assert_eq!(logged.log.lost().0, 1, "the orphan failure is counted, not misattributed");
}
