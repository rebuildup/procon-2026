//! Coordinator loop runner.
//!
//! Implements the per-match orchestration body for the CLI coordinator:
//!
//! 1. Receive [`MatchSetting`] from the upstream (today: GET /).
//! 2. Ask a [`Solver`] for the day's [`DayPlan`].
//! 3. POST the plan via a [`Transport`] (today: POST /) with retry + timeout.
//! 4. Wait for the next day boundary.
//! 5. Stop on end signal, terminal error, or after the configured day count.
//!
//! The transport is intentionally a trait so the loop can be exercised against
//! an in-process mock without spinning up a real HTTP server. The mock lives
//! in the same file (`tests` module) and uses a `HashMap<url, Mutex<Response>>`
//! keyed by endpoint path.

use std::future::Future;
use std::time::Duration;

use domain::{MatchId, PlayerId, Score};
use tracing::{debug, warn};

/// Settings describing a match handed back from GET /match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSetting {
    /// Server-issued match identifier.
    pub match_id: MatchId,
    /// Number of days the match will run for.
    pub days: u32,
    /// Agents registered to our team for this match.
    pub players: Vec<PlayerId>,
    /// Wall-clock budget per day, in seconds. Informational for the solver.
    pub day_seconds: u64,
}

impl MatchSetting {
    /// Convenience constructor for tests and callers that already have
    /// validated ids.
    #[must_use]
    pub fn new(match_id: MatchId, days: u32, players: Vec<PlayerId>, day_seconds: u64) -> Self {
        Self { match_id, days, players, day_seconds }
    }
}

/// A single agent's action at one step within a day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayAction {
    /// Author of the action. Must be a player in [`MatchSetting::players`].
    pub agent_id: PlayerId,
    /// Server-side action index (1-based per `domain::Turn::validate`).
    pub action_index: u16,
    /// Opaque payload forwarded to the engine verbatim.
    pub payload: serde_json::Value,
}

/// Plan for a single day. Submitted to POST /agent as one batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayPlan {
    /// Day number this plan applies to (1-based).
    pub day: u32,
    /// Ordered list of agent actions for the day.
    pub actions: Vec<DayAction>,
}

impl DayPlan {
    #[must_use]
    pub fn new(day: u32, actions: Vec<DayAction>) -> Self {
        Self { day, actions }
    }
}

/// Submission envelope sent to POST /agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    pub match_id: MatchId,
    pub day: u32,
    pub plan: DayPlan,
}

/// Acknowledgement returned by POST /agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionAck {
    pub accepted: bool,
    /// Human-readable reason when `accepted == false`.
    pub reason: Option<String>,
}

/// Signal returned by `Transport::wait_for_next_day` after a successful submit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaySignal {
    /// The server wants us to continue with `next_day`.
    Continue { next_day: u32 },
    /// The match is over; this is the final score.
    #[allow(dead_code)] // only constructed by the loop runner and tests today
    Finished { final_score: Score },
}

/// Error type for the loop runner. Variants are deliberately narrow so the
/// caller can branch on retryable vs terminal failures.
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    /// The solver returned an error while computing a day's plan. Already
    /// retried internally per [`LoopConfig::solver_max_retries`].
    #[error("solver failed on day {day}: {message}")]
    Solver { day: u32, message: String },
    /// The transport rejected or refused the submission after all retries.
    #[error("transport error on day {day} after {attempts} attempts: {message}")]
    Transport { day: u32, attempts: u32, message: String },
    /// The match setting is structurally invalid (zero days, no players, etc).
    #[error("invalid match setting: {0}")]
    InvalidSetting(String),
}

/// Tunables for the loop runner. Defaults are contest-friendly.
#[derive(Debug, Clone)]
pub struct LoopConfig {
    /// Maximum number of POST attempts per submission (initial + retries).
    pub max_post_attempts: u32,
    /// Maximum number of times we re-invoke the solver per day before giving up.
    pub solver_max_retries: u32,
    /// Initial backoff between transport retries. Doubles up to `max_backoff`.
    pub initial_backoff: Duration,
    /// Upper bound on the exponential backoff.
    pub max_backoff: Duration,
    /// Per-attempt timeout for the transport POST.
    pub post_timeout: Duration,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_post_attempts: 3,
            solver_max_retries: 2,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            post_timeout: Duration::from_secs(10),
        }
    }
}

