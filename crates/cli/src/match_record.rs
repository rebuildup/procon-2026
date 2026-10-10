//! Durable per-match record (Issue #394).
//!
//! `procon coordinator --record-dir <DIR>` keeps everything a postmortem
//! needs to rebuild a match, in one directory per process start:
//!
//! ```text
//! <DIR>/<run-id>/
//!   manifest.json    who ran: commit, binary sha256, solver, kinds, OS
//!   setting.json     `GET /setting`, in the wire shape
//!   events.jsonl     one line per observation / plan / `POST /` attempt
//!   coordinator.log  the `tracing` output of this run
//!   state.json       the `--state-output` snapshot (schema unchanged)
//! ```
//!
//! # What the match loop pays
//!
//! Recording is observation, never part of answering a day. The loop hands
//! each record to a bounded queue with `try_send` and moves on; a writer
//! thread serialises, redacts and writes. Nothing here issues an HTTP
//! request, waits for the disk, or returns an error to the loop:
//!
//! - **queue full** — the record is dropped and counted;
//! - **write failure** (disk full, directory gone) — counted, reported on
//!   stderr, and the file is reopened for the next record;
//! - both counts are surfaced by [`MatchRecorder::report_health`] and are
//!   visible in the file as gaps in `seq`.
//!
//! # What survives a kill
//!
//! Every event line is written and `fsync`ed by the writer thread as soon as
//! it is dequeued, so a `kill -9` loses at most the records still in the
//! queue (normally none: a day produces a handful). The loop does **not**
//! wait for that `fsync` before `POST /` — a slow disk must not cost a day —
//! so the one record that can be missing after a power cut is the one being
//! written at that instant. A torn last line is skipped by [`load_record`].
//!
//! # Observed vs. submitted
//!
//! `day_state.observed` is the `GET /` body of that day and nothing else.
//! `plan.actions` is the body handed to `POST /`. The record never stores a
//! position predicted from a plan: where the agents really ended up is the
//! next day's `day_state`.
//!
//! # Secrets
//!
//! The manifest is built from resolved configuration, never from `argv`, and
//! every line of every file passes through [`Secret::redact`] for each
//! registered token before it reaches the disk.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use protocol::match_config::{
    ActionPlanBody, MatchSetting as WireSetting, MatchState as WireMatchState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::arena::Secret;

/// Version of the on-disk layout (`manifest.json` + `events.jsonl`). Bump on
/// any change a reader written against the previous version would misread.
pub const RECORD_SCHEMA_VERSION: u32 = 1;

/// Records the loop may have queued ahead of the writer. A day produces
/// about ten (state, plan, attempts, result, log lines); this is hours of
/// retry-storm headroom, and bounds memory at a few MB if the disk stalls.
const QUEUE_CAPACITY: usize = 8192;

/// Minimum gap between two write-failure reports on stderr.
const COMPLAINT_INTERVAL: Duration = Duration::from_secs(5);

/// How many earlier runs a resumed run looks through for its predecessor.
const PREDECESSOR_SCAN_LIMIT: usize = 64;

/// A predecessor that never saw `starts_at` is only taken for the same match
/// if it started this recently.
const PREDECESSOR_MAX_AGE_MS: i64 = 30 * 60 * 1000;

/// What the server said to one `POST /` attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitOutcome {
    /// `revision >= 0`.
    Accepted,
    /// `revision < 0`: the plan is illegal.
    Rejected,
    /// No verdict (transport failure, timeout, 5xx, undecodable body). The
    /// server may or may not have taken the plan.
    NoVerdict,
    /// `403 AccessTimeError`: the answer window is shut.
    WindowClosed,
    /// A 4xx that a repeat cannot fix.
    ClientError,
    /// Authentication failure or a malformed URL: the run ends.
    Fatal,
}

/// What became of one day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayStatus {
    /// The server returned `revision >= 0` for our plan.
    Accepted,
    /// The server returned `revision < 0`.
    Rejected,
    /// No plan was accepted: none was ready, or none got a verdict.
    Unanswered,
}

