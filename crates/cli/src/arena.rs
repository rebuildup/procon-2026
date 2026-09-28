//! Arena profile, match cadence, pre-match registration, and durable
//! evidence for the unofficial practice arena (Issue #179).
//!
//! This module is a **configuration / orchestration layer** on top of the
//! canonical coordinator path — it never talks to the arena through its own
//! protocol client. [`wait_for_registration`] drives the very same
//! [`protocol::MatchClient`] the coordinator loop later plays the match
//! with, so there is exactly one client, one retry policy, and one error
//! classification for the whole run.
//!
//! # Shape of an arena run
//!
//! ```text
//! procon coordinator --arena --next --candidate '<id>'
//!   ├─ resolve profile     env PROCON_ARENA_SERVER / PROCON_ARENA_TOKEN
//!   ├─ wait for window     next :00/:15/:30/:45 minus the pre-match lead
//!   ├─ register            bounded `GET /setting` poll
//!   │                        403 plain = keep waiting
//!   │                        403 lobby-registered = success (stop polling)
//!   │                        200 JSON = success
//!   ├─ run the loop        run_coordinator_loop (unchanged canonical path)
//!   └─ append evidence     target/arena/runs.jsonl  (JSONL, no secrets)
//! ```
//!
//! # Safety rules encoded here
//!
//! - **Bounded polling.** [`RegistrationOptions::poll_interval`] only ever
//!   runs *inside* the pre-match window and stops at the match start; the
//!   attempt budget is derived from the window, capped by
//!   [`RegistrationOptions::max_attempts`].
//! - **403 during the wait is not an error.** Registration reuses
//!   [`crate::coordinator_loop::retry_on_access_time`], the same bounded
//!   retry the coordinator already applies to `403 AccessTimeError`
//!   (issues #156 / #164 / PR #165). The one exception is the arena's
//!   lobby confirmation — `403 … you are registered for match <id>
//!   departing at <unix>` — which the live recon proved *is* the
//!   registration success signal (Issue #222): it stops the poll and is
//!   recorded, never retried.
//! - **Fail closed on drift.** A non-JSON body (the arena's SPA HTML shell)
//!   surfaces as [`ArenaError::ApiDrift`] after a single request — no
//!   guessed request shapes, no retry storm.
//! - **No secrets in artifacts.** The token is wrapped in [`Secret`]
//!   (redacted `Debug`/`Display`) and scrubbed out of every error string
//!   that can reach evidence or a state snapshot.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
// The submitted `POST /agent` composition (Issues #238/#243) is recorded in
// engine spelling — `Patrol` / `Supply` — so an arena line and the offline
// `RunRecord.agent_kinds` that selected its candidate read the same values.
use engine::model::AgentKind;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// `DaySummary` (Issue #236 per-day evidence) is produced by the coordinator
// loop and embedded verbatim in [`ArenaRunRecord::days`] — one type, one
// wire shape, no arena-specific copy to drift out of sync.
use crate::coordinator_loop::{
    CoordinatorLoopError, CoordinatorOutcome, DaySummary, retry_on_access_time,
};

/// Default arena endpoint. Not a secret — the endpoint is public.
pub const ARENA_SERVER_DEFAULT: &str = "https://procon37arena.online";

/// Arena matches are generated every `:00 / :15 / :30 / :45`.
pub const ARENA_MATCH_PERIOD_SECS: i64 = 15 * 60;

/// Pre-match registration opens this long before each match start
/// (arena notice: 「およそ試合3分前から token 付きで `/settings` 等を GET すると参加登録」).
///
/// The notice's plural `/settings` wording is the arena SPA's fallback
/// route (it answers HTML); the endpoint that actually exists is the
/// singular [`ARENA_REGISTRATION_PATH`] — see Issue #222.
pub const ARENA_PRE_MATCH_LEAD_SECS: i64 = 3 * 60;

/// Default poll interval while waiting inside the pre-match window. Well
/// inside the cadence (a 3-minute window yields at most ~18 requests) and
/// far coarser than per-second polling.
pub const ARENA_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Hard cap on registration attempts regardless of window length.
pub const ARENA_REGISTRATION_MAX_ATTEMPTS: u32 = 60;

/// Where a registration GET is sent: the **singular** `/setting`, the
/// official `ProconApi_getMatchSetting` endpoint of the `OpenAPI` spec
/// (`.reference/procon37/openapi.json`), polled inside the pre-match window
/// to enrol. The arena notice's plural `/settings` is only the SPA fallback
/// (HTML) — the live smoke hit it and failed closed (Issue #222). Kept here
/// so the path is one reviewable constant.
pub const ARENA_REGISTRATION_PATH: &str = "/setting";

/// Default JSONL evidence path. Under `target/` (gitignored) so durable
/// run records never enter the repository; summaries belong in Issues.
pub const ARENA_EVIDENCE_DEFAULT_PATH: &str = "target/arena/runs.jsonl";

/// Schema marker for [`ArenaRunRecord`]. Bump on any breaking change.
pub const ARENA_RECORD_SCHEMA_VERSION: u32 = 1;

/// `record_kind` discriminator. Arena records are deliberately **not**
/// `bench` `RunRecord`s: offline `MatchSim` scores and arena results must
/// never be mixed (Issue #179 AC), so they use a different kind tag, a
/// different file, and a different schema.
pub const ARENA_RECORD_KIND: &str = "arena-run";

/// A secret value that refuses to print itself.
///
/// `Debug` and `Display` both render `[REDACTED]`, so a token wrapped in
/// this type cannot leak through `{:?}` / `{}` formatting, `tracing`
/// field capture, or an `#[error("...")]` interpolation. Read it only to
/// build the [`protocol::MatchClient`] header via [`Self::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a secret. An empty secret stays empty (and stays redacted).
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// The raw value. Only call this to hand the token to the HTTP client.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Replace every occurrence of this secret in `text` with
    /// `[REDACTED]`. Used before any server-provided text (error bodies,
    /// drift details) is persisted into state snapshots or evidence.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        if self.0.is_empty() {
            return text.to_owned();
        }
        text.replace(&self.0, "[REDACTED]")
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Arena environment variables, read once at startup.
///
/// Kept separate from [`resolve_profile`] so the resolution logic is a pure
/// function under test (edition 2024 makes `env::set_var` unsafe, and
/// mutating process env in parallel tests is racy anyway).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArenaEnv {
    /// `PROCON_ARENA_SERVER` — optional, defaults to [`ARENA_SERVER_DEFAULT`].
    pub server: Option<String>,
    /// `PROCON_ARENA_TOKEN` — required for an arena run. Injected from
    /// Infisical at runtime; never committed, never a CLI argument.
    pub token: Option<String>,
}

