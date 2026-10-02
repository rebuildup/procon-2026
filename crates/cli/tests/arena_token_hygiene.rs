//! Token-hygiene audit for Issue #179.
//!
//! `PROCON_ARENA_TOKEN` may flow env → profile → coordinator config
//! (match header only). It must never surface in:
//!
//! - `Debug` / `Display` renderings of any config or profile,
//! - `tracing` output,
//! - error strings (server bodies are assumed hostile — they may echo
//!   material back at us),
//! - the `--state-output` snapshot (a durable artifact),
//! - the arena evidence JSONL (a durable artifact),
//! - `--dry-run` stdout or `--help` text.
//!
//! Every assertion below greps for the same sentinel value end to end.
//! No request in this file leaves `127.0.0.1`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Router, extract::State};
use serde_json::json;
use tokio::net::TcpListener;

use cli::arena::{self, ArenaProfile, ArenaRunRecord, ArenaRunStatus, RegistrationOptions, Secret};
use cli::coordinator_loop::{CoordinatorLoopConfig, run_coordinator_loop};
use cli::solver_adapter::WaitSolver;
use protocol::http_client::{MatchClient, RetryPolicy};

const SENTINEL: &str = "sentinel-arena-token-9f3c-never-log";

/// Behaviour of the single-endpoint fake: what `/setting` (the official
/// registration path, Issue #222) answers on every request. The plural
/// `/settings` route is wired to the same handler so a path regression
/// still exercises the same redaction logic.
#[derive(Clone)]
struct Mode {
    /// `register` — 200 JSON; `auth-echo` — 401 whose body echoes the
    /// sentinel; `drift-500` — 500 whose body echoes the sentinel.
    kind: &'static str,
}

async fn handler(State(mode): State<Mode>) -> Response {
    match mode.kind {
        "register" => {
            ([(header::CONTENT_TYPE, "application/json")], json!({"matchId": "m"}).to_string())
                .into_response()
        }
        "auth-echo" => {
            let body = json!({"code": "AuthError", "message": format!("bad token {SENTINEL}")});
            (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response()
        }
        // A hostile server echoing our token in an error body — the exact
        // case the redaction exists for.
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain")],
            format!("upstream exploded, token was {SENTINEL}"),
        )
            .into_response(),
    }
}

async fn spawn(kind: &'static str) -> String {
    let app = Router::new()
        .route("/settings", get(handler))
        .route("/setting", get(handler))
        .with_state(Mode { kind });
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
        .retry_policy(RetryPolicy { max_retries: 1, backoff_millis: &[0] })
        .request_timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

fn open_options() -> RegistrationOptions {
    RegistrationOptions {
        poll_interval: Duration::from_millis(10),
        window: Some(arena::MatchWindow {
            opens_at: chrono::Utc::now() - chrono::TimeDelta::seconds(2),
            starts_at: chrono::Utc::now() + chrono::TimeDelta::seconds(5),
        }),
        ..RegistrationOptions::default()
    }
}

// ---------------------------------------------------------------------------
// Renderings
// ---------------------------------------------------------------------------

#[test]
fn debug_renderings_of_profile_and_config_never_show_the_token() {
    let profile = profile("https://procon37arena.online".to_owned());
    let rendered = format!("{profile:?}");
    assert!(!rendered.contains(SENTINEL), "profile Debug leaked: {rendered}");
    assert!(rendered.contains("[REDACTED]"), "{rendered}");
    assert!(!format!("{}", profile.token).contains(SENTINEL));
    assert!(!format!("{:?}", profile.token).contains(SENTINEL));

    // The coordinator config carries the secret only as a redaction
    // pattern, and even that is redacted in its own Debug.
    let cfg = CoordinatorLoopConfig {
        secret_redaction: Some(Secret::new(SENTINEL.to_owned())),
        ..CoordinatorLoopConfig::default()
    };
    let rendered = format!("{cfg:?}");
    assert!(!rendered.contains(SENTINEL), "coordinator config Debug leaked: {rendered}");
}

#[test]
fn help_documents_the_env_var_but_offers_no_token_flag() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_procon"))
        .args(["coordinator", "--help"])
        .output()
        .expect("spawn procon");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PROCON_ARENA_TOKEN"), "env var documented: {stdout}");
    assert!(!stdout.contains("--arena-token"), "no token flag: {stdout}");
    assert!(!stdout.contains(SENTINEL), "token in help: {stdout}");
}

#[test]
fn dry_run_reports_only_a_boolean_never_the_token() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_procon"))
        .args([
            "coordinator",
            "--arena",
            "--dry-run",
            "--server",
            "http://127.0.0.1:1",
            "--token",
            SENTINEL,
        ])
        .output()
        .expect("spawn procon");
    assert!(output.status.success(), "dry run must not contact the server");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"has_token\":true"), "{stdout}");
    assert!(!stdout.contains(SENTINEL), "token in dry-run stdout: {stdout}");
}

// ---------------------------------------------------------------------------
// Errors and logs
// ---------------------------------------------------------------------------

/// Shared in-memory `tracing` writer for this test binary.
#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogSink {
    type Writer = LogSink;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl LogSink {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().expect("log lock").clone()).expect("utf8 logs")
    }
}

