//! Async HTTP client for the Procon contest server.
//!
//! Wraps [`reqwest::Client`] and adds:
//!
//! - `Procon-Token` header on every request.
//! - Per-request timeout (default 5 seconds).
//! - GET-only retry-on-transient-failure with exponential backoff
//!   (`100 ms, 200 ms, 400 ms, 800 ms`, max 4 retries).
//!
//! The wire payload types ([`AgentKind`](crate::match_config::AgentKind),
//! [`MatchState`](crate::match_config::MatchState), error envelopes) are
//! shared with the rest of the protocol crate and live in
//! [`crate::match_config`].

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::match_config::{
    AccessTimeError, ActionAccepted, ActionPlanBody, AgentKind, AgentKindsBody, AuthError,
    MatchSetting, MatchState,
};

/// Default timeout for a single request attempt.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Number of retries performed after the initial attempt fails with a
/// transient error (i.e. `1 + MAX_RETRIES` total attempts).
pub const MAX_RETRIES: usize = 4;

/// Backoff schedule, in milliseconds, applied between retries.
///
/// Index `i` is the delay before retry attempt `i + 1`. Total worst-case
/// backoff before giving up is `100 + 200 + 400 + 800 = 1500 ms`.
pub const BACKOFF_MILLIS: [u64; MAX_RETRIES] = [100, 200, 400, 800];

/// Header name carrying the player authentication token.
///
/// Source: Procon 37 `OpenAPI` `PlayerAuth.tokenHeader` parameter.
const TOKEN_HEADER: &str = "Procon-Token";

/// Successful acknowledgement of `POST /agent`.
///
/// The official `OpenAPI` schema declares a `200` response with no body
/// schema. We still wrap the call in a typed result so that callers can
/// pattern-match on the outcome without inspecting HTTP details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitAck {
    /// HTTP status code returned by the server (always `200` on success).
    pub status: u16,
}

impl SubmitAck {
    /// Convenience constructor for the canonical success case.
    #[must_use]
    pub const fn ok() -> Self {
        Self { status: 200 }
    }
}

/// Retry behaviour for transient failures.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Maximum number of retries after the initial attempt.
    pub max_retries: usize,
    /// Backoff schedule between retries, in milliseconds. Length should be
    /// at least `max_retries`; entries beyond that are ignored.
    pub backoff_millis: &'static [u64],
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: MAX_RETRIES, backoff_millis: &BACKOFF_MILLIS }
    }
}

impl PartialEq for RetryPolicy {
    fn eq(&self, other: &Self) -> bool {
        self.max_retries == other.max_retries && self.backoff_millis == other.backoff_millis
    }
}

impl Eq for RetryPolicy {}

/// Errors produced by [`MatchClient`].
#[derive(Debug, Error)]
pub enum HttpClientError {
    /// A non-transient HTTP error (4xx other than 408/429, or a final
    /// exhausted-retry on transient errors).
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },
    /// Authentication failure (`401`). Never retried.
    #[error("authentication failed: {message}")]
    Auth { message: String },
    /// Out-of-window failure (`403`). Never retried.
    #[error("access time error: {message}")]
    AccessTime { message: String },
    /// The server returned a body we could not decode.
    #[error("failed to decode response body: {0}")]
    Decode(#[from] serde_json::Error),
    /// The endpoint returned a non-JSON body where the protocol promises
    /// JSON — typically an HTML SPA shell served with HTTP 200 (the
    /// unofficial arena answers unauthenticated requests that way). Never
    /// retried and never re-shaped: the client fails closed so a drifted
    /// API cannot be papered over with a guessed request/response shape.
    #[error("API drift on {path}: {detail}")]
    ApiDrift { path: String, detail: String },
    /// The underlying transport failed (timeout, connect, DNS, ...).
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    /// The URL supplied to the client is malformed.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
}

impl From<AuthError> for HttpClientError {
    fn from(value: AuthError) -> Self {
        Self::Auth { message: value.message }
    }
}

impl From<AccessTimeError> for HttpClientError {
    fn from(value: AccessTimeError) -> Self {
        Self::AccessTime { message: value.message }
    }
}