impl ArenaEnv {
    /// Read `PROCON_ARENA_*` from the process environment. Empty strings
    /// count as unset.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            server: non_empty_env("PROCON_ARENA_SERVER"),
            token: non_empty_env("PROCON_ARENA_TOKEN"),
        }
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// Resolved arena connection profile: where to talk and which token to
/// authenticate with. Supplied to the canonical coordinator path — the
/// arena gets no dedicated protocol client.
#[derive(Debug, Clone)]
pub struct ArenaProfile {
    /// Base URL of the arena (no trailing slash required).
    pub server_url: String,
    /// Bearer token; redacted everywhere it could be rendered.
    pub token: Secret,
}

/// Errors from arena profile resolution, registration, and evidence IO.
///
/// Every `String` field must already be free of secrets — build messages
/// through [`Secret::redact`] when they embed server-provided text.
#[derive(Debug, Error)]
pub enum ArenaError {
    /// Missing / unusable configuration (e.g. no token).
    #[error("arena configuration error: {0}")]
    Config(String),
    /// The bounded registration poll exhausted its window without a 200.
    #[error("arena registration did not complete: {message}")]
    NotRegistered {
        /// Last `403 AccessTimeError` message seen, if any.
        message: String,
    },
    /// A protocol endpoint answered with a non-JSON body — API drift.
    #[error("arena API drift at {path}: {detail}")]
    ApiDrift {
        /// Endpoint path that drifted.
        path: String,
        /// Short, single-line description of what came back instead.
        detail: String,
    },
    /// Authentication failed (401). Never retried.
    #[error("arena authentication failed: {message}")]
    Auth {
        /// Redacted server message.
        message: String,
    },
    /// Any other HTTP / transport failure.
    #[error("arena request failed: {message}")]
    Http {
        /// Redacted error text.
        message: String,
    },
    /// Writing the evidence JSONL failed.
    #[error("arena evidence write failed: {0}")]
    Evidence(String),
}

impl ArenaError {
    /// Evidence status this error maps to.
    #[must_use]
    pub const fn status(&self) -> ArenaRunStatus {
        match self {
            Self::ApiDrift { .. } => ArenaRunStatus::ApiDrift,
            Self::Config(_)
            | Self::NotRegistered { .. }
            | Self::Auth { .. }
            | Self::Http { .. } => ArenaRunStatus::RegistrationFailed,
            Self::Evidence(_) => ArenaRunStatus::Error,
        }
    }
}

/// Resolve the arena profile.
///
/// Precedence for both fields: explicit CLI value (which clap already
/// merges with `PROCON_SERVER` / `PROCON_TOKEN`) → arena env
/// (`PROCON_ARENA_*`) → the public default server. A missing token is a
/// hard error — the message tells the operator how to inject it, and never
/// contains a token.
///
/// # Errors
/// [`ArenaError::Config`] when no token is available.
pub fn resolve_profile(
    explicit_server: Option<String>,
    explicit_token: Option<String>,
    env: ArenaEnv,
) -> Result<ArenaProfile, ArenaError> {
    let server_url = explicit_server
        .filter(|s| !s.trim().is_empty())
        .or(env.server)
        .unwrap_or_else(|| ARENA_SERVER_DEFAULT.to_owned());
    let token = explicit_token.filter(|t| !t.trim().is_empty()).or(env.token).ok_or_else(|| {
        ArenaError::Config(
            "arena token is not set — inject it from Infisical \
                 (e.g. `infisical run --env=dev -- just arena-next …`) \
                 which exports PROCON_ARENA_TOKEN"
                .to_owned(),
        )
    })?;
    Ok(ArenaProfile { server_url, token: Secret::new(token) })
}

/// One arena match window: when registration opens and when the match
/// starts. Both timestamps are UTC instants on the `:00/:15/:30/:45`
/// cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchWindow {
    /// Registration opens (pre-match lead before [`Self::starts_at`]).
    pub opens_at: DateTime<Utc>,
    /// Match start — the cadence boundary itself.
    pub starts_at: DateTime<Utc>,
}

/// The next match window at or after `now`.
///
/// Pure function of the clock — no I/O — so the cadence maths is unit
/// tested directly. Matches land exactly on `:00/:15/:30/:45`; a `now`
/// sitting on a boundary targets *that* match (joining as soon as the
/// window is open), anything later rolls to the next boundary.
#[must_use]
pub fn next_match_window(now: DateTime<Utc>, pre_match_lead: Duration) -> MatchWindow {
    let lead = i64::try_from(pre_match_lead.as_secs()).unwrap_or(0);
    let timestamp = now.timestamp();
    let remainder = timestamp.rem_euclid(ARENA_MATCH_PERIOD_SECS);
    let starts_unix =
        if remainder == 0 { timestamp } else { timestamp + (ARENA_MATCH_PERIOD_SECS - remainder) };
    let starts_at = DateTime::<Utc>::from_timestamp(starts_unix, 0).unwrap_or(now);
    let opens_at = DateTime::<Utc>::from_timestamp(starts_unix - lead, 0).unwrap_or(starts_at);
    MatchWindow { opens_at, starts_at }
}

/// Knobs for the pre-match registration poll.
#[derive(Debug, Clone)]
pub struct RegistrationOptions {
    /// How long before `starts_at` the window opens.
    pub pre_match_lead: Duration,
    /// Poll interval inside the window only.
    pub poll_interval: Duration,
    /// Upper bound on attempts no matter how long the window is.
    pub max_attempts: u32,
    /// Explicit window. `None` derives it from the `now` handed to
    /// [`wait_for_registration`] — tests (and a future "resume" mode) pass
    /// an already-open window so no wall-clock sleep is involved.
    pub window: Option<MatchWindow>,
}

impl Default for RegistrationOptions {
    fn default() -> Self {
        Self {
            pre_match_lead: Duration::from_secs(
                u64::try_from(ARENA_PRE_MATCH_LEAD_SECS).unwrap_or(180),
            ),
            poll_interval: ARENA_POLL_INTERVAL,
            max_attempts: ARENA_REGISTRATION_MAX_ATTEMPTS,
            window: None,
        }
    }
}

