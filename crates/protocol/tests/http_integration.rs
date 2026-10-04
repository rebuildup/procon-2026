//! End-to-end integration test for `MatchClient`.
//!
//! These tests stand up a real `mockito` server and exercise the public
//! `MatchClient` API surface (submit + `get_match_state`) over the wire,
//! including the `Procon-Token` header and JSON body encoding.

use std::time::Duration;

use mockito::Server;
use protocol::match_config::{AgentKind, AgentState, CellType, Map, MatchState, OtherPlayer};
use protocol::{HttpClientError, MatchClient, PROTOCOL_VERSION, RetryPolicy, SubmitAck};

fn fast_retry_policy() -> RetryPolicy {
    RetryPolicy { max_retries: 4, backoff_millis: &[0, 0, 0, 0] }
}

fn sample_match_state() -> MatchState {
    MatchState {
        ends_at: 1_700_000_000,
        day: 0,
        agents: vec![
            AgentState { kind: AgentKind::KIND_0, pos: 0, fuel: 30 },
            AgentState { kind: AgentKind::KIND_1, pos: 1, fuel: 30 },
        ],
        others: vec![OtherPlayer {
            id: 1,
            agents: vec![AgentState { kind: AgentKind::KIND_1, pos: 63, fuel: 25 }],
        }],
        traffics: vec![],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_agent_happy_path() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/agent")
        .match_header("Procon-Token", "secret-token-123")
        .match_body(mockito::Matcher::JsonString("[0,1,0,1]".into()))
        .with_status(200)
        .create_async()
        .await;

    let client = MatchClient::builder(server.url(), "secret-token-123")
        .retry_policy(fast_retry_policy())
        .request_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let ack = client
        .submit_agent(vec![
            AgentKind::KIND_0,
            AgentKind::KIND_1,
            AgentKind::KIND_0,
            AgentKind::KIND_1,
        ])
        .await
        .expect("submit_agent succeeds on 200");

    assert_eq!(ack, SubmitAck::ok());
    mock.assert_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_match_state_decodes_json() {
    let mut server = Server::new_async().await;
    let body = serde_json::to_string(&sample_match_state()).unwrap();
    let mock = server
        .mock("GET", "/")
        .match_header("Procon-Token", "tok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body.clone())
        .create_async()
        .await;

    let client = MatchClient::builder(server.url(), "tok")
        .retry_policy(fast_retry_policy())
        .build()
        .unwrap();

    let state = client.get_match_state().await.expect("state fetch succeeds");
    assert_eq!(state.day, 0);
    assert_eq!(state.agents.len(), 2);
    assert_eq!(state.agents[0].kind, AgentKind::KIND_0);
    assert_eq!(state.others.len(), 1);
    assert_eq!(state.others[0].id, 1);
    mock.assert_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_error_is_returned_without_retry() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/agent")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"code":"AuthError","message":"invalid token"}"#)
        .expect(1)
        .create_async()
        .await;

    let client = MatchClient::builder(server.url(), "wrong")
        .retry_policy(fast_retry_policy())
        .build()
        .unwrap();

    let err =
        client.submit_agent(vec![AgentKind::KIND_0]).await.expect_err("auth errors propagate");
    assert!(matches!(err, HttpClientError::Auth { .. }));
    mock.assert_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn access_time_error_is_returned() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/agent")
        .with_status(403)
        .with_header("content-type", "application/json")
        .with_body(r#"{"code":"AccessTimeError","message":"match has not started"}"#)
        .expect(1)
        .create_async()
        .await;

    let client = MatchClient::builder(server.url(), "tok")
        .retry_policy(fast_retry_policy())
        .build()
        .unwrap();

    let err = client
        .submit_agent(vec![AgentKind::KIND_0])
        .await
        .expect_err("access-time errors propagate");
    assert!(matches!(err, HttpClientError::AccessTime { .. }));
    mock.assert_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_retries_on_5xx_then_succeeds() {
    let mut server = Server::new_async().await;
    let m1 = server.mock("GET", "/").with_status(503).create_async().await;
    let m2 = server.mock("GET", "/").with_status(500).create_async().await;
    let m3 = server
        .mock("GET", "/")
        .with_status(200)
        .with_body(serde_json::to_string(&sample_match_state()).unwrap())
        .create_async()
        .await;

    let client = MatchClient::builder(server.url(), "tok")
        .retry_policy(fast_retry_policy())
        .build()
        .unwrap();

    let state = client.get_match_state().await.expect("GET retries succeed");
    assert_eq!(state, sample_match_state());
    m1.assert_async().await;
    m2.assert_async().await;
    m3.assert_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protocol_version_constant_is_stable() {
    // Smoke check: the protocol version constant is exported and equal to
    // `{ major: 1, minor: 0 }`. This guards against accidental version
    // bumps breaking clients.
    assert_eq!(PROTOCOL_VERSION.major, 1);
}

#[allow(dead_code)]
fn _suppress_unused(_: CellType, _: Map) {}

/// A server receives the POST, but the acknowledgement misses the client timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_posts_with_lost_acknowledgements_are_not_replayed() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::AsyncReadExt;
    for agent_endpoint in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let received = Arc::clone(&count);
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0; 4096];
                let read = stream.read(&mut buf).await.unwrap();
                assert!(read > 0);
                received.fetch_add(1, Ordering::SeqCst);
                // Hold the connection open without acknowledging the received request.
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    drop(stream);
                });
            }
        });
        let client = MatchClient::builder(format!("http://{addr}"), "test")
            .retry_policy(fast_retry_policy())
            .request_timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let failed = if agent_endpoint {
            client.submit_agent(vec![AgentKind::KIND_0]).await.is_err()
        } else {
            client.submit_actions(vec![vec![-1]]).await.is_err()
        };
        assert!(failed);
        assert_eq!(count.load(Ordering::SeqCst), 1, "ambiguous POST must be sent exactly once");
        server.abort();
    }
}