/// One entry of `events.jsonl`. Our own keys are `snake_case`; `observed`
/// and `actions` hold official wire payloads untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordEvent {
    /// `GET /setting` answered; the body is in `setting.json`.
    Setting {
        /// `startsAt` as first seen (`0` while the kind window is open).
        starts_at: i64,
        /// `403` answers waited out before this one.
        polls: u32,
    },
    /// The composition this run fields.
    Kinds {
        /// `explicit` (`--kinds`), `auto` (`--kinds-auto`), `default`, or
        /// `server` (resumed: read back from `GET /`).
        source: String,
        /// Wire `AgentKind` per agent slot.
        kinds: Vec<i32>,
        /// The same composition in `--kinds` spelling.
        pattern: String,
    },
    /// `POST /agent` finished.
    AgentSubmit {
        /// Whether the server took the composition.
        ok: bool,
        /// Wall time of the call including its retries.
        elapsed_ms: u64,
        /// Why it failed.
        error: Option<String>,
    },
    /// The process joined a match that was already running.
    Resume {
        /// The day the server was on.
        day: i32,
    },
    /// Written by the recorder after `resume`: the earlier run of this match.
    Continues {
        /// Run id of the predecessor directory.
        previous_run_id: String,
        /// `starts_at` (same setting and same start time) or `setting_only`
        /// (same setting; the predecessor ended before the start was known).
        matched_by: String,
    },
    /// The server announced the match start.
    MatchStart {
        /// `startsAt` (UNIX seconds).
        starts_at: i64,
    },
    /// A `GET /` or `GET /setting` attempt that failed and was repeated.
    RequestFailed {
        /// `GET /` or `GET /setting`.
        endpoint: String,
        /// The day being waited for, when there is one.
        day: Option<i32>,
        /// The client's error text.
        error: String,
    },
    /// The state the server reported when a day opened.
    DayState {
        /// The day the loop was waiting for.
        expected_day: i32,
        /// Wall time spent getting this answer, retries included.
        fetch_ms: u64,
        /// The `GET /` body: our agents, the other teams, traffic, `endsAt`.
        observed: WireMatchState,
    },
    /// Days the server moved past without a plan from us being attempted.
    DaysSkipped {
        /// 0-based day indexes.
        days: Vec<u32>,
    },
    /// The final plan for a day, exactly as handed to `POST /`.
    Plan {
        /// 0-based day.
        day: u32,
        /// The day's step budget (`daySteps[day]`).
        day_steps: u32,
        /// Solver budget handed to the solver.
        budget_ms: u64,
        /// Wall time from `GET /` to the finished plan.
        solve_ms: u64,
        /// One action list per agent, in agent order (wire codes).
        actions: ActionPlanBody,
    },
    /// No plan was produced for a day.
    PlanFailed {
        /// 0-based day.
        day: u32,
        /// Wall time spent before giving up.
        solve_ms: u64,
        /// Why.
        reason: String,
    },
    /// One `POST /` call (the HTTP client may retry transport errors inside
    /// a call; those are not visible here).
    SubmitAttempt {
        /// 0-based day.
        day: u32,
        /// 1-based attempt number within the day.
        attempt: u32,
        /// Wall time of the call.
        elapsed_ms: u64,
        /// What the server said.
        outcome: SubmitOutcome,
        /// The revision, when the server returned one.
        revision: Option<i32>,
        /// Error text, when there was no acceptance.
        error: Option<String>,
    },
    /// The day's verdict.
    DayResult {
        /// 0-based day.
        day: u32,
        /// Accepted, rejected or unanswered.
        status: DayStatus,
        /// The revision the server returned, if any.
        revision: Option<i32>,
        /// `POST /` calls made.
        attempts: u32,
        /// Wall time from `GET /` to the verdict.
        answer_ms: i64,
        /// Time left to the server's deadline at the verdict (negative: late).
        margin_ms: i64,
        /// Why the day went unanswered.
        reason: Option<String>,
    },
    /// The loop returned.
    MatchEnd {
        /// `completed`, `rejected` or `error`.
        status: String,
        /// Accepted days.
        days_completed: usize,
        /// Accepted revisions, in day order.
        revisions: Vec<i32>,
        /// Error text for `rejected` / `error`.
        error: Option<String>,
    },
    /// An event type this build does not know (written by a newer one).
    #[serde(other)]
    Unknown,
}

/// One line of `events.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordLine {
    /// [`RECORD_SCHEMA_VERSION`].
    pub v: u32,
    /// Position in this run, assigned when the event happened. A gap means a
    /// record was dropped (queue full) or failed to reach the disk.
    pub seq: u64,
    /// The run directory name.
    pub run_id: String,
    /// UTC wall clock of the event, RFC 3339 with milliseconds — the same
    /// clock and zone as `coordinator.log`.
    pub ts: String,
    /// The same instant as UNIX milliseconds.
    pub unix_ms: i64,
    /// Milliseconds since the recorder started (monotonic clock).
    pub mono_ms: u64,
    /// The event.
    #[serde(flatten)]
    pub event: RecordEvent,
}

enum Msg {
    Event { seq: u64, unix_ms: i64, mono_ms: u64, event: Box<RecordEvent> },
    Setting(Box<WireSetting>),
    State(Vec<u8>),
    Manifest(Value),
    Log(Vec<u8>),
    Flush(SyncSender<()>),
}

struct Inner {
    run_id: String,
    dir: PathBuf,
    tx: SyncSender<Msg>,
    started: Instant,
    seq: Arc<AtomicU64>,
    dropped: AtomicU64,
    write_errors: Arc<AtomicU64>,
    reported: AtomicU64,
}

/// Counters of records that did not make it to the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecorderHealth {
    /// Records dropped because the queue was full.
    pub dropped: u64,
    /// Writes that failed.
    pub write_errors: u64,
}

/// Handle to the recorder. Cheap to clone; a disabled handle (the default)
/// turns every call into a no-op, so call sites need no `if`.
#[derive(Clone, Default)]
pub struct MatchRecorder {
    inner: Option<Arc<Inner>>,
}

impl std::fmt::Debug for MatchRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.as_ref() {
            Some(inner) => write!(f, "MatchRecorder({})", inner.run_id),
            None => f.write_str("MatchRecorder(off)"),
        }
    }
}

/// Inputs of [`MatchRecorder::start`].
#[derive(Debug, Default)]
pub struct RecorderOptions {
    /// Caller-supplied manifest fields (solver, candidate, server, …). The
    /// recorder adds `schema_version`, `run_id`, start time, OS and arch.
    pub manifest: Map<String, Value>,
    /// Tokens that must never reach the disk.
    pub secrets: Vec<Secret>,
    /// The running executable, hashed into the manifest on the writer
    /// thread. `None` skips the hash.
    pub binary_path: Option<PathBuf>,
}