/// Result of a successful registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationOutcome {
    /// Window that was targeted. When the arena confirmed through the
    /// lobby, `starts_at` carries its departure time instead of the local
    /// cadence estimate.
    pub window: MatchWindow,
    /// When the arena acknowledged the registration.
    pub registered_at: DateTime<Utc>,
    /// How many `GET /setting` requests it took (1 = first attempt).
    pub attempts: u32,
    /// Match identifier echoed by the arena (`matchId` in a 200 body, or
    /// the lobby confirmation's `match <id>`), when it provides one.
    pub arena_match_id: Option<String>,
}

/// Attempt budget for one registration: as many polls as fit between the
/// window open (or "now", whichever is later) and the match start, at
/// `interval`, capped by `max_attempts` and never below 1.
#[must_use]
pub fn registration_attempt_budget(
    window: MatchWindow,
    now: DateTime<Utc>,
    interval: Duration,
    max_attempts: u32,
) -> u32 {
    let open = if now > window.opens_at { now } else { window.opens_at };
    let budget_ms = window.starts_at.signed_duration_since(open).num_milliseconds().max(0);
    let interval_ms = i64::try_from(interval.as_millis()).unwrap_or(i64::MAX).max(1);
    let fits = budget_ms / interval_ms + 1;
    let fits = u32::try_from(fits).unwrap_or(u32::MAX);
    fits.clamp(1, max_attempts.max(1))
}

/// What one `GET /setting` attempt answered during registration.
enum RegistrationAnswer {
    /// 200 JSON — the registration / match-setting payload.
    Json(serde_json::Value),
    /// `403 AccessTimeError` carrying the lobby's registration
    /// confirmation (live recon 2026-09-27, Issue #222).
    Lobby(LobbyRegistration),
}

/// A registration the arena lobby confirmed inside a `403 AccessTimeError`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LobbyRegistration {
    /// Arena match id (`match <id>` in the lobby message).
    match_id: String,
    /// Match departure time (`departing at <unix>`) — the authoritative
    /// match start, recorded as the evidence window.
    departing_at_unix: i64,
}

/// Parse the arena's lobby confirmation out of an `AccessTimeError`
/// message, e.g.
/// `lobby: you are registered for match 469 departing at 1790001234`.
///
/// This wording shape is spec, not a guess: the official arena guide
/// (`https://procon37arena.online/arena/guide`) states that a tokened GET
/// at an official endpoint during reception is the entry, answered with
/// the same pre-match `403 AccessTimeError` whose message carries
/// 「lobby: you are registered …」 — and the live recon captured a full
/// sample (Issue #222).
///
/// `None` for anything else: a plain "not yet" 403 keeps the bounded
/// retry, and an unrecognised wording fails towards today's
/// [`ArenaError::NotRegistered`] give-up rather than a guessed success.
fn parse_lobby_registration(message: &str) -> Option<LobbyRegistration> {
    let rest = message.split_once("you are registered for match ")?.1;
    let (match_id, rest) = rest.split_once(" departing at ")?;
    let match_id = match_id.trim();
    let departing_at_unix = rest.trim().parse::<i64>().ok()?;
    if match_id.is_empty() || departing_at_unix < 0 {
        return None;
    }
    Some(LobbyRegistration { match_id: match_id.to_owned(), departing_at_unix })
}

/// One registration attempt against the official singular endpoint.
///
/// A `403 AccessTimeError` that parses as the lobby confirmation becomes
/// [`RegistrationAnswer::Lobby`] — success, so the caller stops polling.
/// Every other response (including a plain `403 AccessTimeError`) is
/// handed back untouched, keeping the existing bounded retry semantics
/// and the fail-closed drift / auth classification unchanged.
async fn registration_attempt(
    client: &protocol::MatchClient,
) -> Result<RegistrationAnswer, protocol::HttpClientError> {
    match client.get_json::<serde_json::Value>(ARENA_REGISTRATION_PATH).await {
        Ok(body) => Ok(RegistrationAnswer::Json(body)),
        Err(protocol::HttpClientError::AccessTime { message }) => {
            match parse_lobby_registration(&message) {
                Some(registered) => Ok(RegistrationAnswer::Lobby(registered)),
                None => Err(protocol::HttpClientError::AccessTime { message }),
            }
        }
        Err(e) => Err(e),
    }
}