/// Solver trait local to the loop runner. Kept narrow on purpose so it does
/// not collide with the engine-level trait that lands in #57. Once that trait
/// stabilises we can re-export an adapter from `engine` here.
pub trait Solver {
    /// Compute the plan for `day`. Must be deterministic given the setting.
    fn decide(&self, setting: &MatchSetting, day: u32) -> Result<DayPlan, LoopError>;
}

/// Transport trait abstracting GET /match, POST /agent, and the day boundary.
///
/// All methods are infallible on the happy path; failures surface as
/// [`LoopError::Transport`]. The loop runner applies its own retry policy.
pub trait Transport {
    /// POST a day's submission. Implementations should respect their own
    /// per-call timeout; the loop also wraps the call in `post_timeout`.
    fn post_submission(
        &self,
        submission: &Submission,
    ) -> impl Future<Output = Result<SubmissionAck, LoopError>> + Send;

    /// Wait for the next day boundary after a successful submit.
    fn wait_for_next_day(
        &self,
        current_day: u32,
    ) -> impl Future<Output = Result<DaySignal, LoopError>> + Send;
}

/// Convenience closure-based transport. Useful for tests and one-shot scripts.
#[cfg(test)]
pub struct ClosureTransport<FPost, FWait>
where
    FPost: Fn(&Submission) -> Result<SubmissionAck, LoopError> + Send + Sync,
    FWait: Fn(u32) -> Result<DaySignal, LoopError> + Send + Sync,
{
    post: FPost,
    wait: FWait,
}

#[cfg(test)]
impl<FPost, FWait> ClosureTransport<FPost, FWait>
where
    FPost: Fn(&Submission) -> Result<SubmissionAck, LoopError> + Send + Sync,
    FWait: Fn(u32) -> Result<DaySignal, LoopError> + Send + Sync,
{
    pub fn new(post: FPost, wait: FWait) -> Self {
        Self { post, wait }
    }
}

#[cfg(test)]
impl<FPost, FWait> Transport for ClosureTransport<FPost, FWait>
where
    FPost: Fn(&Submission) -> Result<SubmissionAck, LoopError> + Send + Sync,
    FWait: Fn(u32) -> Result<DaySignal, LoopError> + Send + Sync,
{
    async fn post_submission(&self, submission: &Submission) -> Result<SubmissionAck, LoopError> {
        tokio::task::yield_now().await;
        (self.post)(submission)
    }

    async fn wait_for_next_day(&self, current_day: u32) -> Result<DaySignal, LoopError> {
        tokio::task::yield_now().await;
        (self.wait)(current_day)
    }
}

/// Drive the match loop end-to-end.
///
/// Behaviour:
/// - validates the setting up front,
/// - for each day `1..=setting.days`, asks the solver for a plan and POSTs it,
/// - applies retry + exponential backoff on transport failures,
/// - waits for the server's day boundary signal between submissions,
/// - stops and returns the final score when the server signals `Finished` or
///   when the loop has covered all days.
pub async fn run_match_loop<S, T>(
    setting: MatchSetting,
    solver: &S,
    transport: &T,
    config: &LoopConfig,
) -> Result<Score, LoopError>
where
    S: Solver,
    T: Transport,
{
    validate_setting(&setting)?;

    for day in 1..=setting.days {
        let plan = decide_with_retry(solver, &setting, day, config.solver_max_retries)?;
        debug!(match_id = %setting.match_id, day, actions = plan.actions.len(), "decided plan");

        let submission = Submission { match_id: setting.match_id.clone(), day, plan };
        post_with_retry(transport, &submission, config).await?;
        debug!(match_id = %setting.match_id, day, "submitted plan");

        if day < setting.days {
            let signal = transport.wait_for_next_day(day).await?;
            match signal {
                DaySignal::Continue { .. } => {}
                DaySignal::Finished { final_score } => return Ok(final_score),
            }
        }
    }

    Ok(Score(0))
}