impl MatchRecorder {
    /// A recorder that records nothing.
    #[must_use]
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Create `<root>/<run-id>/`, write the first `manifest.json` and start
    /// the writer thread.
    ///
    /// This is the one fallible, synchronous step: it runs at process start,
    /// before any match traffic, so an unusable directory is found out then.
    ///
    /// # Errors
    /// The directory or the manifest could not be created.
    pub fn start(root: &Path, options: RecorderOptions) -> io::Result<Self> {
        let now = Utc::now();
        let (run_id, dir) = create_run_dir(root, now)?;
        let mut manifest = options.manifest;
        manifest.insert("schema_version".to_owned(), json!(RECORD_SCHEMA_VERSION));
        manifest.insert("run_id".to_owned(), json!(run_id));
        manifest.insert("started_at".to_owned(), json!(rfc3339(now.timestamp_millis())));
        manifest.insert("started_unix_ms".to_owned(), json!(now.timestamp_millis()));
        manifest.insert("pid".to_owned(), json!(std::process::id()));
        manifest.insert("os".to_owned(), json!(std::env::consts::OS));
        manifest.insert("arch".to_owned(), json!(std::env::consts::ARCH));
        manifest.insert("package_version".to_owned(), json!(env!("CARGO_PKG_VERSION")));
        manifest.insert(
            "match_identity".to_owned(),
            json!({
                "official_match_id": null,
                "note": "the official API publishes no match id; run_id is ours",
            }),
        );
        manifest.insert(
            "files".to_owned(),
            json!({
                "setting": "setting.json",
                "events": "events.jsonl",
                "log": "coordinator.log",
                "state": "state.json",
            }),
        );

        let write_errors = Arc::new(AtomicU64::new(0));
        let seq = Arc::new(AtomicU64::new(0));
        let writer = Writer {
            dir: dir.clone(),
            root: root.to_path_buf(),
            run_id: run_id.clone(),
            secrets: options.secrets,
            events: None,
            log: None,
            manifest,
            setting: None,
            seq: Arc::clone(&seq),
            write_errors: Arc::clone(&write_errors),
            last_complaint: None,
            started: Instant::now(),
        };
        writer.write_manifest()?;

        let (tx, rx) = sync_channel(QUEUE_CAPACITY);
        let started = writer.started;
        let binary_path = options.binary_path;
        std::thread::Builder::new()
            .name("match-record".to_owned())
            .spawn(move || writer.run(&rx, binary_path.as_deref()))?;

        Ok(Self {
            inner: Some(Arc::new(Inner {
                run_id,
                dir,
                tx,
                started,
                seq,
                dropped: AtomicU64::new(0),
                write_errors,
                reported: AtomicU64::new(0),
            })),
        })
    }

    /// Whether anything is being recorded.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// The run id (directory name), when recording.
    #[must_use]
    pub fn run_id(&self) -> Option<&str> {
        self.inner.as_ref().map(|i| i.run_id.as_str())
    }