/// Wait for the pre-match window, then register by polling
/// `GET /setting` with the arena token until the arena confirms.
///
/// Behaviour:
///
/// - sleeps until [`MatchWindow::opens_at`] when the window is not open
///   yet (this is the `--next` cadence wait),
/// - polls at [`RegistrationOptions::poll_interval`] until the match start,
///   never faster and never outside the window,
/// - treats the arena's **lobby confirmation** — `403 AccessTimeError`
///   whose message reads `lobby: you are registered for match <id>
///   departing at <unix>` (live recon 2026-09-27, Issue #222) — as a
///   successful registration: the poll stops, `arena_match_id` is
///   recorded, and [`MatchWindow::starts_at`] is re-based on the
///   departure time (the evidence's `window_starts_at_unix`),
/// - succeeds on a 200 JSON body too, picking up `matchId` / `match_id` /
///   `id` when present,
/// - treats any other `403 AccessTimeError` as "not registered yet" and
///   retries it with the coordinator's own bounded `AccessTime` retry
///   semantics — the shared [`retry_on_access_time`] helper used by
///   `POST /agent` and `GET /` since PR #165,
/// - fails closed immediately on a non-JSON (HTML SPA shell) response via
///   [`ArenaError::ApiDrift`], on 401 via [`ArenaError::Auth`], and on any
///   other error via [`ArenaError::Http`] — no request-shape guessing, no
///   retry storms.
///
/// # Errors
/// See [`ArenaError`].
pub async fn wait_for_registration(
    client: &protocol::MatchClient,
    profile: &ArenaProfile,
    opts: &RegistrationOptions,
    now: DateTime<Utc>,
) -> Result<RegistrationOutcome, ArenaError> {
    let window = opts.window.unwrap_or_else(|| next_match_window(now, opts.pre_match_lead));
    if now < window.opens_at {
        let wait = window.opens_at.signed_duration_since(now).num_milliseconds().max(0);
        tracing::info!(
            seconds = wait / 1000,
            opens_at = %window.opens_at,
            starts_at = %window.starts_at,
            "waiting for the arena pre-match window"
        );
        tokio::time::sleep(Duration::from_millis(u64::try_from(wait).unwrap_or(0))).await;
    }
    let effective_now = Utc::now();
    let attempts_budget =
        registration_attempt_budget(window, effective_now, opts.poll_interval, opts.max_attempts);
    tracing::info!(
        path = ARENA_REGISTRATION_PATH,
        attempts_budget,
        "registering for the arena match"
    );

    let calls = std::sync::atomic::AtomicU32::new(0);
    let answer: RegistrationAnswer = retry_on_access_time(
        "GET /setting",
        attempts_budget,
        opts.poll_interval,
        "the pre-match registration window did not open within the retry budget",
        || {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            registration_attempt(client)
        },
    )
    .await
    .map_err(|e| classify_registration_error(e, profile))?;
    let attempts = calls.load(std::sync::atomic::Ordering::Relaxed);

    match answer {
        RegistrationAnswer::Lobby(registered) => {
            // Entry is first-come-first-served (arena guide): this one
            // lobby `GET /setting` already consumed the slot, so return
            // immediately — never retry and re-register once confirmed.
            //
            // The arena's own departure time is authoritative: it becomes
            // the recorded window start (evidence `window_starts_at_unix`).
            let starts_at = DateTime::<Utc>::from_timestamp(registered.departing_at_unix, 0)
                .unwrap_or(window.starts_at);
            let lead =
                i64::try_from(opts.pre_match_lead.as_secs()).unwrap_or(ARENA_PRE_MATCH_LEAD_SECS);
            let opens_at = DateTime::<Utc>::from_timestamp(starts_at.timestamp() - lead, 0)
                .unwrap_or(window.opens_at);
            tracing::info!(
                attempts,
                match_id = %registered.match_id,
                departing_at_unix = registered.departing_at_unix,
                "arena registration confirmed in the lobby"
            );
            Ok(RegistrationOutcome {
                window: MatchWindow { opens_at, starts_at },
                registered_at: Utc::now(),
                attempts,
                arena_match_id: Some(registered.match_id),
            })
        }
        RegistrationAnswer::Json(body) => {
            let arena_match_id = body
                .get("matchId")
                .or_else(|| body.get("match_id"))
                .or_else(|| body.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            Ok(RegistrationOutcome { window, registered_at: Utc::now(), attempts, arena_match_id })
        }
    }
}

/// Map a transport-level registration failure onto a redacted
/// [`ArenaError`]. Server bodies can echo request material, so every
/// message passes through [`Secret::redact`] first.
fn classify_registration_error(e: protocol::HttpClientError, profile: &ArenaProfile) -> ArenaError {
    use protocol::HttpClientError as E;
    match e {
        E::ApiDrift { path, detail } => {
            ArenaError::ApiDrift { path, detail: profile.token.redact(&detail) }
        }
        E::Auth { message } => ArenaError::Auth { message: profile.token.redact(&message) },
        E::AccessTime { message } => {
            ArenaError::NotRegistered { message: profile.token.redact(&message) }
        }
        other => ArenaError::Http { message: profile.token.redact(&other.to_string()) },
    }
}

/// Outcome classification for an arena run. Deliberately overlaps neither
/// `bench::RunStatus` (offline `MatchSim`) nor the coordinator snapshot
/// status — see [`ARENA_RECORD_KIND`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArenaRunStatus {
    /// Match ran to completion (every day accepted).
    Completed,
    /// Server rejected a plan (`revision < 0`).
    Rejected,
    /// Transport / conversion failure while playing the match.
    Error,
    /// Registration never completed inside the pre-match window.
    RegistrationFailed,
    /// The arena answered a protocol endpoint with a non-JSON body.
    ApiDrift,
}

impl fmt::Display for ArenaRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Completed => "completed",
            Self::Rejected => "rejected",
            Self::Error => "error",
            Self::RegistrationFailed => "registration_failed",
            Self::ApiDrift => "api_drift",
        })
    }
}

/// Solver configuration submitted to the arena, recorded verbatim so an
/// arena record can be matched against the offline evidence that selected
/// it (Issue #179 correspondence AC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArenaSolverConfig {
    /// How the solver was chosen: `"candidate"` (Issue #178 registry id via
    /// `--candidate`) or `"cli-solver"` (`--solver` + loop knobs).
    pub mode: String,
    /// `--solver` value, or the candidate's algorithm name.
    pub solver: String,
    /// `--loop-initial-strategy` label, when the CLI solver path was used.
    pub loop_initial_strategy: Option<String>,
    /// `--loop-iterations`, when the CLI solver path was used.
    pub loop_iterations: Option<u32>,
}

/// One durable arena run record (JSONL, one line per run).
///
/// Field set mirrors `bench::evaluate::RunRecord`'s discipline — stable
/// identity, provenance, configuration, outcome — but the schema is
/// intentionally separate so an arena result can never be re-aggregated
/// into an offline score. **No token, email, or account field exists in
/// this type.**
///
/// Additive evolution: fields added later (`day_steps`, `day_seconds`,
/// `days` — Issue #236) are `#[serde(default)]`, so evidence lines written
/// before them still parse and the record stays one JSONL line either way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArenaRunRecord {
    /// Wire-stable schema version (see [`ARENA_RECORD_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Always [`ARENA_RECORD_KIND`] (`"arena-run"`).
    pub record_kind: String,
    /// Issue #178 candidate id, verbatim (e.g.
    /// `multiloop:kmeans=4:seed=42`). `None` when the run used `--solver`.
    pub candidate_id: Option<String>,
    /// The submitted solver configuration.
    pub solver: ArenaSolverConfig,
    /// Commit the run executed at — see [`source_commit_sha`]
    /// (`PROCON_SOURCE_COMMIT_SHA` → `git rev-parse HEAD` → `"unknown"`).
    pub source_commit_sha: String,
    /// Arena base URL the run targeted.
    pub arena_server: String,
    /// Team id the coordinator played as.
    pub team_id: String,
    /// Match start of the targeted window (UNIX seconds).
    pub window_starts_at_unix: i64,
    /// `GET /setting` attempts used during registration, when it ran.
    pub registration_attempts: Option<u32>,
    /// When registration succeeded (UNIX seconds), if registered here.
    pub registered_at_unix: Option<i64>,
    /// Arena-provided match identifier, when available.
    pub arena_match_id: Option<String>,
    /// Outcome classification (see [`ArenaRunStatus`]).
    pub status: ArenaRunStatus,
    /// Days that completed (`revision >= 0`), when the match was played.
    pub days_completed: usize,
    /// Per-day revisions returned by `POST /`.
    pub revisions: Vec<i32>,
    /// `GET /setting`'s `daySteps` — the per-day step budget, recorded once
    /// so every entry of [`days`](Self::days) can be read against its
    /// budget without a replay-JSON download (Issue #236).
    #[serde(default)]
    pub day_steps: Vec<i32>,
    /// `GET /setting`'s `daySeconds` — the per-day answer window, recorded
    /// once alongside [`day_steps`](Self::day_steps).
    #[serde(default)]
    pub day_seconds: Vec<i32>,
    /// Per-day evidence (Issue #236): one entry per played day carrying the
    /// day-start `kind / pos / fuel` of our agents (wire `GET /` state) and
    /// a **movement summary** of the plan we submitted — move/wait counts
    /// and a direction histogram, never the raw answer body. This is what
    /// makes a loss diagnosable without downloading replay JSON (#233 方針5).
    ///
    /// Bounded by construction: entries are one per day, each entry a fixed
    /// number of small fields (see the size-guard test in `tests`).
    #[serde(default)]
    pub days: Vec<DaySummary>,
    /// The agent-kind composition the run declared in `POST /agent`, one
    /// entry per agent slot (Issues #238/#243) — e.g.
    /// `["Patrol","Patrol","Patrol","Patrol","Supply"]` for `--kinds
    /// P,P,P,P,S` on a 5-agent team.
    ///
    /// Same type and spelling as the offline `RunRecord.agent_kinds`, so an
    /// arena line and the offline ablation that selected its candidate
    /// compare without a translation step. `[]` when the run never reached
    /// the kind submission (registration failure, early error).
    #[serde(default)]
    pub kinds: Vec<AgentKind>,
    /// Arena result URL / identifier, when the arena exposes one.
    pub result_ref: Option<String>,
    /// Redacted error text for non-completed runs.
    pub error: Option<String>,
    /// When this record was written (UNIX seconds).
    pub recorded_at_unix: i64,
}