/// Install **one process-wide** capture subscriber before any registration
/// runs, and return the sink it writes to.
///
/// Why global: `tracing` caches per-callsite interest from whichever
/// dispatcher evaluates a callsite first. These tests exercise the same
/// callsites concurrently; letting one of them run against the default
/// (no-op) dispatcher can cache "never" and silently swallow the very
/// events this audit needs to read. A single global default — installed
/// through `OnceLock` before *any* registration starts — removes the race
/// and captures every test's logs in this binary (they are all token-free
/// by construction, which is exactly what we assert).
fn global_capture() -> LogSink {
    static SINK: std::sync::OnceLock<LogSink> = std::sync::OnceLock::new();
    SINK.get_or_init(|| {
        let sink = LogSink::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(sink.clone())
            .finish();
        // Nothing else in this binary installs a global default; a failure
        // here would simply mean someone else's subscriber owns the logs.
        let _ = tracing::subscriber::set_global_default(subscriber);
        sink.clone()
    })
    .clone()
}

#[test]
fn a_server_that_echoes_the_token_gets_redacted_error_messages() {
    // First thing: pin the global capture so no callsite in this binary is
    // ever evaluated against the no-op dispatcher (see `global_capture`).
    let _capture = global_capture();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    rt.block_on(async {
        let url = spawn("auth-echo").await;
        let err = arena::wait_for_registration(
            &client(&url),
            &profile(url),
            &open_options(),
            chrono::Utc::now(),
        )
        .await
        .expect_err("401 must fail registration");
        let text = err.to_string();
        assert!(!text.contains(SENTINEL), "token leaked into the error: {text}");
        assert!(text.contains("[REDACTED]"), "{text}");
        assert_eq!(err.status(), ArenaRunStatus::RegistrationFailed);
    });
}

/// Run a registration and read back everything `tracing` emitted — for the
/// presence of the registration log itself (proof the capture is live) and
/// for the absence of the sentinel token (the actual hygiene property).
#[test]
fn tracing_output_during_registration_never_contains_the_token() {
    let sink = global_capture();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    rt.block_on(async {
        let url = spawn("register").await;
        arena::wait_for_registration(
            &client(&url),
            &profile(url),
            &open_options(),
            chrono::Utc::now(),
        )
        .await
        .expect("registration succeeds");
    });

    let logs = sink.contents();
    assert!(
        logs.contains("registering for the arena match"),
        "the capture must actually have seen the registration log: {logs}"
    );
    assert!(!logs.contains(SENTINEL), "token leaked into logs: {logs}");
}

// ---------------------------------------------------------------------------
// Durable artifacts
// ---------------------------------------------------------------------------

/// A `403`-free failure path: `GET /setting` answers 500 with the token
/// echoed in the body. The coordinator writes a terminal `error` snapshot —
/// which must be scrubbed before it hits the disk.
#[test]
fn state_snapshots_redact_the_token_before_writing() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    rt.block_on(async {
        let url = spawn("drift-500").await;
        let dir = std::env::temp_dir().join(format!("procon-arena-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let snapshot = dir.join("state.json");

        let cfg = CoordinatorLoopConfig {
            team_id: "arena-team".to_owned(),
            setting_poll_interval: Duration::from_millis(10),
            solver_name: "wait".to_owned(),
            server_url: Some(url.clone()),
            state_output_path: Some(snapshot.clone()),
            secret_redaction: Some(Secret::new(SENTINEL.to_owned())),
            ..CoordinatorLoopConfig::default()
        };
        let client = MatchClient::builder(url, SENTINEL)
            .retry_policy(RetryPolicy { max_retries: 1, backoff_millis: &[0] })
            .request_timeout(Duration::from_secs(5))
            .build()
            .expect("client");
        let err = run_coordinator_loop(client, Box::new(WaitSolver), cfg)
            .await
            .expect_err("500 on GET /setting must fail the loop");

        // The raw error still carries the server body (it may be shown to
        // the operator through the redacted CLI wrapper)…
        assert!(
            err.to_string().contains(SENTINEL),
            "test precondition: the server really did echo the token"
        );

        // …but the durable snapshot never does.
        let written = std::fs::read_to_string(&snapshot).expect("snapshot written");
        assert!(!written.contains(SENTINEL), "token in state snapshot: {written}");
        assert!(written.contains("[REDACTED]"), "redaction marker missing: {written}");
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn evidence_records_redact_the_token_from_error_text() {
    let profile = profile("https://procon37arena.online".to_owned());
    let record = ArenaRunRecord::build(arena::ArenaRecordInput {
        profile: &profile,
        candidate_id: Some("astar:policy=step"),
        solver: arena::ArenaSolverConfig {
            mode: "candidate".to_owned(),
            solver: "astar".to_owned(),
            loop_initial_strategy: None,
            loop_iterations: None,
        },
        team_id: "arena-team",
        source_commit_sha: "deadbeef".to_owned(),
        window_starts_at_unix: 1_790_000_100,
        registration: None,
        outcome: None,
        status: ArenaRunStatus::RegistrationFailed,
        error: Some(format!("upstream exploded, token was {SENTINEL}")),
    });
    let json = serde_json::to_string(&record).expect("serialises");
    assert!(!json.contains(SENTINEL), "token in evidence: {json}");
    assert!(json.contains("[REDACTED]"), "{json}");
    assert!(!json.contains("PROCON_ARENA_TOKEN"), "{json}");
    // The schema itself has nowhere to put a secret.
    let value: serde_json::Value = serde_json::from_str(&json).expect("parses");
    for key in ["token", "email", "display_name", "account"] {
        assert!(value.get(key).is_none(), "evidence schema must not grow a {key} field");
    }
}