/// One HTTP request and what came back, as seen on the wire (Issue #396).
///
/// Built only when an [`HttpObserver`] is attached. The token never leaves
/// the client: its header value is replaced by [`MASKED`] here, before the
/// observer sees anything.
#[derive(Debug, Clone)]
pub struct HttpExchange {
    /// Identifies this exchange within the client (and its clones), from 1.
    /// [`HttpObserver::decode_failed`] refers to an exchange by this id.
    pub id: u64,
    /// `GET` / `POST`.
    pub method: String,
    /// The full request URL.
    pub url: String,
    /// The headers this client set (`Procon-Token` masked, and
    /// `Content-Type` for a POST). The HTTP library adds `host`, `accept`
    /// and `content-length` below this layer; those are not listed.
    pub request_headers: Vec<(String, String)>,
    /// The request body, byte for byte. `None` for a GET.
    pub request_body: Option<Vec<u8>>,
    /// 1-based attempt within one client call (a GET may be repeated on a
    /// transient failure; a POST never is).
    pub attempt: u32,
    /// Wall clock when the request was handed to the HTTP library.
    pub started_at: std::time::SystemTime,
    /// Time until the response body was complete, or until the failure.
    pub elapsed: Duration,
    /// What came back.
    pub outcome: HttpOutcome,
}

/// The result of one [`HttpExchange`].
#[derive(Debug, Clone)]
pub enum HttpOutcome {
    /// The server answered and the whole body arrived.
    Response {
        /// HTTP status code.
        status: u16,
        /// Response headers, in the order received.
        headers: Vec<(String, String)>,
        /// The response body, byte for byte.
        body: Vec<u8>,
    },
    /// No complete response.
    Failed {
        /// `timeout`, `connect`, `body` (the body broke off) or `request`.
        kind: &'static str,
        /// The HTTP library's error text.
        error: String,
        /// The status line and headers, when they had arrived before the
        /// body broke off.
        partial: Option<(u16, Vec<(String, String)>)>,
    },
}

/// Placeholder written instead of a secret header value.
pub const MASKED: &str = "[MASKED]";

/// Receives every HTTP exchange of a [`MatchClient`] (Issue #396).
///
/// Implementations must return quickly and must not block: they are called
/// on the task that is talking to the contest server.
pub trait HttpObserver: Send + Sync + std::fmt::Debug {
    /// One request finished (with a response or with a failure).
    fn exchange(&self, exchange: HttpExchange);

    /// The response of exchange `exchange_id` arrived but could not be
    /// decoded into the type the protocol promises.
    fn decode_failed(&self, exchange_id: u64, error: &str);
}

