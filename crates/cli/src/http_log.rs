//! HTTP exchange log (Issue #396).
//!
//! `procon coordinator --http-log-dir <DIR>` reports every HTTP request the
//! coordinator makes — one by one, retries included — to two places that
//! carry the same content:
//!
//! - **stderr**: a readable block per request (number, time, method, URL,
//!   status or failure, duration, headers and full bodies);
//! - **`<DIR>/<run-id>/http.jsonl`**: one JSON object per request.
//!
//! Only what was actually sent and received is written. A request without a
//! response is a `failed` entry with the transport error; nothing is filled
//! in from what the caller hoped for. Bodies are never shortened.
//!
//! The observer side ([`HttpLog`] as a [`protocol::http_client::HttpObserver`])
//! runs on the task that talks to the server, so it does one thing: a
//! `try_send` into a bounded queue. A writer thread formats, masks and
//! appends to the file (with `fsync` per entry), then hands the readable
//! copy to a third thread that owns the console. A console that stops
//! accepting output (a paused Windows console window) therefore delays
//! neither a request nor the file; a failing disk delays neither a request
//! nor the console. A full queue drops the entry and counts it.
//!
//! # Secrets
//!
//! The `Procon-Token` header value is masked inside the HTTP client. Here,
//! header values whose name looks like a credential are masked again, and
//! every registered token is replaced wherever it appears — URL, bodies,
//! header values, error text — before anything is printed or stored.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use protocol::http_client::{HttpExchange, HttpObserver, HttpOutcome, MASKED};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::arena::Secret;

/// Entries the client may have queued ahead of the writer.
const QUEUE_CAPACITY: usize = 4096;

/// Exchanges kept for matching a later decode failure.
const RECENT_EXCHANGES: usize = 64;

/// Header names (lowercase substrings) whose values are never shown.
const SECRET_HEADER_MARKERS: [&str; 7] =
    ["token", "authorization", "cookie", "api-key", "apikey", "secret", "password"];

/// One line of `http.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpLogEntry {
    /// `exchange` (one request) or `decode_error` (the response of request
    /// `n` could not be decoded).
    #[serde(rename = "type")]
    pub kind: String,
    /// The run this belongs to (shared with the match record when both are
    /// on).
    pub run_id: String,
    /// Request number within the run, from 1. A `decode_error` carries the
    /// number of the request it refers to.
    pub n: u64,
    /// UTC time the request was sent, RFC 3339 with milliseconds.
    pub ts: String,
    /// The same instant as UNIX milliseconds.
    pub unix_ms: i64,
    /// The match day this request belongs to: the `day` of the most recent
    /// `GET /` answer. `None` before the first one.
    pub day: Option<i64>,
    /// `GET` / `POST`.
    pub method: String,
    /// Full URL.
    pub url: String,
    /// Attempt within one client call (a GET is repeated on transient
    /// failures).
    pub attempt: u32,
    /// Milliseconds until the response was complete or the failure.
    pub elapsed_ms: u64,
    /// `response` or `failed`.
    pub result: String,
    /// HTTP status code, when a status line arrived.
    pub status: Option<u16>,
    /// The standard reason phrase for `status` (`OK`, `Unauthorized`, …).
    pub status_text: Option<String>,
    /// `timeout`, `connect`, `body` or `request`, for `failed`.
    pub failure: Option<String>,
    /// Error text: the transport error, or the decode error.
    pub error: Option<String>,
    /// Request headers set by the client (credentials masked).
    pub request_headers: Vec<(String, String)>,
    /// Request body, complete. `None` for a GET.
    pub request_body: Option<String>,
    /// Response headers (credentials masked).
    pub response_headers: Vec<(String, String)>,
    /// Response body, complete. `None` when none arrived.
    pub response_body: Option<String>,
}

enum Msg {
    Exchange(Box<HttpExchange>),
    DecodeFailed(u64, String),
    Flush(SyncSender<()>),
}

enum ConsoleMsg {
    Text(String),
    Flush(SyncSender<()>),
}

/// Where the readable copy goes. `stderr` in the binary; a buffer in tests.
pub type Console = Box<dyn Write + Send>;