/// Inputs for [`ArenaRunRecord::build`].
pub struct ArenaRecordInput<'a> {
    /// Resolved profile (server + redaction source).
    pub profile: &'a ArenaProfile,
    /// Canonical candidate id, when `--candidate` was used.
    pub candidate_id: Option<&'a str>,
    /// Submitted solver configuration.
    pub solver: ArenaSolverConfig,
    /// Team id.
    pub team_id: &'a str,
    /// Source commit SHA — resolved by [`source_commit_sha`].
    pub source_commit_sha: String,
    /// Targeted match window start (UNIX seconds).
    pub window_starts_at_unix: i64,
    /// Registration outcome, when the registration phase ran.
    pub registration: Option<&'a RegistrationOutcome>,
    /// Coordinator outcome, when the match was played.
    pub outcome: Option<&'a CoordinatorOutcome>,
    /// Classified status.
    pub status: ArenaRunStatus,
    /// Raw error text; redacted before it touches the record.
    pub error: Option<String>,
}

impl ArenaRunRecord {
    /// Build a record. Secret material in `input.error` is redacted here so
    /// callers cannot accidentally persist it.
    #[must_use]
    pub fn build(input: ArenaRecordInput<'_>) -> Self {
        let registration = input.registration;
        let outcome = input.outcome;
        Self {
            schema_version: ARENA_RECORD_SCHEMA_VERSION,
            record_kind: ARENA_RECORD_KIND.to_owned(),
            candidate_id: input.candidate_id.map(str::to_owned),
            solver: input.solver,
            source_commit_sha: input.source_commit_sha,
            arena_server: input.profile.server_url.clone(),
            team_id: input.team_id.to_owned(),
            window_starts_at_unix: input.window_starts_at_unix,
            registration_attempts: registration.map(|r| r.attempts),
            registered_at_unix: registration.map(|r| r.registered_at.timestamp()),
            arena_match_id: registration.and_then(|r| r.arena_match_id.clone()),
            status: input.status,
            days_completed: outcome.map_or(0, |o| o.days_completed),
            revisions: outcome.map_or_else(Vec::new, |o| o.revisions.clone()),
            day_steps: outcome.map_or_else(Vec::new, |o| o.day_steps.clone()),
            day_seconds: outcome.map_or_else(Vec::new, |o| o.day_seconds.clone()),
            days: outcome.map_or_else(Vec::new, |o| o.days.clone()),
            kinds: outcome.map_or_else(Vec::new, |o| o.kinds.clone()),
            result_ref: None,
            error: input.error.map(|e| input.profile.token.redact(&e)),
            recorded_at_unix: Utc::now().timestamp(),
        }
    }
}

/// Classify a coordinator-loop result into (status, error text).
pub fn classify_loop_result(
    result: &Result<CoordinatorOutcome, CoordinatorLoopError>,
) -> (ArenaRunStatus, Option<String>) {
    match result {
        Ok(_) => (ArenaRunStatus::Completed, None),
        Err(CoordinatorLoopError::Rejected { day, revision }) => {
            (ArenaRunStatus::Rejected, Some(format!("rejected day={day} revision={revision}")))
        }
        Err(e) => (ArenaRunStatus::Error, Some(e.to_string())),
    }
}

/// Append one record to the evidence JSONL (creating parents as needed).
///
/// # Errors
/// [`ArenaError::Evidence`] on IO or serialisation failure.
pub fn append_evidence(path: &Path, record: &ArenaRunRecord) -> Result<(), ArenaError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| ArenaError::Evidence(e.to_string()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| ArenaError::Evidence(e.to_string()))?;
    let line = serde_json::to_string(record).map_err(|e| ArenaError::Evidence(e.to_string()))?;
    writeln!(file, "{line}").map_err(|e| ArenaError::Evidence(e.to_string()))
}

/// Default evidence path (`target/arena/runs.jsonl`).
#[must_use]
pub fn default_evidence_path() -> PathBuf {
    PathBuf::from(ARENA_EVIDENCE_DEFAULT_PATH)
}

/// Resolve the evidence `source_commit_sha` for an arena run.
///
/// Precedence:
///
/// 1. `PROCON_SOURCE_COMMIT_SHA` (exported by the `just` targets and by
///    `procon-eval`; blank counts as unset),
/// 2. `git rev-parse HEAD` in the process working directory — the pattern
///    `procon-eval` already uses. The live smoke (Issue #222) ran
///    `cargo run` directly, so the env var was absent and the evidence
///    recorded `"unknown"`;
/// 3. `"unknown"` as the last resort, so an ad-hoc run still writes a
///    well-formed record.
#[must_use]
pub fn source_commit_sha() -> String {
    resolve_source_commit_sha(std::env::var("PROCON_SOURCE_COMMIT_SHA").ok(), git_head())
}