/// Asynchronous HTTP client for the contest server.
///
/// `MatchClient` is cheap to clone: the underlying [`reqwest::Client`]
/// internally holds an `Arc` and is designed for concurrent use.
#[derive(Debug, Clone)]
pub struct MatchClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
    retry_policy: RetryPolicy,
    request_timeout: Duration,
    /// `None` on every default path: no observer, no change to how a
    /// request is sent or how its body is read.
    observer: Option<std::sync::Arc<dyn HttpObserver>>,
    /// Source of [`HttpExchange::id`], shared by clones of this client.
    exchange_ids: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl MatchClient {
    /// Build a new client targeting `base_url` (e.g. `https://game.example.com`)
    /// with the given player token.
    ///
    /// # Errors
    ///
    /// Returns [`HttpClientError::Transport`] if the underlying
    /// [`reqwest::Client`] builder fails, or [`HttpClientError::InvalidUrl`]
    /// if `base_url` cannot be parsed.
    pub fn new(
        base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Result<Self, HttpClientError> {
        Self::builder(base_url, token).build()
    }

    /// Create a configurable builder.
    #[must_use]
    pub fn builder(base_url: impl Into<String>, token: impl Into<String>) -> MatchClientBuilder {
        MatchClientBuilder {
            base_url: base_url.into(),
            token: token.into(),
            retry_policy: RetryPolicy::default(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            observer: None,
        }
    }

    /// Tell the observer, if any, that decoding the response of `exchange`
    /// failed.
    fn observed<T>(
        &self,
        exchange: u64,
        result: Result<T, HttpClientError>,
    ) -> Result<T, HttpClientError> {
        if let (
            Some(observer),
            Err(e @ (HttpClientError::Decode(_) | HttpClientError::ApiDrift { .. })),
        ) = (self.observer.as_ref(), result.as_ref())
        {
            observer.decode_failed(exchange, &e.to_string());
        }
        result
    }

    /// Submit the per-agent [`AgentKind`] assignments to `POST /agent`.
    ///
    /// # Errors
    ///
    /// See [`HttpClientError`].
    pub async fn submit_agent(&self, kinds: Vec<AgentKind>) -> Result<SubmitAck, HttpClientError> {
        let url = self.endpoint("/agent")?;
        let body: AgentKindsBody = kinds;
        let body_bytes = serde_json::to_vec(&body)?;
        let (response, exchange) =
            self.execute(reqwest::Method::POST, &url, Some(body_bytes), true).await?;
        self.observed(exchange, handle_submit_response(response).await)
    }

    /// Fetch the current [`MatchState`] snapshot from `GET /`.
    ///
    /// # Errors
    ///
    /// See [`HttpClientError`].
    pub async fn get_match_state(&self) -> Result<MatchState, HttpClientError> {
        let url = self.endpoint("/")?;
        let (response, exchange) =
            self.execute_with_retry(reqwest::Method::GET, &url, None).await?;
        self.observed(exchange, Self::decode_match_state(response).await)
    }

    /// Fetch the static match configuration from `GET /setting`.
    ///
    /// This endpoint is only available before the match begins; once the
    /// match is in progress the server returns `403` with an
    /// [`AccessTimeError`](crate::match_config::AccessTimeError) body.
    ///
    /// # Errors
    ///
    /// See [`HttpClientError`].
    pub async fn get_match_setting(&self) -> Result<MatchSetting, HttpClientError> {
        let url = self.endpoint("/setting")?;
        let (response, exchange) =
            self.execute_with_retry(reqwest::Method::GET, &url, None).await?;
        self.observed(exchange, Self::decode_setting_response(response).await)
    }

    /// Submit the day's action plan for all agents to `POST /`.
    ///
    /// The official wire format returns `200` with `{ "revision": <i32> }`
    /// on both acceptance (`revision >= 0`) and rejection
    /// (`revision < 0`). To distinguish transport-level errors from a
    /// server-side rejection, the negative-revision case is surfaced as
    /// [`HttpClientError::Http`] with `status = 200` and a `rejected: ...`
    /// message, and is **not** retried (it slips past the retry loop
    /// because `is_success()` is true; the rejection is detected only
    /// after decoding the body).
    ///
    /// # Errors
    ///
    /// See [`HttpClientError`].
    pub async fn submit_actions(
        &self,
        plan: ActionPlanBody,
    ) -> Result<ActionAccepted, HttpClientError> {
        let url = self.endpoint("/")?;
        let body_bytes = serde_json::to_vec(&plan)?;
        let (response, exchange) =
            self.execute_with_retry(reqwest::Method::POST, &url, Some(body_bytes)).await?;
        self.observed(exchange, Self::decode_action_accepted(response).await)
    }

    /// Fetch an arbitrary protocol path and decode its body as `T`.
    ///
    /// Used by the arena registration phase (`GET /setting` polled inside
    /// the pre-match window — Issue #222; the arena notice's plural
    /// `/settings` is only the SPA fallback), which shares the same client,
    /// token header, retry policy and error classification with
    /// [`Self::get_match_setting`]. The response body must be JSON: an HTML
    /// shell (the arena's SPA fallback for unauthenticated / unknown paths)
    /// is reported as [`HttpClientError::ApiDrift`] and never retried.
    ///
    /// # Errors
    ///
    /// See [`HttpClientError`]; in particular [`HttpClientError::ApiDrift`]
    /// when the endpoint answers with a non-JSON body.
    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, HttpClientError> {
        let url = self.endpoint(path)?;
        let (response, exchange) =
            self.execute_with_retry(reqwest::Method::GET, &url, None).await?;
        self.observed(exchange, Self::decode_success_json(response, path).await)
    }

    /// Decode a `MatchSetting` from a successful response.
    async fn decode_setting_response(
        response: reqwest::Response,
    ) -> Result<MatchSetting, HttpClientError> {
        Self::decode_success_json(response, "/setting").await
    }

    /// Decode an `ActionAccepted` from a `POST /` response.
    ///
    /// The official spec returns `200` even on rejection (`revision < 0`).
    /// Such rejections are surfaced as
    /// [`HttpClientError::Http`] with `status = 200` and a `rejected: ...`
    /// message so that callers can distinguish them from transport errors
    /// without retrying.
    async fn decode_action_accepted(
        response: reqwest::Response,
    ) -> Result<ActionAccepted, HttpClientError> {
        let accepted: ActionAccepted = Self::decode_success_json(response, "POST /").await?;
        if accepted.revision < 0 {
            return Err(HttpClientError::Http {
                status: 200,
                message: format!("rejected: revision={}", accepted.revision),
            });
        }
        Ok(accepted)
    }

    /// Decode a `MatchState` from a successful response.
    async fn decode_match_state(
        response: reqwest::Response,
    ) -> Result<MatchState, HttpClientError> {
        Self::decode_success_json(response, "/").await
    }

    /// Decode a JSON body from a successful response, **failing closed** on
    /// anything that is not the JSON document the protocol promises.
    ///
    /// Status codes alone are not trusted (the unofficial arena serves the
    /// SPA HTML shell with HTTP 200), so two checks run before the serde
    /// decode:
    ///
    /// 1. a present `Content-Type` that does not mention JSON ⇒
    ///    [`HttpClientError::ApiDrift`];
    /// 2. a body whose first byte is not a JSON value starter (after
    ///    trimming whitespace) — the classic HTML shell served without a
    ///    content type — ⇒ [`HttpClientError::ApiDrift`].
    ///
    /// Non-success statuses keep the existing classification path
    /// ([`classify_error`]). The drift detail is truncated to one short
    /// line so it is safe to persist into state snapshots and evidence.
    async fn decode_success_json<T: serde::de::DeserializeOwned>(
        response: reqwest::Response,
        path: &str,
    ) -> Result<T, HttpClientError> {
        let status = response.status();
        if !status.is_success() {
            return Err(classify_error(status, response).await);
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !content_type.is_empty() && !content_type.contains("json") {
            return Err(HttpClientError::ApiDrift {
                path: path.to_owned(),
                detail: format!("expected JSON, got content-type {content_type}"),
            });
        }
        let body = response.bytes().await?;
        let trimmed = trim_ascii_start(&body);
        let json_starter = matches!(
            trimmed.first().copied(),
            Some(b'{' | b'[' | b'"' | b't' | b'f' | b'n' | b'-' | b'0'..=b'9')
        );
        if !json_starter {
            return Err(HttpClientError::ApiDrift {
                path: path.to_owned(),
                detail: format!(
                    "expected JSON, got a non-JSON body starting with {:?}",
                    snippet(trimmed)
                ),
            });
        }
        serde_json::from_slice(&body).map_err(HttpClientError::Decode)
    }

    /// Compute the full URL for an endpoint path.
    fn endpoint(&self, path: &str) -> Result<String, HttpClientError> {
        let trimmed = path.trim_start_matches('/');
        let base = self.base_url.trim_end_matches('/');
        let combined = format!("{base}/{trimmed}");
        // Validate parseability so callers fail fast.
        reqwest::Url::parse(&combined).map_err(|e| HttpClientError::InvalidUrl(e.to_string()))?;
        Ok(combined)
    }

    /// Dispatch a request whose response body the caller will read.
    async fn execute_with_retry(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Vec<u8>>,
    ) -> Result<(reqwest::Response, u64), HttpClientError> {
        self.execute(method, url, body, false).await
    }

    /// Dispatch a request, retrying on transient failures.
    ///
    /// `body_unused_on_success` is set by callers that never read the body
    /// of a 2xx answer (`POST /agent`). It only matters with an observer,
    /// which reads every body in order to log it: a 2xx whose body breaks
    /// off must then still count as the success it is without the observer.
    async fn execute(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Vec<u8>>,
        body_unused_on_success: bool,
    ) -> Result<(reqwest::Response, u64), HttpClientError> {
        // Id of the latest observed exchange; 0 without an observer.
        let mut exchange = 0_u64;
        // POST outcomes can be ambiguous: only reads may be replayed.
        let max_retries =
            if method == reqwest::Method::GET { self.retry_policy.max_retries } else { 0 };
        let mut last_error: Option<HttpClientError> = None;
        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay =
                    self.retry_policy.backoff_millis.get(attempt - 1).copied().unwrap_or(800);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }

            let mut builder = self
                .http
                .request(method.clone(), url)
                .header(TOKEN_HEADER, &self.token)
                .timeout(self.request_timeout);
            if let Some(bytes) = &body {
                builder = builder
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(bytes.clone());
            }

            let sent = match self.observer.as_deref() {
                None => builder.send().await.map_err(|error| SendFailure { error, in_body: false }),
                Some(observer) => {
                    exchange =
                        self.exchange_ids.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    self.send_observed(
                        observer,
                        exchange,
                        builder,
                        &method,
                        url,
                        body.as_deref(),
                        attempt,
                        body_unused_on_success,
                    )
                    .await
                }
            };
            match sent {
                Ok(response) => {
                    if Self::is_transient_status(response.status()) && attempt < max_retries {
                        last_error = Some(HttpClientError::Http {
                            status: response.status().as_u16(),
                            message: "transient failure".to_owned(),
                        });
                        continue;
                    }
                    return Ok((response, exchange));
                }
                Err(SendFailure { error: err, in_body }) => {
                    // A body that broke off is never repeated here: without
                    // an observer that failure surfaces in the decoder,
                    // after this loop, and the caller decides what to do.
                    let transient =
                        !in_body && (err.is_timeout() || err.is_connect() || err.is_request());
                    if transient && attempt < max_retries {
                        last_error = Some(HttpClientError::Transport(err));
                        continue;
                    }
                    return Err(HttpClientError::Transport(err));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| HttpClientError::Http {
            status: 0,
            message: "retries exhausted".to_owned(),
        }))
    }

    /// Send one request for an observer: buffer the whole response, report
    /// it, and hand the decoders an equivalent response built from the
    /// buffer.
    ///
    /// A body that breaks off is reported as a failure and then handled the
    /// way the unobserved path would have handled it:
    ///
    /// - on a non-2xx status the caller only wants the status (the error
    ///   classifier reads the body with `unwrap_or_default`), so the status
    ///   is handed on with an empty body — retries on 5xx work as before;
    /// - on a 2xx whose body the caller never reads (`POST /agent`), the
    ///   success is handed on;
    /// - otherwise the error is returned marked `in_body`, so it is not
    ///   retried and reaches the caller as the same transport error the
    ///   decoder would have produced.
    #[allow(clippy::too_many_arguments, reason = "one call site; mirrors the request")]
    async fn send_observed(
        &self,
        observer: &dyn HttpObserver,
        id: u64,
        builder: reqwest::RequestBuilder,
        method: &reqwest::Method,
        url: &str,
        body: Option<&[u8]>,
        attempt: usize,
        body_unused_on_success: bool,
    ) -> Result<reqwest::Response, SendFailure> {
        let started_at = std::time::SystemTime::now();
        let began = std::time::Instant::now();
        let mut request_headers = vec![(TOKEN_HEADER.to_owned(), MASKED.to_owned())];
        if body.is_some() {
            request_headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
        }
        let report = |outcome: HttpOutcome| {
            observer.exchange(HttpExchange {
                id,
                method: method.as_str().to_owned(),
                url: url.to_owned(),
                request_headers: request_headers.clone(),
                request_body: body.map(<[u8]>::to_vec),
                attempt: u32::try_from(attempt).unwrap_or(u32::MAX).saturating_add(1),
                started_at,
                elapsed: began.elapsed(),
                outcome,
            });
        };
        let response = match builder.send().await {
            Ok(response) => response,
            Err(err) => {
                report(HttpOutcome::Failed {
                    kind: failure_kind(&err),
                    error: error_chain(&err),
                    partial: None,
                });
                return Err(SendFailure { error: err, in_body: false });
            }
        };
        let status = response.status();
        let version = response.version();
        let header_map = response.headers().clone();
        let headers: Vec<(String, String)> = header_map
            .iter()
            .map(|(name, value)| {
                (name.as_str().to_owned(), String::from_utf8_lossy(value.as_bytes()).into_owned())
            })
            .collect();
        match response.bytes().await {
            Ok(bytes) => {
                report(HttpOutcome::Response {
                    status: status.as_u16(),
                    headers,
                    body: bytes.to_vec(),
                });
                Ok(rebuild_response(status, version, header_map, bytes))
            }
            Err(err) => {
                report(HttpOutcome::Failed {
                    kind: if err.is_timeout() { "timeout" } else { "body" },
                    error: error_chain(&err),
                    partial: Some((status.as_u16(), headers)),
                });
                if !status.is_success() || body_unused_on_success {
                    Ok(rebuild_response(status, version, header_map, bytes::Bytes::new()))
                } else {
                    Err(SendFailure { error: err, in_body: true })
                }
            }
        }
    }

    /// Decide whether a `4xx/5xx` status code is transient (worth retrying).
    fn is_transient_status(status: reqwest::StatusCode) -> bool {
        let code = status.as_u16();
        matches!(code, 408 | 425 | 429) || (500..600).contains(&code)
    }
}

/// Why a request produced no usable response.
struct SendFailure {
    error: reqwest::Error,
    /// The status line and headers arrived; the body did not.
    in_body: bool,
}

/// A response equivalent to the one that was buffered for the observer.
fn rebuild_response(
    status: reqwest::StatusCode,
    version: reqwest::Version,
    headers: reqwest::header::HeaderMap,
    body: bytes::Bytes,
) -> reqwest::Response {
    let mut rebuilt = http::Response::new(reqwest::Body::from(body));
    *rebuilt.status_mut() = status;
    *rebuilt.version_mut() = version;
    *rebuilt.headers_mut() = headers;
    reqwest::Response::from(rebuilt)
}

/// Name the failure for the log: the distinction an operator acts on.
fn failure_kind(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else if err.is_body() || err.is_decode() {
        "body"
    } else {
        "request"
    }
}

/// An error with its causes (`reqwest` keeps "connection refused" and the
/// like in the source chain, not in its own message).
fn error_chain(err: &reqwest::Error) -> String {
    let mut text = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Leading ASCII whitespace of `bytes`, without allocation.
fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let mut out = bytes;
    while let Some((&first, rest)) = out.split_first() {
        if first.is_ascii_whitespace() {
            out = rest;
        } else {
            break;
        }
    }
    out
}

/// A short, single-line excerpt of a response body, safe to embed in an
/// error message (and therefore in state snapshots / evidence records).
fn snippet(bytes: &[u8]) -> String {
    let head: String = bytes
        .iter()
        .take(64)
        .map(|&b| if b.is_ascii_graphic() || b == b' ' { char::from(b) } else { '?' })
        .collect();
    format!("{head}…")
}

/// Translate an error response into the typed [`HttpClientError`] variant.
async fn classify_error(
    status: reqwest::StatusCode,
    response: reqwest::Response,
) -> HttpClientError {
    let code = status.as_u16();
    let body = response.text().await.unwrap_or_default();
    match code {
        401 => match serde_json::from_str::<AuthError>(&body) {
            Ok(auth) => HttpClientError::from(auth),
            Err(_) => HttpClientError::Http { status: code, message: body },
        },
        403 => match serde_json::from_str::<AccessTimeError>(&body) {
            Ok(access) => HttpClientError::from(access),
            Err(_) => HttpClientError::Http { status: code, message: body },
        },
        other => HttpClientError::Http { status: other, message: body },
    }
}

/// Translate a `POST /agent` response into either `Ok(SubmitAck)` or an
/// [`HttpClientError`]. The body of the success path is empty per spec.
async fn handle_submit_response(response: reqwest::Response) -> Result<SubmitAck, HttpClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(SubmitAck::ok());
    }
    Err(classify_error(status, response).await)
}