/// The HTTP log: attach it to a client with
/// [`protocol::http_client::MatchClientBuilder::observer`].
#[derive(Debug)]
pub struct HttpLog {
    tx: SyncSender<Msg>,
    path: PathBuf,
    dropped: Arc<AtomicU64>,
    write_errors: Arc<AtomicU64>,
}

impl HttpLog {
    /// Create `<root>/<run_id>/http.jsonl` and start the writer thread.
    ///
    /// # Errors
    /// The directory or the file could not be created — found out at
    /// process start, before any match traffic.
    pub fn start(
        root: &Path,
        run_id: &str,
        secrets: Vec<Secret>,
        console: Console,
    ) -> io::Result<Arc<Self>> {
        let dir = root.join(run_id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("http.jsonl");
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let write_errors = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let (tx, rx) = sync_channel(QUEUE_CAPACITY);
        let (console_tx, console_rx) = sync_channel(QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("http-log-console".to_owned())
            .spawn(move || run_console(console, &console_rx))?;
        let writer = Writer {
            run_id: run_id.to_owned(),
            path: path.clone(),
            file: Some(file),
            console: console_tx,
            dropped: Arc::clone(&dropped),
            secrets,
            n: 0,
            day: None,
            write_errors: Arc::clone(&write_errors),
            last_complaint: None,
        };
        std::thread::Builder::new().name("http-log".to_owned()).spawn(move || writer.run(&rx))?;
        Ok(Arc::new(Self { tx, path, dropped, write_errors }))
    }

    /// The `http.jsonl` path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Entries dropped because the queue was full, and failed file writes.
    #[must_use]
    pub fn lost(&self) -> (u64, u64) {
        (self.dropped.load(Ordering::Relaxed), self.write_errors.load(Ordering::Relaxed))
    }

    fn send(&self, msg: Msg) {
        if self.tx.try_send(msg).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Wait until everything queued so far is printed and on disk, for at
    /// most `timeout`. For process exit and tests; never called from the
    /// match loop.
    pub fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (ack_tx, ack_rx) = sync_channel(1);
        let mut msg = Msg::Flush(ack_tx);
        loop {
            match self.tx.try_send(msg) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(back)) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    msg = back;
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        ack_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_ok()
    }
}

impl HttpObserver for HttpLog {
    fn exchange(&self, exchange: HttpExchange) {
        self.send(Msg::Exchange(Box::new(exchange)));
    }

    fn decode_failed(&self, exchange_id: u64, error: &str) {
        self.send(Msg::DecodeFailed(exchange_id, error.to_owned()));
    }
}

/// Writer-thread state. Must not panic: the release profile aborts on panic.
struct Writer {
    run_id: String,
    path: PathBuf,
    file: Option<File>,
    console: SyncSender<ConsoleMsg>,
    dropped: Arc<AtomicU64>,
    secrets: Vec<Secret>,
    n: u64,
    day: Option<i64>,
    write_errors: Arc<AtomicU64>,
    last_complaint: Option<Instant>,
}

impl Writer {
    fn run(mut self, rx: &Receiver<Msg>) {
        // The exchanges a decode failure may still refer to, by client id
        // (without their headers and bodies, which the failure line omits).
        let mut recent: VecDeque<(u64, HttpLogEntry)> = VecDeque::with_capacity(RECENT_EXCHANGES);
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Exchange(exchange) => {
                    let entry = self.entry(&exchange);
                    self.emit(&entry);
                    if recent.len() == RECENT_EXCHANGES {
                        recent.pop_front();
                    }
                    recent.push_back((
                        exchange.id,
                        HttpLogEntry {
                            kind: "decode_error".to_owned(),
                            result: "failed".to_owned(),
                            failure: Some("decode".to_owned()),
                            error: None,
                            request_headers: Vec::new(),
                            request_body: None,
                            response_headers: Vec::new(),
                            response_body: None,
                            ..entry
                        },
                    ));
                }
                Msg::DecodeFailed(id, error) => {
                    // Matched by id, never by "the last one": an exchange
                    // that was dropped, or another client clone's request in
                    // between, must not be blamed.
                    match recent.iter().find(|(known, _)| *known == id) {
                        Some((_, template)) => {
                            let entry = HttpLogEntry {
                                error: Some(self.redact(&error)),
                                ..template.clone()
                            };
                            self.emit(&entry);
                        }
                        None => {
                            self.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                Msg::Flush(ack) => {
                    // The console thread acknowledges once it has printed
                    // everything before this point. If it is stuck, the
                    // caller's `flush` times out — it is never waited for
                    // here.
                    let _ = self.console.try_send(ConsoleMsg::Flush(ack));
                }
            }
        }
    }

    fn redact(&self, text: &str) -> String {
        self.secrets.iter().fold(text.to_owned(), |acc, secret| secret.redact(&acc))
    }

    fn headers(&self, headers: &[(String, String)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(name, value)| {
                let lower = name.to_ascii_lowercase();
                let value = if SECRET_HEADER_MARKERS.iter().any(|marker| lower.contains(marker)) {
                    MASKED.to_owned()
                } else {
                    self.redact(value)
                };
                (name.clone(), value)
            })
            .collect()
    }

    fn body(&self, bytes: &[u8]) -> String {
        self.redact(&String::from_utf8_lossy(bytes))
    }

    fn entry(&mut self, exchange: &HttpExchange) -> HttpLogEntry {
        self.n += 1;
        let unix_ms = exchange
            .started_at
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        let ts =
            DateTime::<Utc>::from(exchange.started_at).to_rfc3339_opts(SecondsFormat::Millis, true);
        let mut entry = HttpLogEntry {
            kind: "exchange".to_owned(),
            run_id: self.run_id.clone(),
            n: self.n,
            ts,
            unix_ms,
            day: self.day,
            method: exchange.method.clone(),
            url: self.redact(&exchange.url),
            attempt: exchange.attempt,
            elapsed_ms: u64::try_from(exchange.elapsed.as_millis()).unwrap_or(u64::MAX),
            result: String::new(),
            status: None,
            status_text: None,
            failure: None,
            error: None,
            request_headers: self.headers(&exchange.request_headers),
            request_body: exchange.request_body.as_deref().map(|b| self.body(b)),
            response_headers: Vec::new(),
            response_body: None,
        };
        match &exchange.outcome {
            HttpOutcome::Response { status, headers, body } => {
                "response".clone_into(&mut entry.result);
                entry.status = Some(*status);
                entry.status_text = status_text(*status);
                entry.response_headers = self.headers(headers);
                entry.response_body = Some(self.body(body));
                // A `GET /` answer opens a day: later requests belong to it.
                if exchange.method == "GET" && (200..300).contains(status) {
                    if let Some(day) = serde_json::from_slice::<Value>(body)
                        .ok()
                        .filter(|v| v.get("endsAt").is_some())
                        .and_then(|v| v.get("day").and_then(Value::as_i64))
                    {
                        self.day = Some(day);
                        entry.day = Some(day);
                    }
                }
            }
            HttpOutcome::Failed { kind, error, partial } => {
                "failed".clone_into(&mut entry.result);
                entry.failure = Some((*kind).to_owned());
                entry.error = Some(self.redact(error));
                if let Some((status, headers)) = partial {
                    entry.status = Some(*status);
                    entry.status_text = status_text(*status);
                    entry.response_headers = self.headers(headers);
                }
            }
        }
        entry
    }

    fn print(&self, text: String) {
        if self.console.try_send(ConsoleMsg::Text(text)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// File first: the durable copy must not wait for a console.
    fn emit(&mut self, entry: &HttpLogEntry) {
        let result = serde_json::to_string(entry).map_err(io::Error::other).and_then(|mut line| {
            line.push('\n');
            self.append(line.as_bytes())
        });
        self.print(render(entry));
        if let Err(error) = result {
            self.write_errors.fetch_add(1, Ordering::Relaxed);
            self.file = None;
            let due = self.last_complaint.is_none_or(|at| at.elapsed() >= Duration::from_secs(5));
            if due {
                self.last_complaint = Some(Instant::now());
                self.print(format!(
                    "http-log: writing {} failed ({error}). The match continues.\n",
                    self.path.display()
                ));
            }
        }
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.file.is_some() && !self.path.exists() {
            self.file = None;
        }
        if self.file.is_none() {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
            if file.metadata()?.len() > 0 {
                file.write_all(b"\n")?;
            }
            self.file = Some(file);
        }
        let Some(file) = self.file.as_mut() else { return Ok(()) };
        file.write_all(bytes)?;
        file.sync_data()
    }
}

/// The console thread: the only place that touches the console, so a
/// console that blocks holds up nothing else.
fn run_console(mut console: Console, rx: &Receiver<ConsoleMsg>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            ConsoleMsg::Text(text) => {
                let _ = console.write_all(text.as_bytes());
                let _ = console.flush();
            }
            ConsoleMsg::Flush(ack) => {
                let _ = console.flush();
                let _ = ack.try_send(());
            }
        }
    }
}

fn status_text(status: u16) -> Option<String> {
    reqwest_reason(status).map(str::to_owned)
}

/// Standard reason phrases for the statuses a contest server can return.
const fn reqwest_reason(status: u16) -> Option<&'static str> {
    Some(match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        425 => "Too Early",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => return None,
    })
}

/// The console form of an entry. Everything in the entry is printed, so the
/// console and `http.jsonl` can be compared field by field.
#[must_use]
pub fn render(entry: &HttpLogEntry) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let day = entry.day.map_or_else(|| "-".to_owned(), |d| d.to_string());
    if entry.kind == "decode_error" {
        let _ = writeln!(
            out,
            "[HTTP #{} DECODE ERROR] {} {} {} day={day} : {}",
            entry.n,
            entry.ts,
            entry.method,
            entry.url,
            entry.error.as_deref().unwrap_or("")
        );
        return out;
    }
    let verdict = match (entry.result.as_str(), entry.status) {
        ("response", Some(status)) => {
            format!("{status} {}", entry.status_text.as_deref().unwrap_or("(no reason phrase)"))
        }
        _ => format!(
            "FAILED {}{}",
            entry.failure.as_deref().unwrap_or("unknown"),
            entry.status.map_or_else(String::new, |s| format!(
                " after status {s} {}",
                entry.status_text.as_deref().unwrap_or("")
            ))
        ),
    };
    let _ = writeln!(
        out,
        "[HTTP #{}] {} {} {} -> {verdict} ({} ms) attempt={} day={day}",
        entry.n, entry.ts, entry.method, entry.url, entry.elapsed_ms, entry.attempt
    );
    for (name, value) in &entry.request_headers {
        let _ = writeln!(out, "  > {name}: {value}");
    }
    if let Some(body) = &entry.request_body {
        let _ = writeln!(out, "  > body: {body}");
    }
    for (name, value) in &entry.response_headers {
        let _ = writeln!(out, "  < {name}: {value}");
    }
    if let Some(body) = &entry.response_body {
        let _ = writeln!(out, "  < body: {body}");
    }
    if let Some(error) = &entry.error {
        let _ = writeln!(out, "  ! error: {error}");
    }
    out
}

/// A run id for a run that has no match record to share one with.
#[must_use]
pub fn new_run_id() -> String {
    let now: DateTime<Utc> = SystemTime::now().into();
    format!("{}-p{}", now.format("%Y%m%dT%H%M%SZ"), std::process::id())
}

/// Parse `http.jsonl`: the entries, and how many non-empty lines did not
/// parse (a torn write).
///
/// # Errors
/// The file cannot be read.
#[allow(dead_code)] // Library API for tests and tooling; the binary only writes.
pub fn load_http_log(path: &Path) -> io::Result<(Vec<HttpLogEntry>, usize)> {
    let raw = std::fs::read(path)?;
    let mut entries = Vec::new();
    let mut unparsed = 0;
    for line in String::from_utf8_lossy(&raw).lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(entry) => entries.push(entry),
            Err(_) => unparsed += 1,
        }
    }
    Ok((entries, unparsed))
}