    /// The run directory, when recording.
    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        self.inner.as_ref().map(|i| i.dir.as_path())
    }

    fn send(inner: &Inner, msg: Msg) {
        match inner.tx.try_send(msg) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                inner.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record an event. `build` runs only when recording is on, so a
    /// disabled recorder costs one branch.
    pub fn event_with(&self, build: impl FnOnce() -> RecordEvent) {
        let Some(inner) = self.inner.as_ref() else { return };
        let msg = Msg::Event {
            seq: inner.seq.fetch_add(1, Ordering::Relaxed),
            unix_ms: Utc::now().timestamp_millis(),
            mono_ms: u64::try_from(inner.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            event: Box::new(build()),
        };
        Self::send(inner, msg);
    }

    /// Record an event.
    pub fn event(&self, event: RecordEvent) {
        self.event_with(|| event);
    }

    /// Save the `GET /setting` body as `setting.json`.
    pub fn setting(&self, setting: &WireSetting) {
        if let Some(inner) = self.inner.as_ref() {
            Self::send(inner, Msg::Setting(Box::new(setting.clone())));
        }
    }

    /// Replace `state.json` with an already serialised snapshot.
    pub fn state_snapshot(&self, json: Vec<u8>) {
        if let Some(inner) = self.inner.as_ref() {
            Self::send(inner, Msg::State(json));
        }
    }

    /// Merge `patch` (a JSON object) into `manifest.json`.
    pub fn update_manifest(&self, patch: Value) {
        if let Some(inner) = self.inner.as_ref() {
            Self::send(inner, Msg::Manifest(patch));
        }
    }

    /// Records lost so far.
    #[must_use]
    pub fn health(&self) -> RecorderHealth {
        self.inner.as_ref().map_or_else(RecorderHealth::default, |inner| RecorderHealth {
            dropped: inner.dropped.load(Ordering::Relaxed),
            write_errors: inner.write_errors.load(Ordering::Relaxed),
        })
    }

    /// Log an error if records were lost since the last call. Two atomic
    /// loads when nothing is wrong.
    pub fn report_health(&self) {
        let Some(inner) = self.inner.as_ref() else { return };
        let health = self.health();
        let total = health.dropped.saturating_add(health.write_errors);
        if inner.reported.swap(total, Ordering::Relaxed) != total {
            tracing::error!(
                dropped = health.dropped,
                write_errors = health.write_errors,
                dir = %inner.dir.display(),
                "match record is incomplete: records were lost (the match itself is unaffected)"
            );
        }
    }

    /// Wait until everything queued so far is on disk, for at most
    /// `timeout`. Returns `false` if the writer did not get there in time.
    /// Called at process exit and by tests; never from the match loop.
    pub fn flush(&self, timeout: Duration) -> bool {
        let Some(inner) = self.inner.as_ref() else { return true };
        let deadline = Instant::now() + timeout;
        let (ack_tx, ack_rx) = sync_channel(1);
        let mut msg = Msg::Flush(ack_tx);
        loop {
            match inner.tx.try_send(msg) {
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

    /// A `tracing` writer that appends to `coordinator.log` through the
    /// same queue (so logging never waits for the disk either).
    #[must_use]
    pub fn log_writer(&self) -> RecordLogWriter {
        RecordLogWriter { recorder: self.clone() }
    }
}

/// `MakeWriter` for the `coordinator.log` layer.
#[derive(Debug, Clone)]
pub struct RecordLogWriter {
    recorder: MatchRecorder,
}

/// One formatted log event, sent to the writer thread when dropped.
#[derive(Debug)]
pub struct RecordLogLine {
    recorder: MatchRecorder,
    buf: Vec<u8>,
}

impl Write for RecordLogLine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RecordLogLine {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        if let Some(inner) = self.recorder.inner.as_ref() {
            MatchRecorder::send(inner, Msg::Log(std::mem::take(&mut self.buf)));
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RecordLogWriter {
    type Writer = RecordLogLine;

    fn make_writer(&'a self) -> Self::Writer {
        RecordLogLine { recorder: self.recorder.clone(), buf: Vec::with_capacity(256) }
    }
}

/// Check that a run directory could be created under `root`, without
/// leaving one behind. Used by `coordinator --dry-run` as the pre-match
/// check that recording is configured and the disk is writable.
///
/// # Errors
/// `root` cannot be created or written to.
pub fn probe_record_root(root: &Path) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    let probe = root.join(format!(".probe-{}", std::process::id()));
    let result =
        File::create(&probe).and_then(|mut f| f.write_all(b"ok").and_then(|()| f.sync_all()));
    let _ = std::fs::remove_file(&probe);
    result
}

fn rfc3339(unix_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(unix_ms)
        .map_or_else(|| unix_ms.to_string(), |t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn create_run_dir(root: &Path, now: DateTime<Utc>) -> io::Result<(String, PathBuf)> {
    std::fs::create_dir_all(root)?;
    let base = format!("{}-p{}", now.format("%Y%m%dT%H%M%SZ"), std::process::id());
    for n in 0..100_u32 {
        let run_id = if n == 0 { base.clone() } else { format!("{base}-{n}") };
        let dir = root.join(&run_id);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((run_id, dir)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free run directory name"))
}

/// Recursive object merge: `patch` keys win, nested objects are merged.
fn merge(into: &mut Map<String, Value>, patch: Map<String, Value>) {
    for (key, value) in patch {
        match (into.get_mut(&key), value) {
            (Some(Value::Object(existing)), Value::Object(incoming)) => merge(existing, incoming),
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

fn without_starts_at(setting: &Value) -> Value {
    let mut copy = setting.clone();
    if let Some(object) = copy.as_object_mut() {
        object.remove("startsAt");
    }
    copy
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut file = File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

/// The writer thread's state. Nothing in here may panic: the release
/// profile aborts on panic, and this thread shares the process with a match.
struct Writer {
    dir: PathBuf,
    root: PathBuf,
    run_id: String,
    secrets: Vec<Secret>,
    events: Option<File>,
    log: Option<File>,
    manifest: Map<String, Value>,
    setting: Option<Value>,
    seq: Arc<AtomicU64>,
    write_errors: Arc<AtomicU64>,
    last_complaint: Option<Instant>,
    started: Instant,
}

impl Writer {
    fn run(mut self, rx: &Receiver<Msg>, binary_path: Option<&Path>) {
        if let Some(path) = binary_path {
            let binary = match sha256_file(path) {
                Ok((digest, size)) => json!({
                    "path": path.display().to_string(),
                    "sha256": digest,
                    "size_bytes": size,
                }),
                Err(e) => json!({
                    "path": path.display().to_string(),
                    "sha256": null,
                    "error": e.to_string(),
                }),
            };
            self.manifest.insert("binary".to_owned(), binary);
            self.save_manifest();
        }
        while let Ok(msg) = rx.recv() {
            self.handle(msg);
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Event { seq, unix_ms, mono_ms, event } => {
                self.write_event(seq, unix_ms, mono_ms, &event);
                self.after_event(&event);
            }
            Msg::Setting(setting) => self.save_setting(&setting),
            Msg::State(bytes) => {
                let text = self.redact(&String::from_utf8_lossy(&bytes));
                let result = write_atomic(&self.dir.join("state.json"), text.as_bytes());
                self.check("state.json", result);
            }
            Msg::Manifest(patch) => {
                if let Value::Object(patch) = patch {
                    merge(&mut self.manifest, patch);
                    self.save_manifest();
                }
            }
            Msg::Log(bytes) => {
                let text = self.redact(&String::from_utf8_lossy(&bytes));
                let path = self.dir.join("coordinator.log");
                let result = append(&self.dir, &path, &mut self.log, text.as_bytes(), false);
                self.check("coordinator.log", result);
            }
            Msg::Flush(ack) => {
                if let Some(file) = self.log.as_ref() {
                    let result = file.sync_data();
                    self.check("coordinator.log", result);
                }
                let _ = ack.try_send(());
            }
        }
    }

    fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for secret in &self.secrets {
            out = secret.redact(&out);
            // The same token as it appears inside a JSON string.
            let escaped = serde_json::to_string(secret.expose()).unwrap_or_default();
            let escaped = escaped.trim_matches('"');
            if !escaped.is_empty() && escaped != secret.expose() {
                out = out.replace(escaped, "[REDACTED]");
            }
        }
        out
    }

    fn check(&mut self, what: &str, result: io::Result<()>) {
        let Err(error) = result else { return };
        self.write_errors.fetch_add(1, Ordering::Relaxed);
        // stderr, not `tracing`: a log line would come straight back here.
        let due = self.last_complaint.is_none_or(|at| at.elapsed() >= COMPLAINT_INTERVAL);
        if due {
            self.last_complaint = Some(Instant::now());
            let total = self.write_errors.load(Ordering::Relaxed);
            let _ = writeln!(
                io::stderr(),
                "match-record: writing {what} in {} failed ({error}); {total} failed write(s) so far. The match continues.",
                self.dir.display()
            );
        }
    }

    fn write_event(&mut self, seq: u64, unix_ms: i64, mono_ms: u64, event: &RecordEvent) {
        let line = RecordLine {
            v: RECORD_SCHEMA_VERSION,
            seq,
            run_id: self.run_id.clone(),
            ts: rfc3339(unix_ms),
            unix_ms,
            mono_ms,
            event: event.clone(),
        };
        let result = serde_json::to_string(&line).map_err(io::Error::other).and_then(|json| {
            let mut text = self.redact(&json);
            text.push('\n');
            let path = self.dir.join("events.jsonl");
            append(&self.dir, &path, &mut self.events, text.as_bytes(), true)
        });
        self.check("events.jsonl", result);
    }

    /// Keep `manifest.json` a summary of the run as the events go by.
    fn after_event(&mut self, event: &RecordEvent) {
        match event {
            RecordEvent::Kinds { source, kinds, pattern } => {
                self.manifest.insert(
                    "agent_kinds".to_owned(),
                    json!({ "source": source, "kinds": kinds, "pattern": pattern }),
                );
            }
            RecordEvent::MatchStart { starts_at } => {
                merge(&mut self.manifest, object(json!({ "match": { "starts_at": starts_at } })));
            }
            RecordEvent::Resume { day } => {
                self.manifest.insert("resumed_at_day".to_owned(), json!(day));
                if let Some((run_id, matched_by)) = self.find_predecessor() {
                    self.manifest.insert("continues_run_id".to_owned(), json!(run_id));
                    let seq = self.seq.fetch_add(1, Ordering::Relaxed);
                    let mono_ms =
                        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let link = RecordEvent::Continues {
                        previous_run_id: run_id,
                        matched_by: matched_by.to_owned(),
                    };
                    self.write_event(seq, Utc::now().timestamp_millis(), mono_ms, &link);
                }
            }
            RecordEvent::MatchEnd { status, days_completed, revisions, error } => {
                self.manifest.insert(
                    "result".to_owned(),
                    json!({
                        "status": status,
                        "days_completed": days_completed,
                        "revisions": revisions,
                        "error": error,
                    }),
                );
            }
            _ => return,
        }
        self.save_manifest();
    }

    fn save_setting(&mut self, setting: &WireSetting) {
        let value = match serde_json::to_value(setting) {
            Ok(value) => value,
            Err(e) => {
                self.check("setting.json", Err(io::Error::other(e)));
                return;
            }
        };
        let result = serde_json::to_string_pretty(&value)
            .map_err(io::Error::other)
            .and_then(|t| write_atomic(&self.dir.join("setting.json"), self.redact(&t).as_bytes()));
        self.check("setting.json", result);
        merge(
            &mut self.manifest,
            object(json!({
                "match": {
                    "starts_at": setting.starts_at,
                    "days": setting.day_steps.len(),
                    "map_width": setting.map.width,
                    "map_height": setting.map.height,
                    "agents": setting.agents.len(),
                    "players": setting.players,
                }
            })),
        );
        self.setting = Some(value);
        self.save_manifest();
    }

    fn write_manifest(&self) -> io::Result<()> {
        let text = serde_json::to_string_pretty(&self.manifest).map_err(io::Error::other)?;
        write_atomic(&self.dir.join("manifest.json"), self.redact(&text).as_bytes())
    }

    fn save_manifest(&mut self) {
        let result = self.write_manifest();
        self.check("manifest.json", result);
    }

    /// The most recent earlier run that recorded the same match.
    fn find_predecessor(&self) -> Option<(String, &'static str)> {
        let ours = self.setting.as_ref()?;
        let our_start = ours.get("startsAt").and_then(Value::as_i64).unwrap_or(0);
        let our_begin = self.manifest.get("started_unix_ms").and_then(Value::as_i64).unwrap_or(0);
        let ours = without_starts_at(ours);

        let mut names: Vec<String> = std::fs::read_dir(&self.root)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| *name != self.run_id)
            .collect();
        names.sort_unstable_by(|a, b| b.cmp(a));
        names.truncate(PREDECESSOR_SCAN_LIMIT);

        for name in names {
            let dir = self.root.join(&name);
            let Some(theirs) = read_json(&dir.join("setting.json")) else { continue };
            if without_starts_at(&theirs) != ours {
                continue;
            }
            let manifest = read_json(&dir.join("manifest.json")).unwrap_or(Value::Null);
            let their_start =
                manifest.pointer("/match/starts_at").and_then(Value::as_i64).unwrap_or(0);
            if our_start > 0 && their_start == our_start {
                return Some((name, "starts_at"));
            }
            let their_begin = manifest.get("started_unix_ms").and_then(Value::as_i64).unwrap_or(0);
            let age = our_begin.saturating_sub(their_begin);
            if their_start == 0 && (0..=PREDECESSOR_MAX_AGE_MS).contains(&age) {
                return Some((name, "setting_only"));
            }
        }
        None
    }
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Append `bytes` to `path`, opening (and re-creating the directory) on
/// demand. After a failure the handle is dropped, and the next write starts
/// on a fresh line so a torn record cannot swallow its successor.
fn append(
    dir: &Path,
    path: &Path,
    slot: &mut Option<File>,
    bytes: &[u8],
    sync: bool,
) -> io::Result<()> {
    // An open handle outlives its file on Unix: without this check, records
    // written after the directory was removed would go nowhere, silently.
    if slot.is_some() && !path.exists() {
        *slot = None;
    }
    if slot.is_none() {
        std::fs::create_dir_all(dir)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        if file.metadata()?.len() > 0 {
            file.write_all(b"\n")?;
        }
        *slot = Some(file);
    }
    let Some(file) = slot.as_mut() else { return Ok(()) };
    let result = file.write_all(bytes).and_then(|()| if sync { file.sync_data() } else { Ok(()) });
    if result.is_err() {
        *slot = None;
    }
    result
}

/// A run directory read back from disk.
#[derive(Debug, Clone)]
pub struct LoadedRecord {
    /// `manifest.json`.
    pub manifest: Value,
    /// `setting.json`, when the run got as far as `GET /setting`.
    pub setting: Option<WireSetting>,
    /// The parsed lines of `events.jsonl`, in file order.
    pub lines: Vec<RecordLine>,
    /// Non-empty lines that did not parse (a torn write).
    pub unparsed_lines: usize,
}

/// Read a run directory. A missing `setting.json` or `events.jsonl` is not
/// an error (the run may have ended before either existed); lines that do
/// not parse are counted, not fatal.
///
/// # Errors
/// `manifest.json` is missing or not JSON, or a file that exists cannot be
/// read.
pub fn load_record(dir: &Path) -> io::Result<LoadedRecord> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(dir.join("manifest.json"))?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("manifest.json: {e}")))?;
    let setting = match std::fs::read(dir.join("setting.json")) {
        Ok(raw) => Some(serde_json::from_slice(&raw).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("setting.json: {e}"))
        })?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let raw = match std::fs::read(dir.join("events.jsonl")) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let mut lines = Vec::new();
    let mut unparsed_lines = 0;
    for line in String::from_utf8_lossy(&raw).lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<RecordLine>(line) {
            Ok(parsed) => lines.push(parsed),
            Err(_) => unparsed_lines += 1,
        }
    }
    Ok(LoadedRecord { manifest, setting, lines, unparsed_lines })
}

impl LoadedRecord {
    /// The plan handed to `POST /` for each day (the last one, if a day has
    /// several).
    #[must_use]
    pub fn plans(&self) -> BTreeMap<u32, &ActionPlanBody> {
        self.lines
            .iter()
            .filter_map(|line| match &line.event {
                RecordEvent::Plan { day, actions, .. } => Some((*day, actions)),
                _ => None,
            })
            .collect()
    }

    /// The `GET /` state observed when each day opened.
    #[must_use]
    pub fn observed_days(&self) -> BTreeMap<i32, &WireMatchState> {
        self.lines
            .iter()
            .filter_map(|line| match &line.event {
                RecordEvent::DayState { observed, .. } => Some((observed.day, observed)),
                _ => None,
            })
            .collect()
    }

    /// The verdict of each day: status and the revision the server returned.
    #[must_use]
    pub fn day_results(&self) -> BTreeMap<u32, (DayStatus, Option<i32>)> {
        self.lines
            .iter()
            .filter_map(|line| match &line.event {
                RecordEvent::DayResult { day, status, revision, .. } => {
                    Some((*day, (*status, *revision)))
                }
                _ => None,
            })
            .collect()
    }

    /// `POST /` calls recorded for `day`.
    #[must_use]
    pub fn submit_attempts(&self, day: u32) -> Vec<&RecordEvent> {
        self.lines
            .iter()
            .map(|line| &line.event)
            .filter(|event| matches!(event, RecordEvent::SubmitAttempt { day: d, .. } if *d == day))
            .collect()
    }

    /// Sequence numbers missing from the file: records that were dropped or
    /// failed to reach the disk.
    #[must_use]
    pub fn missing_seq(&self) -> u64 {
        let Some(last) = self.lines.iter().map(|l| l.seq).max() else { return 0 };
        let mut seen: Vec<u64> = self.lines.iter().map(|l| l.seq).collect();
        seen.sort_unstable();
        seen.dedup();
        (last + 1).saturating_sub(u64::try_from(seen.len()).unwrap_or(u64::MAX))
    }

    /// A compact, human-checkable summary (`procon record show`).
    #[must_use]
    pub fn summary(&self) -> Value {
        let plans = self.plans();
        let observed = self.observed_days();
        let results = self.day_results();
        let mut day_keys: Vec<u32> = plans.keys().copied().collect();
        day_keys.extend(results.keys().copied());
        day_keys.extend(observed.keys().filter_map(|d| u32::try_from(*d).ok()));
        day_keys.sort_unstable();
        day_keys.dedup();
        let days: Vec<Value> = day_keys
            .into_iter()
            .map(|day| {
                let state = i32::try_from(day).ok().and_then(|d| observed.get(&d));
                let plan = plans.get(&day);
                let result = results.get(&day);
                json!({
                    "day": day,
                    "observed": state.is_some(),
                    "agents": state.map(|s| s.agents.len()),
                    "others": state.map(|s| s.others.len()),
                    "traffics": state.map(|s| s.traffics.len()),
                    "plan_agents": plan.map(|p| p.len()),
                    "plan_actions": plan.map(|p| p.iter().map(Vec::len).sum::<usize>()),
                    "post_attempts": self.submit_attempts(day).len(),
                    "status": result.map(|r| r.0),
                    "revision": result.and_then(|r| r.1),
                })
            })
            .collect();
        json!({
            "run_id": self.manifest.get("run_id"),
            "schema_version": self.manifest.get("schema_version"),
            "started_at": self.manifest.get("started_at"),
            "source_commit_sha": self.manifest.get("source_commit_sha"),
            "binary_sha256": self.manifest.pointer("/binary/sha256"),
            "solver": self.manifest.get("solver"),
            "agent_kinds": self.manifest.pointer("/agent_kinds/pattern"),
            "continues_run_id": self.manifest.get("continues_run_id"),
            "result": self.manifest.get("result"),
            "map": self.setting.as_ref().map(|s| json!({
                "width": s.map.width,
                "height": s.map.height,
                "cells": s.map.cells.iter().map(Vec::len).sum::<usize>(),
                "spots": s.spots.len(),
                "agents": s.agents.len(),
                "days": s.day_steps.len(),
            })),
            "events": self.lines.len(),
            "unparsed_lines": self.unparsed_lines,
            "missing_seq": self.missing_seq(),
            "days": days,
        })
    }
}

/// SHA-256 of a file, as lowercase hex, with its size. Local implementation
/// so the venue binary gains no dependency for one manifest field.
fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 1 << 16];
    let mut size = 0_u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(buf.get(..n).unwrap_or(&[]));
    }
    Ok((hasher.finish(), size))
}

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    length: u64,
}

impl Sha256 {
    const fn new() -> Self {
        Self {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            block: [0; 64],
            filled: 0,
            length: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        for &byte in data {
            self.block[self.filled] = byte;
            self.filled += 1;
            if self.filled == 64 {
                self.compress();
                self.filled = 0;
            }
        }
    }

    #[allow(clippy::many_single_char_names, reason = "the names of FIPS 180-4")]
    fn compress(&mut self) {
        let mut w = [0_u32; 64];
        for (i, chunk) in self.block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 =
                h.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    fn finish(mut self) -> String {
        let bits = self.length.wrapping_mul(8);
        self.update_padding(0x80);
        while self.filled != 56 {
            self.update_padding(0);
        }
        for byte in bits.to_be_bytes() {
            self.update_padding(byte);
        }
        let mut hex = String::with_capacity(64);
        for word in self.state {
            let _ = write!(hex, "{word:08x}");
        }
        hex
    }

    fn update_padding(&mut self, byte: u8) {
        self.block[self.filled] = byte;
        self.filled += 1;
        if self.filled == 64 {
            self.compress();
            self.filled = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finish()
    }

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(sha(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256_of_a_file_matches_the_in_memory_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("blob");
        let data: Vec<u8> = (0..200_000_u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).expect("write");
        let (digest, size) = sha256_file(&path).expect("hash");
        assert_eq!(digest, sha(&data));
        assert_eq!(size, 200_000);
    }

    fn start(root: &Path, secrets: Vec<Secret>) -> MatchRecorder {
        let mut manifest = Map::new();
        manifest.insert("solver".to_owned(), json!("test"));
        MatchRecorder::start(root, RecorderOptions { manifest, secrets, binary_path: None })
            .expect("recorder starts")
    }

    const FLUSH: Duration = Duration::from_secs(10);

    #[test]
    fn disabled_recorder_is_a_no_op() {
        let rec = MatchRecorder::disabled();
        assert!(!rec.is_enabled());
        rec.event_with(|| unreachable!("a disabled recorder must not build events"));
        rec.state_snapshot(Vec::new());
        rec.report_health();
        assert!(rec.flush(Duration::from_millis(1)));
        assert_eq!(rec.health(), RecorderHealth::default());
    }

    #[test]
    fn events_round_trip_through_the_file() {
        let root = tempfile::tempdir().expect("tempdir");
        let rec = start(root.path(), Vec::new());
        let plan: ActionPlanBody = vec![vec![0, 1, -3, 5], vec![-6], vec![2, 2, 2, 3, 4, -1]];
        rec.event(RecordEvent::Plan {
            day: 2,
            day_steps: 6,
            budget_ms: 1000,
            solve_ms: 12,
            actions: plan.clone(),
        });
        rec.event(RecordEvent::SubmitAttempt {
            day: 2,
            attempt: 1,
            elapsed_ms: 3,
            outcome: SubmitOutcome::NoVerdict,
            revision: None,
            error: Some("transport error".to_owned()),
        });
        rec.event(RecordEvent::SubmitAttempt {
            day: 2,
            attempt: 2,
            elapsed_ms: 4,
            outcome: SubmitOutcome::Accepted,
            revision: Some(7),
            error: None,
        });
        rec.event(RecordEvent::DayResult {
            day: 2,
            status: DayStatus::Accepted,
            revision: Some(7),
            attempts: 2,
            answer_ms: 20,
            margin_ms: 900,
            reason: None,
        });
        assert!(rec.flush(FLUSH));

        let loaded = load_record(rec.dir().expect("dir")).expect("load");
        assert_eq!(loaded.unparsed_lines, 0);
        assert_eq!(loaded.missing_seq(), 0);
        assert_eq!(loaded.plans().get(&2), Some(&&plan));
        assert_eq!(loaded.submit_attempts(2).len(), 2);
        assert_eq!(loaded.day_results().get(&2), Some(&(DayStatus::Accepted, Some(7))));
        assert_eq!(loaded.manifest["schema_version"], json!(RECORD_SCHEMA_VERSION));
        assert_eq!(loaded.manifest["run_id"], json!(rec.run_id().expect("run id")));
        assert_eq!(loaded.manifest["solver"], json!("test"));
        let seqs: Vec<u64> = loaded.lines.iter().map(|l| l.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3]);
        assert!(loaded.lines.iter().all(|l| l.ts.ends_with('Z') && l.unix_ms > 0));
    }

    #[test]
    fn two_starts_get_two_directories() {
        let root = tempfile::tempdir().expect("tempdir");
        let a = start(root.path(), Vec::new());
        let b = start(root.path(), Vec::new());
        assert_ne!(a.run_id(), b.run_id());
        a.event(RecordEvent::MatchStart { starts_at: 1 });
        b.event(RecordEvent::MatchStart { starts_at: 2 });
        assert!(a.flush(FLUSH) && b.flush(FLUSH));
        let la = load_record(a.dir().expect("dir")).expect("load a");
        let lb = load_record(b.dir().expect("dir")).expect("load b");
        assert_eq!(la.lines.len(), 1);
        assert_eq!(lb.lines.len(), 1);
        assert_eq!(la.manifest["match"]["starts_at"], json!(1));
        assert_eq!(lb.manifest["match"]["starts_at"], json!(2));
    }

    #[test]
    fn secrets_never_reach_any_file() {
        let root = tempfile::tempdir().expect("tempdir");
        let token = "s3cr3t-\"quoted\"-token";
        let rec = start(root.path(), vec![Secret::new(token.to_owned())]);
        rec.event(RecordEvent::RequestFailed {
            endpoint: "GET /".to_owned(),
            day: Some(0),
            error: format!("server echoed {token}"),
        });
        rec.state_snapshot(format!("{{\"error\":\"bad token {token}\"}}").into_bytes());
        rec.update_manifest(json!({ "note": format!("token is {token}") }));
        {
            let mut line = tracing_subscriber::fmt::MakeWriter::make_writer(&rec.log_writer());
            writeln!(line, "header Procon-Token: {token}").expect("log write");
        }
        assert!(rec.flush(FLUSH));
        let dir = rec.dir().expect("dir");
        for name in ["manifest.json", "events.jsonl", "coordinator.log", "state.json"] {
            let text = std::fs::read_to_string(dir.join(name)).expect(name);
            assert!(!text.contains(token), "{name} leaked the token");
            assert!(!text.contains("s3cr3t"), "{name} leaked part of the token");
            assert!(text.contains("[REDACTED]"), "{name} should show the redaction");
        }
        assert_eq!(load_record(dir).expect("load").unparsed_lines, 0);
    }

    #[test]
    fn a_vanished_directory_is_counted_and_recording_recovers() {
        let root = tempfile::tempdir().expect("tempdir");
        let rec = start(root.path(), Vec::new());
        rec.event(RecordEvent::MatchStart { starts_at: 1 });
        assert!(rec.flush(FLUSH));
        let dir = rec.dir().expect("dir").to_path_buf();

        // Put a file where the run directory was: every write now fails.
        std::fs::remove_dir_all(&dir).expect("remove run dir");
        std::fs::write(&dir, b"in the way").expect("block the path");
        rec.event(RecordEvent::MatchStart { starts_at: 2 });
        assert!(rec.flush(FLUSH), "the writer keeps running through failures");
        assert!(rec.health().write_errors >= 1);

        // The obstacle goes away: recording picks up again on its own.
        std::fs::remove_file(&dir).expect("unblock");
        rec.event(RecordEvent::Resume { day: 3 });
        assert!(rec.flush(FLUSH));
        let text = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(text.contains("\"type\":\"resume\""));
    }

    #[test]
    fn a_torn_line_is_skipped_not_fatal() {
        let root = tempfile::tempdir().expect("tempdir");
        let rec = start(root.path(), Vec::new());
        rec.event(RecordEvent::MatchStart { starts_at: 9 });
        assert!(rec.flush(FLUSH));
        let dir = rec.dir().expect("dir");
        let mut file =
            OpenOptions::new().append(true).open(dir.join("events.jsonl")).expect("open");
        file.write_all(b"{\"v\":1,\"seq\":1,\"run_id\":\"x\",\"ts\":\"2026").expect("tear");
        let loaded = load_record(dir).expect("load");
        assert_eq!(loaded.lines.len(), 1);
        assert_eq!(loaded.unparsed_lines, 1);
    }

    #[test]
    fn unknown_event_types_still_load() {
        let line = r#"{"v":2,"seq":5,"run_id":"r","ts":"t","unix_ms":1,"mono_ms":2,"type":"from_the_future","x":1}"#;
        let parsed: RecordLine = serde_json::from_str(line).expect("parse");
        assert_eq!(parsed.event, RecordEvent::Unknown);
    }

    #[test]
    fn a_full_queue_drops_and_counts_instead_of_blocking() {
        let (tx, _rx) = sync_channel(1);
        let inner = Inner {
            run_id: "r".to_owned(),
            dir: PathBuf::from("unused"),
            tx,
            started: Instant::now(),
            seq: Arc::new(AtomicU64::new(0)),
            dropped: AtomicU64::new(0),
            write_errors: Arc::new(AtomicU64::new(0)),
            reported: AtomicU64::new(0),
        };
        let rec = MatchRecorder { inner: Some(Arc::new(inner)) };
        let began = Instant::now();
        for _ in 0..1000 {
            rec.event(RecordEvent::MatchStart { starts_at: 1 });
        }
        assert!(began.elapsed() < Duration::from_secs(1), "try_send must not block");
        assert_eq!(rec.health().dropped, 999);
        assert!(!rec.flush(Duration::from_millis(20)), "a stuck writer is reported, not awaited");
    }

    #[test]
    fn probe_reports_an_unusable_root() {
        let root = tempfile::tempdir().expect("tempdir");
        assert!(probe_record_root(&root.path().join("new/records")).is_ok());
        let file = root.path().join("a-file");
        std::fs::write(&file, b"x").expect("write");
        assert!(probe_record_root(&file.join("records")).is_err());
    }
}