/// Builder for [`MatchClient`].
#[derive(Debug, Clone)]
pub struct MatchClientBuilder {
    base_url: String,
    token: String,
    retry_policy: RetryPolicy,
    request_timeout: Duration,
    observer: Option<std::sync::Arc<dyn HttpObserver>>,
}

impl MatchClientBuilder {
    /// Report every HTTP exchange to `observer` (Issue #396). Without this
    /// call the client behaves exactly as before.
    #[must_use]
    pub fn observer(mut self, observer: std::sync::Arc<dyn HttpObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Override the retry policy.
    #[must_use]
    pub const fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Override the per-request timeout.
    #[must_use]
    pub const fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Build the [`MatchClient`].
    ///
    /// # Errors
    ///
    /// See [`MatchClient::new`].
    pub fn build(self) -> Result<MatchClient, HttpClientError> {
        let http = reqwest::Client::builder().timeout(self.request_timeout).build()?;
        Ok(MatchClient {
            base_url: self.base_url,
            token: self.token,
            http,
            retry_policy: self.retry_policy,
            request_timeout: self.request_timeout,
            observer: self.observer,
            exchange_ids: std::sync::Arc::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    /// Build a client that targets a mockito server with zero backoff so
    /// tests run quickly.
    fn fast_client(server: &Server) -> MatchClient {
        let policy = RetryPolicy { max_retries: 4, backoff_millis: &[0, 0, 0, 0] };
        MatchClient::builder(server.url(), "test-token")
            .retry_policy(policy)
            .request_timeout(Duration::from_secs(5))
            .build()
            .expect("client builds")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_agent_success_path() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/agent")
            .match_header("Procon-Token", "test-token")
            .match_body(mockito::Matcher::JsonString("[0,1,0]".into()))
            .with_status(200)
            .create_async()
            .await;

        let client = fast_client(&server);
        let ack = client
            .submit_agent(vec![AgentKind::KIND_0, AgentKind::KIND_1, AgentKind::KIND_0])
            .await
            .expect("submit succeeds");
        assert_eq!(ack, SubmitAck::ok());
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn posts_are_never_retried_on_transient_status() {
        for path in ["/agent", "/"] {
            let mut server = Server::new_async().await;
            let mock = server.mock("POST", path).with_status(503).expect(1).create_async().await;
            let client = fast_client(&server);
            let failed = if path == "/agent" {
                client.submit_agent(vec![AgentKind::KIND_0]).await.is_err()
            } else {
                client.submit_actions(vec![vec![-1]]).await.is_err()
            };
            assert!(failed);
            mock.assert_async().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_agent_does_not_retry_auth_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/agent")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(r#"{"code":"AuthError","message":"bad token"}"#)
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client.submit_agent(vec![AgentKind::KIND_0]).await.expect_err("auth must fail");
        assert!(matches!(err, HttpClientError::Auth { .. }));
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_agent_timeout_retries_then_fails() {
        // A non-routable address causes connect timeouts.
        let policy = RetryPolicy { max_retries: 1, backoff_millis: &[0] };
        let client = MatchClient::builder("http://10.255.255.1:1", "t")
            .retry_policy(policy)
            .request_timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let err = client
            .submit_agent(vec![AgentKind::KIND_0])
            .await
            .expect_err("timeout must surface as error");
        assert!(matches!(err, HttpClientError::Transport(_)), "got {err:?}");
    }

    /// Minimal `MatchSetting` fixture used by the `/setting` tests.
    fn sample_setting() -> MatchSetting {
        MatchSetting {
            starts_at: 1_700_000_000,
            day_seconds: vec![60],
            day_steps: vec![10],
            map: crate::match_config::Map {
                height: 8,
                width: 8,
                cells: vec![vec![crate::match_config::CellType::TYPE_0; 8]; 8],
            },
            spots: vec![crate::match_config::Spot { brand: 0, pos: 0, stocks: 3 }],
            agents: vec![0],
            fuel_limits: 30,
            players: 2,
            busy_threshold: 3,
            jammed_threshold: 5,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_match_setting_success_path() {
        let mut server = Server::new_async().await;
        let body = serde_json::to_string(&sample_setting()).expect("serialise");
        let mock = server
            .mock("GET", "/setting")
            .match_header("Procon-Token", "test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = fast_client(&server);
        let setting = client.get_match_setting().await.expect("setting succeeds");
        assert_eq!(setting, sample_setting());
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_match_setting_does_not_retry_on_401() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/setting")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(r#"{"code":"AuthError","message":"bad token"}"#)
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client.get_match_setting().await.expect_err("auth must fail");
        assert!(matches!(err, HttpClientError::Auth { .. }), "got {err:?}");
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_match_setting_does_not_retry_on_403() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/setting")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(r#"{"code":"AccessTimeError","message":"試合がまだ開始されていません"}"#)
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client.get_match_setting().await.expect_err("window error must fail");
        assert!(matches!(err, HttpClientError::AccessTime { .. }), "got {err:?}");
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_match_setting_retries_transient_then_succeeds() {
        let mut server = Server::new_async().await;
        let m503 = server.mock("GET", "/setting").with_status(503).create_async().await;
        let m502 = server.mock("GET", "/setting").with_status(502).create_async().await;
        let body = serde_json::to_string(&sample_setting()).expect("serialise");
        let m200 = server
            .mock("GET", "/setting")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create_async()
            .await;

        let client = fast_client(&server);
        let setting = client.get_match_setting().await.expect("transient retries then succeed");
        assert_eq!(setting, sample_setting());
        m503.assert_async().await;
        m502.assert_async().await;
        m200.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_actions_returns_action_accepted_on_positive_revision() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"revision":42}"#)
            .create_async()
            .await;

        let client = fast_client(&server);
        let accepted =
            client.submit_actions(vec![vec![1, 2, 3], vec![0, 0, 0]]).await.expect("plan accepted");
        assert_eq!(accepted, ActionAccepted { revision: 42 });
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_actions_does_not_retry_on_negative_revision() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"revision":-1}"#)
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client
            .submit_actions(vec![vec![1, 2, 3]])
            .await
            .expect_err("negative revision must be rejected");
        match err {
            HttpClientError::Http { status, message } => {
                assert_eq!(status, 200);
                assert_eq!(message, "rejected: revision=-1");
            }
            other => panic!("expected HttpClientError::Http, got {other:?}"),
        }
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_actions_sends_procon_token_header() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header("Procon-Token", "test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"revision":7}"#)
            .create_async()
            .await;

        let client = fast_client(&server);
        let accepted = client.submit_actions(vec![vec![0, 0, 0]]).await.expect("plan accepted");
        assert_eq!(accepted, ActionAccepted { revision: 7 });
        mock.assert_async().await;
    }

    // ------------------------------------------------------------------
    // Issue #179: `get_json` (arena registration) + fail-closed drift
    // detection on every JSON-where-it-must-be endpoint.
    // ------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_json_decodes_with_the_token_header() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/settings")
            .match_header("Procon-Token", "test-token")
            .with_status(200)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(r#"{"matchId":"m-1"}"#)
            .create_async()
            .await;

        let client = fast_client(&server);
        let value: serde_json::Value = client.get_json("/settings").await.expect("json");
        assert_eq!(value["matchId"], "m-1");
        mock.assert_async().await;
    }

    /// The unofficial arena serves its SPA HTML shell with HTTP 200 for
    /// unauthenticated / unknown paths. That must surface as `ApiDrift`
    /// after exactly **one** request — never a retry storm, never a
    /// guessed parse of the body.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_json_fails_closed_on_an_html_shell() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/settings")
            .with_status(200)
            .with_header("content-type", "text/html; charset=utf-8")
            .with_body("<!DOCTYPE html><html><title>ヘキサうどん 非公式練習場</title></html>")
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client
            .get_json::<serde_json::Value>("/settings")
            .await
            .expect_err("HTML must not decode as JSON");
        match err {
            HttpClientError::ApiDrift { path, detail } => {
                assert_eq!(path, "/settings");
                assert!(detail.contains("text/html"), "{detail}");
            }
            other => panic!("expected ApiDrift, got {other:?}"),
        }
        mock.assert_async().await;
    }

    /// A JSON-less body without any content type (the other half of the
    /// SPA fallback) is drift too — detected from the body itself.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_json_fails_closed_when_the_body_is_html_without_content_type() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/settings")
            .with_status(200)
            .with_body("<!DOCTYPE html><html></html>")
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client
            .get_json::<serde_json::Value>("/settings")
            .await
            .expect_err("HTML must not decode");
        assert!(matches!(err, HttpClientError::ApiDrift { .. }), "got {err:?}");
        mock.assert_async().await;
    }

    /// The same fail-closed rule applies to the official endpoints, so an
    /// arena (or server) that starts serving HTML where `MatchSetting` was
    /// promised stops the coordinator with a drift error instead of a bare
    /// serde decode failure.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_match_setting_fails_closed_on_an_html_shell() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/setting")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<!DOCTYPE html><html></html>")
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client.get_match_setting().await.expect_err("drift must fail closed");
        assert!(matches!(err, HttpClientError::ApiDrift { .. }), "got {err:?}");
        mock.assert_async().await;
    }

    /// Status codes alone are never trusted: a 403 carrying the HTML shell
    /// is classified from the body, not silently retried as `AccessTime`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_json_403_html_is_not_mistaken_for_an_access_time_window() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/settings")
            .with_status(403)
            .with_header("content-type", "text/html")
            .with_body("<!DOCTYPE html><html></html>")
            .expect(1)
            .create_async()
            .await;

        let client = fast_client(&server);
        let err = client
            .get_json::<serde_json::Value>("/settings")
            .await
            .expect_err("403 HTML is not an AccessTime envelope");
        assert!(
            !matches!(err, HttpClientError::AccessTime { .. }),
            "a 403 HTML body must not enter the retry path: {err:?}"
        );
        mock.assert_async().await;
    }
}