fn validate_setting(setting: &MatchSetting) -> Result<(), LoopError> {
    if setting.days == 0 {
        return Err(LoopError::InvalidSetting("days must be > 0".to_owned()));
    }
    if setting.players.is_empty() {
        return Err(LoopError::InvalidSetting("players must be non-empty".to_owned()));
    }
    Ok(())
}

fn decide_with_retry<S: Solver>(
    solver: &S,
    setting: &MatchSetting,
    day: u32,
    max_retries: u32,
) -> Result<DayPlan, LoopError> {
    let mut attempt = 0_u32;
    loop {
        attempt += 1;
        match solver.decide(setting, day) {
            Ok(plan) => {
                if plan.day != day {
                    return Err(LoopError::Solver {
                        day,
                        message: format!(
                            "solver returned plan for day {} when asked for {day}",
                            plan.day
                        ),
                    });
                }
                return Ok(plan);
            }
            Err(err) => {
                if attempt > max_retries {
                    return Err(err);
                }
                warn!(?err, attempt, "solver failed, retrying");
            }
        }
    }
}

async fn post_with_retry<T: Transport>(
    transport: &T,
    submission: &Submission,
    config: &LoopConfig,
) -> Result<(), LoopError> {
    let mut attempt = 0_u32;
    let mut backoff = config.initial_backoff;
    loop {
        attempt += 1;
        let result =
            tokio::time::timeout(config.post_timeout, transport.post_submission(submission)).await;

        match result {
            Ok(Ok(ack)) if ack.accepted => return Ok(()),
            Ok(Ok(ack)) => {
                warn!(
                    attempt,
                    reason = ?ack.reason,
                    "server rejected submission, retrying"
                );
                if attempt >= config.max_post_attempts {
                    return Err(LoopError::Transport {
                        day: submission.day,
                        attempts: attempt,
                        message: format!(
                            "server rejected: {}",
                            ack.reason.unwrap_or_else(|| "<no reason>".to_owned())
                        ),
                    });
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.max_backoff);
            }
            Ok(Err(err)) => {
                warn!(?err, attempt, "transport error, retrying");
                if attempt >= config.max_post_attempts {
                    return Err(LoopError::Transport {
                        day: submission.day,
                        attempts: attempt,
                        message: err.to_string(),
                    });
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.max_backoff);
            }
            Err(_elapsed) => {
                warn!(attempt, "transport timeout, retrying");
                if attempt >= config.max_post_attempts {
                    return Err(LoopError::Transport {
                        day: submission.day,
                        attempts: attempt,
                        message: "timeout".to_owned(),
                    });
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(config.max_backoff);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Scripted response for the mock server.
    #[derive(Debug, Clone)]
    enum Scripted {
        /// Always accepted.
        Ok,
        /// Accept after `failures` prior attempts have returned the message.
        RetryThenOk { failures: u32, message: &'static str },
        /// Always rejected with the given reason.
        Reject(&'static str),
    }

    /// `HashMap<url, Mutex<Response>>`-shaped mock server. The key is the
    /// conceptual endpoint path; values are scripted responses.
    #[derive(Debug, Default)]
    struct MockServer {
        agents: HashMap<String, Mutex<Scripted>>,
        submissions_seen: AtomicU32,
    }

    impl MockServer {
        fn with_agent(path: &str, scripted: Scripted) -> Arc<Self> {
            let mut agents = HashMap::new();
            agents.insert(path.to_owned(), Mutex::new(scripted));
            Arc::new(Self { agents, submissions_seen: AtomicU32::new(0) })
        }

        fn submissions(&self) -> u32 {
            self.submissions_seen.load(Ordering::SeqCst)
        }
    }

    /// Test transport backed by a mock server.
    struct MockTransport {
        server: Arc<MockServer>,
        /// Forced delay for the post call. 0 = no delay.
        post_delay: Duration,
        /// Days the `wait_for_next_day` signal should finish the match at.
        finish_at: Option<u32>,
    }

    impl MockTransport {
        fn new(server: Arc<MockServer>) -> Self {
            Self { server, post_delay: Duration::ZERO, finish_at: None }
        }

        fn with_finish_at(mut self, day: u32) -> Self {
            self.finish_at = Some(day);
            self
        }
    }

    impl Transport for MockTransport {
        async fn post_submission(
            &self,
            _submission: &Submission,
        ) -> Result<SubmissionAck, LoopError> {
            if !self.post_delay.is_zero() {
                tokio::time::sleep(self.post_delay).await;
            }
            self.server.submissions_seen.fetch_add(1, Ordering::SeqCst);
            let scripted = {
                let guard = self
                    .server
                    .agents
                    .get("/agent")
                    .expect("mock server must register /agent response");
                let guard = guard.lock().expect("mock poisoned");
                match &*guard {
                    Scripted::Ok => Scripted::Ok,
                    Scripted::Reject(reason) => Scripted::Reject(reason),
                    Scripted::RetryThenOk { failures, message } => {
                        let seen = self.server.submissions() - 1;
                        if seen < *failures { Scripted::Reject(message) } else { Scripted::Ok }
                    }
                }
            };
            match scripted {
                Scripted::Ok => Ok(SubmissionAck { accepted: true, reason: None }),
                Scripted::RetryThenOk { .. } => unreachable!("normalised above"),
                Scripted::Reject(reason) => {
                    Ok(SubmissionAck { accepted: false, reason: Some(reason.to_owned()) })
                }
            }
        }

        async fn wait_for_next_day(&self, current_day: u32) -> Result<DaySignal, LoopError> {
            // Force the function to need an async boundary so we don't trip
            // `unused_async_trait_impl`. Tests rely on this being awaitable.
            tokio::task::yield_now().await;
            if Some(current_day) == self.finish_at {
                Ok(DaySignal::Finished { final_score: Score(42) })
            } else {
                Ok(DaySignal::Continue { next_day: current_day + 1 })
            }
        }
    }

    /// Solver that emits a deterministic per-day plan. Counts invocations.
    struct EchoSolver {
        invocations: Arc<AtomicU32>,
    }

    impl EchoSolver {
        fn new() -> Self {
            Self { invocations: Arc::new(AtomicU32::new(0)) }
        }

        fn calls(&self) -> u32 {
            self.invocations.load(Ordering::SeqCst)
        }
    }

    impl Solver for EchoSolver {
        fn decide(&self, setting: &MatchSetting, day: u32) -> Result<DayPlan, LoopError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            let action = DayAction {
                agent_id: setting.players[0].clone(),
                action_index: u16::try_from(day).unwrap_or(u16::MAX),
                payload: serde_json::json!({ "day": day }),
            };
            Ok(DayPlan::new(day, vec![action]))
        }
    }

    /// Solver that always fails. Counts invocations.
    struct FailingSolver {
        invocations: Arc<AtomicU32>,
    }

    impl FailingSolver {
        fn new() -> Self {
            Self { invocations: Arc::new(AtomicU32::new(0)) }
        }
    }

    impl Solver for FailingSolver {
        fn decide(&self, _setting: &MatchSetting, day: u32) -> Result<DayPlan, LoopError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            Err(LoopError::Solver { day, message: "boom".to_owned() })
        }
    }

    fn sample_setting(days: u32) -> MatchSetting {
        let match_id = MatchId::new("match-test").unwrap();
        let players = vec![PlayerId::new("player-a").unwrap()];
        MatchSetting::new(match_id, days, players, 60)
    }

    fn fast_config() -> LoopConfig {
        LoopConfig {
            max_post_attempts: 3,
            solver_max_retries: 1,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
            post_timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn happy_path_three_days() {
        let server = MockServer::with_agent("/agent", Scripted::Ok);
        let transport = MockTransport::new(Arc::clone(&server));
        let solver = EchoSolver::new();

        let score =
            run_match_loop(sample_setting(3), &solver, &transport, &fast_config()).await.unwrap();

        assert_eq!(score, Score(0));
        assert_eq!(server.submissions(), 3);
        assert_eq!(solver.calls(), 3);
    }

    #[tokio::test]
    async fn transport_retries_then_succeeds() {
        let server = MockServer::with_agent(
            "/agent",
            Scripted::RetryThenOk { failures: 2, message: "transient" },
        );
        let transport = MockTransport::new(Arc::clone(&server));
        let solver = EchoSolver::new();

        let mut config = fast_config();
        config.max_post_attempts = 5;

        let score = run_match_loop(sample_setting(1), &solver, &transport, &config).await.unwrap();

        assert_eq!(score, Score(0));
        assert_eq!(server.submissions(), 3, "should have retried twice before success");
    }

    #[tokio::test]
    async fn transport_exhausts_retries() {
        let server = MockServer::with_agent("/agent", Scripted::Reject("nope"));
        let transport = MockTransport::new(Arc::clone(&server));
        let solver = EchoSolver::new();

        let mut config = fast_config();
        config.max_post_attempts = 3;

        let err =
            run_match_loop(sample_setting(1), &solver, &transport, &config).await.unwrap_err();

        assert!(
            matches!(err, LoopError::Transport { day: 1, attempts: 3, .. }),
            "expected transport error after retries, got {err:?}"
        );
        assert_eq!(server.submissions(), 3);
    }

    #[tokio::test]
    async fn solver_retries_then_gives_up() {
        let server = MockServer::with_agent("/agent", Scripted::Ok);
        let transport = MockTransport::new(Arc::clone(&server));
        let solver = FailingSolver::new();

        let score = run_match_loop(sample_setting(1), &solver, &transport, &fast_config()).await;

        assert!(matches!(
            score,
            Err(LoopError::Solver { day: 1, message }) if message == "boom"
        ));
    }

    #[tokio::test]
    async fn server_ends_match_early() {
        let server = MockServer::with_agent("/agent", Scripted::Ok);
        let transport = MockTransport::new(Arc::clone(&server)).with_finish_at(2);
        let solver = EchoSolver::new();

        let score =
            run_match_loop(sample_setting(5), &solver, &transport, &fast_config()).await.unwrap();

        assert_eq!(score, Score(42), "early-finish signal wins over running the full day count");
        assert_eq!(server.submissions(), 2, "should stop submitting after the finish signal");
        assert_eq!(solver.calls(), 2);
    }

    #[tokio::test]
    async fn zero_days_is_rejected() {
        let server = MockServer::with_agent("/agent", Scripted::Ok);
        let transport = MockTransport::new(Arc::clone(&server));
        let solver = EchoSolver::new();

        let err = run_match_loop(sample_setting(0), &solver, &transport, &fast_config())
            .await
            .unwrap_err();

        assert!(matches!(err, LoopError::InvalidSetting(_)));
        assert_eq!(server.submissions(), 0);
        assert_eq!(solver.calls(), 0);
    }

    #[tokio::test]
    async fn empty_players_is_rejected() {
        let server = MockServer::with_agent("/agent", Scripted::Ok);
        let transport = MockTransport::new(Arc::clone(&server));
        let solver = EchoSolver::new();

        let match_id = MatchId::new("match-test").unwrap();
        let setting = MatchSetting::new(match_id, 3, vec![], 60);

        let err = run_match_loop(setting, &solver, &transport, &fast_config()).await.unwrap_err();

        assert!(matches!(err, LoopError::InvalidSetting(_)));
    }

    #[tokio::test]
    async fn closure_transport_works() {
        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);
        let transport = ClosureTransport::new(
            move |_sub: &Submission| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(SubmissionAck { accepted: true, reason: None })
            },
            |day: u32| Ok(DaySignal::Continue { next_day: day + 1 }),
        );
        let solver = EchoSolver::new();
        let score =
            run_match_loop(sample_setting(2), &solver, &transport, &fast_config()).await.unwrap();
        assert_eq!(score, Score(0));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