/// Pure resolution core of [`source_commit_sha`] — `env` and `git` are
/// injected so both branches are unit tested without mutating the process
/// environment (edition 2024 makes `env::set_var` unsafe, and mutating it
/// in parallel tests is racy).
#[must_use]
fn resolve_source_commit_sha(env: Option<String>, git: Option<String>) -> String {
    env.filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_owned())
        .or(git)
        .unwrap_or_else(|| "unknown".to_owned())
}

/// `git rev-parse HEAD` in the process working directory. `None` when git
/// is unavailable or the directory is not a worktree — same pattern as
/// `bench/src/bin/procon-eval.rs::git_head`.
fn git_head() -> Option<String> {
    let out = std::process::Command::new("git").args(["rev-parse", "HEAD"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: &str = "sentinel-arena-token-do-not-log";

    fn fixed_instant() -> DateTime<Utc> {
        // Chosen so `timestamp % 900 == 840` — i.e. exactly 60s before the
        // next `:00/:15/:30/:45` boundary.
        DateTime::<Utc>::from_timestamp(1_790_000_040, 0).expect("valid timestamp")
    }

    #[test]
    fn secret_redacts_debug_display_and_text() {
        let secret = Secret::new(SENTINEL.to_owned());
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert!(!format!("{secret:?}").contains(SENTINEL));
        let msg = secret.redact(&format!("token={SENTINEL} rejected"));
        assert_eq!(msg, "token=[REDACTED] rejected");
        // Empty secret: redaction is a no-op rather than a blanket wipe.
        assert_eq!(Secret::new(String::new()).redact("keep"), "keep");
    }

    #[test]
    fn profile_debug_never_shows_the_token() {
        let profile = ArenaProfile {
            server_url: ARENA_SERVER_DEFAULT.to_owned(),
            token: Secret::new(SENTINEL.to_owned()),
        };
        let rendered = format!("{profile:?}");
        assert!(!rendered.contains(SENTINEL), "leaked token: {rendered}");
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
    }

    #[test]
    fn resolve_profile_precedence_and_defaults() {
        // Explicit beats env beats default.
        let profile = resolve_profile(
            Some("http://explicit".to_owned()),
            Some("explicit-token".to_owned()),
            ArenaEnv { server: Some("http://env".to_owned()), token: Some("env-token".to_owned()) },
        )
        .expect("resolves");
        assert_eq!(profile.server_url, "http://explicit");
        assert_eq!(profile.token.expose(), "explicit-token");

        // Env fills in when nothing explicit is given.
        let profile = resolve_profile(
            None,
            None,
            ArenaEnv { server: Some("http://env".to_owned()), token: Some("env-token".to_owned()) },
        )
        .expect("resolves");
        assert_eq!(profile.server_url, "http://env");
        assert_eq!(profile.token.expose(), "env-token");

        // Server default applies; token is mandatory.
        let err =
            resolve_profile(None, None, ArenaEnv::default()).expect_err("a token is required");
        let text = err.to_string();
        assert!(text.contains("PROCON_ARENA_TOKEN"), "{text}");

        let profile = resolve_profile(
            None,
            None,
            ArenaEnv { server: None, token: Some(SENTINEL.to_owned()) },
        )
        .expect("token alone is enough");
        assert_eq!(profile.server_url, ARENA_SERVER_DEFAULT);
        // The error path renders without the secret, too.
        let err = resolve_profile(None, Some("   ".to_owned()), ArenaEnv::default())
            .expect_err("blank token counts as unset");
        assert!(!format!("{err}").contains(SENTINEL));
    }

    #[test]
    fn next_match_window_walks_the_quarter_hour_cadence() {
        let lead = Duration::from_secs(u64::try_from(ARENA_PRE_MATCH_LEAD_SECS).unwrap_or(180));
        // One minute before :45 → targets :45, opens 3 minutes earlier.
        let now = DateTime::<Utc>::from_timestamp(1_790_000_040, 0).expect("ts");
        let w = next_match_window(now, lead);
        assert_eq!(w.starts_at.timestamp() % ARENA_MATCH_PERIOD_SECS, 0);
        assert_eq!(w.starts_at - now, chrono::TimeDelta::seconds(60));
        assert_eq!(w.starts_at - w.opens_at, chrono::TimeDelta::seconds(180));

        // Exactly on a boundary → join the match starting now.
        let on_boundary = w.starts_at;
        let w2 = next_match_window(on_boundary, lead);
        assert_eq!(w2.starts_at, on_boundary);

        // One second after the boundary → roll to the next quarter hour.
        let just_after = on_boundary + chrono::TimeDelta::seconds(1);
        let w3 = next_match_window(just_after, lead);
        assert_eq!(
            w3.starts_at - just_after,
            chrono::TimeDelta::seconds(ARENA_MATCH_PERIOD_SECS - 1)
        );

        // Zero lead collapses the window onto the start.
        let w4 = next_match_window(now, Duration::ZERO);
        assert_eq!(w4.opens_at, w4.starts_at);
    }

    #[test]
    fn registration_budget_is_bounded_by_window_and_cap() {
        let lead = Duration::from_secs(180);
        let now = fixed_instant();
        let window = next_match_window(now, lead);
        let interval = Duration::from_secs(10);

        // 60s until start at a 10s interval → 7 attempts (60s/10s + 1).
        let mut w = window;
        w.opens_at = now;
        w.starts_at = now + chrono::TimeDelta::seconds(60);
        assert_eq!(registration_attempt_budget(w, now, interval, 60), 7);

        // Cap wins over a long window.
        w.starts_at = now + chrono::TimeDelta::seconds(3600);
        assert_eq!(registration_attempt_budget(w, now, interval, 60), 60);

        // Already past the start → exactly one attempt, never zero.
        w.starts_at = now - chrono::TimeDelta::seconds(5);
        w.opens_at = now - chrono::TimeDelta::seconds(185);
        assert_eq!(registration_attempt_budget(w, now, interval, 60), 1);
    }

    #[test]
    fn record_round_trips_without_secret_fields() {
        let profile = ArenaProfile {
            server_url: ARENA_SERVER_DEFAULT.to_owned(),
            token: Secret::new(SENTINEL.to_owned()),
        };
        let registration = RegistrationOutcome {
            window: next_match_window(fixed_instant(), Duration::from_secs(180)),
            registered_at: fixed_instant(),
            attempts: 3,
            arena_match_id: Some("m-1234".to_owned()),
        };
        let outcome = CoordinatorOutcome {
            days_completed: 4,
            revisions: vec![0, 1, 2, 3],
            ..CoordinatorOutcome::default()
        };
        let record = ArenaRunRecord::build(ArenaRecordInput {
            profile: &profile,
            candidate_id: Some("multiloop:kmeans=4:seed=42"),
            solver: ArenaSolverConfig {
                mode: "candidate".to_owned(),
                solver: "multiloop".to_owned(),
                loop_initial_strategy: None,
                loop_iterations: None,
            },
            team_id: "mvp-player",
            source_commit_sha: "deadbeef".to_owned(),
            window_starts_at_unix: registration.window.starts_at.timestamp(),
            registration: Some(&registration),
            outcome: Some(&outcome),
            status: ArenaRunStatus::Completed,
            error: Some(format!("server said token={SENTINEL}")),
        });

        let json = serde_json::to_string(&record).expect("serialises");
        assert!(!json.contains(SENTINEL), "token leaked into evidence: {json}");
        assert!(json.contains("[REDACTED]"), "{json}");
        assert_eq!(record.record_kind, "arena-run");
        assert_eq!(record.candidate_id.as_deref(), Some("multiloop:kmeans=4:seed=42"));
        assert_eq!(record.source_commit_sha, "deadbeef");
        assert_eq!(record.days_completed, 4);
        assert_eq!(record.registration_attempts, Some(3));
        assert_eq!(record.arena_match_id.as_deref(), Some("m-1234"));

        // Round-trips so `read`-side tooling can validate the schema.
        let back: ArenaRunRecord = serde_json::from_str(&json).expect("parses back");
        assert_eq!(back, record);
    }

    #[test]
    fn loop_results_classify_into_arena_statuses() {
        let ok: Result<CoordinatorOutcome, CoordinatorLoopError> = Ok(CoordinatorOutcome {
            days_completed: 1,
            revisions: vec![0],
            ..CoordinatorOutcome::default()
        });
        assert_eq!(classify_loop_result(&ok).0, ArenaRunStatus::Completed);

        let rejected: Result<CoordinatorOutcome, CoordinatorLoopError> =
            Err(CoordinatorLoopError::Rejected { day: 2, revision: -1 });
        let (status, err) = classify_loop_result(&rejected);
        assert_eq!(status, ArenaRunStatus::Rejected);
        assert!(err.unwrap().contains("revision=-1"));

        // Synthetic drift on a *plural / unknown* path: the arena SPA
        // fallback serves HTML there, and the classification must not care
        // which path drifted (Issue #222 — `/settings` vs `/setting`).
        let drift = ArenaError::ApiDrift {
            path: "/settings".to_owned(),
            detail: "expected JSON, got content-type text/html".to_owned(),
        };
        assert_eq!(drift.status(), ArenaRunStatus::ApiDrift);
        let reg = ArenaError::NotRegistered { message: "budget".to_owned() };
        assert_eq!(reg.status(), ArenaRunStatus::RegistrationFailed);
    }

    #[test]
    fn registration_path_is_the_official_singular_setting() {
        // The plural `/settings` is only the arena SPA fallback (HTML):
        // registering there is exactly what the live smoke hit (Issue #222).
        assert_eq!(ARENA_REGISTRATION_PATH, "/setting");
    }

    #[test]
    fn lobby_registration_messages_parse_only_when_complete() {
        // The exact live shape (recon 2026-09-27, Issue #222).
        let parsed = parse_lobby_registration(
            "lobby: you are registered for match 469 departing at 1790001234",
        )
        .expect("the live lobby confirmation parses");
        assert_eq!(parsed.match_id, "469");
        assert_eq!(parsed.departing_at_unix, 1_790_001_234);

        // A plain "not yet" 403 keeps today's bounded retry.
        assert!(parse_lobby_registration("試合がまだ開始されていません").is_none());

        // Anything unrecognised fails towards `NotRegistered`, never
        // towards a guessed success.
        assert!(parse_lobby_registration("lobby: you are registered for match 469").is_none());
        assert!(
            parse_lobby_registration("you are registered for match  departing at 1790001234")
                .is_none(),
            "empty match id must not register"
        );
        assert!(
            parse_lobby_registration("you are registered for match 469 departing at soon")
                .is_none(),
            "non-numeric departure must not register"
        );
        assert!(
            parse_lobby_registration("you are registered for match 469 departing at -5").is_none(),
            "negative departure must not register"
        );
        assert!(
            parse_lobby_registration("registered for match 469 departing at 1790001234").is_none()
        );
    }

    #[test]
    fn source_commit_sha_prefers_the_env_override() {
        assert_eq!(
            resolve_source_commit_sha(Some("feedbeef-179".to_owned()), Some("git-sha".to_owned())),
            "feedbeef-179"
        );
        // Blank / whitespace-only env counts as unset and falls through.
        assert_eq!(
            resolve_source_commit_sha(Some("  ".to_owned()), Some("git-sha".to_owned())),
            "git-sha"
        );
    }

    #[test]
    fn source_commit_sha_falls_back_to_the_git_head_of_this_checkout() {
        // Tests run inside the repository worktree, so `git rev-parse HEAD`
        // resolves — the branch the live smoke missed (Issue #222).
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git available in the dev/CI environment");
        assert!(out.status.success(), "tests must run inside the git worktree");
        let expected = String::from_utf8(out.stdout).expect("utf8").trim().to_owned();

        let sha = resolve_source_commit_sha(None, git_head());
        assert_eq!(sha, expected, "env unset ⇒ evidence records the git head");
        assert_ne!(sha, "unknown");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "not a sha: {sha}");
    }

    #[test]
    fn source_commit_sha_is_unknown_only_when_nothing_is_available() {
        assert_eq!(resolve_source_commit_sha(None, None), "unknown");
        assert_eq!(resolve_source_commit_sha(Some(String::new()), None), "unknown");
    }

    /// Build a synthetic `days`-day record with `agents` agents per day
    /// (the official team maximum), sized so every per-day value keeps a
    /// constant digit width — the growth assertions below then measure the
    /// *shape* of the schema rather than the luck of a digit.
    fn record_with_days(days: usize, agents: usize) -> ArenaRunRecord {
        use crate::coordinator_loop::{DayAgentState, DayMovement};
        // Per-day agent state is the wire spelling (`0 | 1`); the record's
        // top-level `kinds` uses the engine spelling imported above.
        use protocol::match_config::AgentKind as WireAgentKind;

        let profile = ArenaProfile {
            server_url: ARENA_SERVER_DEFAULT.to_owned(),
            token: Secret::new(SENTINEL.to_owned()),
        };
        let summaries: Vec<DaySummary> = (0..days)
            .map(|d| DaySummary {
                day: u32::try_from(d).unwrap_or(0),
                agents: (0..agents)
                    .map(|a| DayAgentState {
                        kind: if a % 2 == 0 {
                            WireAgentKind::KIND_0
                        } else {
                            WireAgentKind::KIND_1
                        },
                        pos: 40 + i32::try_from(a).unwrap_or(0) * 3,
                        fuel: 900 - i32::try_from(a).unwrap_or(0) * 7,
                    })
                    .collect(),
                movement: (0..agents)
                    .map(|a| DayMovement {
                        moves: u32::try_from(4 + a).unwrap_or(0),
                        waits: u32::try_from(20_usize.saturating_sub(a)).unwrap_or(0),
                        runs: u32::try_from(1 + a % 3).unwrap_or(0),
                        dirs: [1, 0, 2, 0, 1, 0],
                    })
                    .collect(),
            })
            .collect();
        let outcome = CoordinatorOutcome {
            days_completed: days,
            revisions: vec![7; days],
            day_steps: vec![100; days],
            day_seconds: vec![60; days],
            days: summaries,
            // Constant across the size-guard's three records, so the linear
            // growth assertion keeps measuring days only.
            kinds: (0..agents)
                .map(|a| if a % 2 == 0 { AgentKind::Patrol } else { AgentKind::Supply })
                .collect(),
        };
        ArenaRunRecord::build(ArenaRecordInput {
            profile: &profile,
            candidate_id: Some("multiloop:kmeans=4:seed=42"),
            solver: ArenaSolverConfig {
                mode: "candidate".to_owned(),
                solver: "multiloop".to_owned(),
                loop_initial_strategy: None,
                loop_iterations: None,
            },
            team_id: "mvp-player",
            source_commit_sha: "deadbeef".to_owned(),
            window_starts_at_unix: 1_790_000_100,
            registration: None,
            outcome: Some(&outcome),
            status: ArenaRunStatus::Completed,
            error: None,
        })
    }

    /// Size guard (Issue #236 AC): a synthetic **9-day** record with the
    /// official 8-agent maximum must stay one modest JSONL line, each day
    /// entry must cost a bounded number of bytes, and the growth must be
    /// exactly linear in the day count — i.e. the schema's field count
    /// scales with days only (no raw plans, no per-step arrays).
    #[test]
    fn nine_day_record_stays_one_modest_jsonl_line() {
        use serde_json::Value;

        const AGENTS: usize = 8;
        const SIZE_CAP: usize = 16 * 1024;
        /// Ceiling for a single day's entry: 8 agents × (state + movement)
        /// at tens of bytes each, with headroom for digit width.
        const PER_DAY_CAP: usize = 1024;

        let record = record_with_days(9, AGENTS);
        assert_eq!(record.days.len(), 9, "one summary per played day");
        assert_eq!(record.day_steps.len(), 9, "budget recorded once, per day");
        assert_eq!(record.day_seconds.len(), 9);

        let json = serde_json::to_string(&record).expect("serialises");
        assert!(json.len() < SIZE_CAP, "9-day record is {} bytes, cap is {SIZE_CAP}", json.len());
        // Still exactly one JSONL line.
        assert!(!json.contains('\n'), "one record must be one line");

        // Fixed field counts: a day entry is `day` / `agents` / `movement`,
        // an agent state is `kind` / `pos` / `fuel`, a movement summary is
        // `moves` / `waits` / `runs` / `dirs`. Nothing grows with plan
        // length or day index.
        let value: serde_json::Value = serde_json::from_str(&json).expect("parses");
        let days = value.get("days").and_then(Value::as_array).expect("days array");
        for (idx, day) in days.iter().enumerate() {
            let entry = day.as_object().expect("day entry is an object");
            assert_eq!(entry.len(), 3, "day entry fields: {entry:?}");
            let agents = entry.get("agents").and_then(Value::as_array).expect("agents");
            let movement = entry.get("movement").and_then(Value::as_array).expect("movement");
            assert_eq!(agents.len(), AGENTS);
            assert_eq!(movement.len(), AGENTS);
            for a in agents {
                assert_eq!(a.as_object().expect("agent is an object").len(), 3, "kind/pos/fuel");
            }
            for m in movement {
                assert_eq!(
                    m.as_object().expect("movement is an object").len(),
                    4,
                    "moves/waits/runs/dirs"
                );
            }
            let bytes = serde_json::to_vec(day).expect("serialises");
            assert!(
                bytes.len() < PER_DAY_CAP,
                "day {idx} entry is {} bytes, cap {PER_DAY_CAP}",
                bytes.len()
            );
        }

        // Exactly linear in days: 9 days cost 8× the (2 − 1) day delta.
        let one = serde_json::to_string(&record_with_days(1, AGENTS)).expect("serialises").len();
        let two = serde_json::to_string(&record_with_days(2, AGENTS)).expect("serialises").len();
        let nine = json.len();
        let per_extra_day = two - one;
        assert!(per_extra_day > 0, "each day must add bytes");
        assert_eq!(
            nine - one,
            per_extra_day * 8,
            "growth must scale with days only: {one} → {two} → {nine}"
        );
    }

    #[test]
    fn append_evidence_creates_parents_and_appends_lines() {
        let dir =
            std::env::temp_dir().join(format!("procon-arena-evidence-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested/runs.jsonl");

        let profile = ArenaProfile {
            server_url: ARENA_SERVER_DEFAULT.to_owned(),
            token: Secret::new(SENTINEL.to_owned()),
        };
        let record = ArenaRunRecord::build(ArenaRecordInput {
            profile: &profile,
            candidate_id: None,
            solver: ArenaSolverConfig {
                mode: "cli-solver".to_owned(),
                solver: "wait".to_owned(),
                loop_initial_strategy: None,
                loop_iterations: None,
            },
            team_id: "mvp-player",
            source_commit_sha: "unknown".to_owned(),
            window_starts_at_unix: 0,
            registration: None,
            outcome: None,
            status: ArenaRunStatus::RegistrationFailed,
            error: Some(format!("boom {SENTINEL}")),
        });

        append_evidence(&path, &record).expect("first append");
        append_evidence(&path, &record).expect("second append");
        let text = std::fs::read_to_string(&path).expect("readable");
        assert_eq!(text.lines().count(), 2, "two JSONL lines");
        assert!(!text.contains(SENTINEL), "token in evidence file");
        let parsed: ArenaRunRecord =
            serde_json::from_str(text.lines().next().expect("line")).expect("parse");
        assert_eq!(parsed.status, ArenaRunStatus::RegistrationFailed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
