//! End-to-end test: official 4-endpoint match protocol against mockito.
//!
//! Verifies that `run_coordinator_loop` from `crates/cli` drives a full match
//! of `match-config.example.json`'s `daySeconds.len()` days using a
//! mockito-backed `MatchClient`, with each `POST /` body satisfying the
//! **official terrain-weighted step rule** (Issue #226: Σ enter-costs + waits
//! `== day_steps[day]`; the mock rejects element-count-only plans with
//! `revision: -1`, exactly like the real server) and final `revision >= 0`.
//!
//! To inject the mockito URL into the binary instead, see the
//! `--server http://127.0.0.1:NNNN` flag on `procon coordinator`.
//! Running the same scenario through the binary exercises the
//! `MatchClient::builder(base_url, token)` initialisation path and the
//! CLI's `--server` override (`PROCON_SERVER`); the test below targets the
//! library directly because we want to assert on the inner wire payloads.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::time::Duration;

use mockito::Server;
use serde_json::json;

use cli::coordinator_loop::{CoordinatorLoopConfig, CoordinatorOutcome, run_coordinator_loop};
use cli::openapi_fixture::OpenApiFixture;
use cli::solver_adapter::EngineLoopAdapter;
use protocol::http_client::{MatchClient, RetryPolicy};
use protocol::match_config::{CellType, Map, MatchSetting};

const OPENAPI_PATH: &str = "../../.reference/procon37/openapi.json";
const MATCH_CONFIG_PATH: &str = "../../.reference/procon37/match-config.example.json";

/// Synthesise a minimal but wire-valid [`MatchSetting`] for an N-day match.
///
/// The on-disk `match-config.example.json` is the *problem* schema
/// (`{problem: {...}, teams: [...]}`), not the wire shape, so
/// [`OpenApiFixture::load`] cannot always parse it into [`MatchSetting`].
/// We build a valid wire shape directly for the mock server.
///
/// The map is the smallest allowed size (`8x8`) per
/// `engine::model::MapSize::MIN`. Spots are empty; one controlled agent at
/// cell index `0` so `WaitSolver` produces one inner `Vec<i32>` per day.
fn synthesized_setting(day_seconds: &[i32], day_steps: &[i32]) -> MatchSetting {
    let width: i32 = 8;
    let height: i32 = 8;
    let width_u = usize::try_from(width).expect("width fits usize");
    let height_u = usize::try_from(height).expect("height fits usize");
    MatchSetting {
        // Non-zero `starts_at` so the setting-poll loop exits on the first
        // `GET /setting` instead of burning a poll interval.
        starts_at: 1,
        day_seconds: day_seconds.to_vec(),
        day_steps: day_steps.to_vec(),
        map: Map { height, width, cells: vec![vec![CellType::TYPE_0; width_u]; height_u] },
        spots: vec![],
        agents: vec![0],
        fuel_limits: 30,
        players: 2,
        busy_threshold: 3,
        jammed_threshold: 5,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn e2e_openapi_fixture_completes_four_days() {
    // ----- 1. Drift guard: read fixtures and check schema coverage -----
    let fixture = OpenApiFixture::load(
        std::path::Path::new(OPENAPI_PATH),
        std::path::Path::new(MATCH_CONFIG_PATH),
    )
    .expect("load OpenAPI fixture");
    fixture.assert_minimal_coverage().expect("coverage");

    // Determine day count from whichever shape the fixture has. The on-disk
    // example JSON declares `daySeconds` under `/problem` (problem schema);
    // the wire schema would expose it at the root level.
    let day_count: usize = fixture
        .match_setting
        .as_ref()
        .map(|s| s.day_seconds.len())
        .or_else(|| {
            fixture
                .match_config_raw
                .pointer("/problem/daySeconds")
                .and_then(|v| v.as_array())
                .map(Vec::len)
        })
        .or_else(|| {
            fixture.match_config_raw.get("daySeconds").and_then(|v| v.as_array()).map(Vec::len)
        })
        .expect("no day count in fixture");
    assert!(day_count >= 1, "fixture must declare at least one day");
    assert_eq!(day_count, 4, "the example fixture declares 4 days");

    // Per-day step budget, read from whichever shape is available. Falls
    // back to `20` per day when the fixture is the problem schema (the
    // example declares `daySteps: [20, 20, 20, 20]`).
    let day_steps: Vec<i32> = fixture
        .match_setting
        .as_ref()
        .map(|s| s.day_steps.clone())
        .or_else(|| {
            fixture
                .match_config_raw
                .pointer("/problem/daySteps")
                .and_then(|v| v.as_array())
                .and_then(|a| serde_json::from_value::<Vec<i32>>(json!(a)).ok())
        })
        .unwrap_or_else(|| vec![20_i32; day_count]);
    let day_seconds: Vec<i32> = vec![30_i32; day_count];

    // ----- 2. Build mockito mock with day counter -----
    let mut server = Server::new_async().await;

    let synthesized = synthesized_setting(&day_seconds, &day_steps);

    // The mock validates `POST /` with the engine's own official-rule
    // implementation (`engine::wire_plan`, Issue #226): an element-count-only
    // plan is rejected here exactly like the real server rejects it, so this
    // E2E proves submission-boundary conformance, not just wire shape.
    let validation_setting = cli::protocol_to_engine::protocol_setting_to_engine(
        &synthesized,
        domain::PlayerId::new("mock-validator").expect("valid team id"),
    )
    .expect("synthesized setting converts");

    // GET /setting -> static MatchSetting built from fixture data.
    let setting_body = serde_json::to_value(&synthesized).expect("serialise setting");
    let m_setting = server
        .mock("GET", "/setting")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(setting_body.to_string())
        .expect_at_least(1)
        .create_async()
        .await;

    // POST /agent -> 200 with empty body. The official `OpenAPI` schema
    // declares a 200 response with no body schema; the client only checks
    // the HTTP status code on this endpoint.
    let m_agent = server
        .mock("POST", "/agent")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("")
        .expect_at_least(1)
        .create_async()
        .await;

    // GET / -> MatchState with day incrementing per call. `endsAt` is set
    // to a value in the past so the loop's between-day sleep
    // (`state.ends_at - now`) is skipped (the loop only sleeps when
    // `diff > 0`).
    let day_counter = Arc::new(AtomicUsize::new(0));
    let dc = day_counter.clone();
    let m_state = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |_req| {
            let d = dc.fetch_add(1, Ordering::SeqCst);
            json!({
                "endsAt": 0_i64,
                "day": i64::try_from(d).unwrap_or(0),
                "agents": [],
                "others": [],
                "traffics": [],
            })
            .to_string()
            .into_bytes()
        })
        .expect_at_least(day_count)
        .create_async()
        .await;

    // POST / -> {revision: <monotonic counter>}; capture each request
    // body in `captured_bodies` for later assertion that inner step sums
    // equal `day_steps[day]`.
    let captured_bodies: Arc<Mutex<Vec<Vec<Vec<i32>>>>> =
        Arc::new(Mutex::new(Vec::with_capacity(day_count)));
    let cb = captured_bodies.clone();
    let revision_counter = Arc::new(AtomicI32::new(1));
    let rc = revision_counter.clone();
    // Official-rule gate (Issue #226): reject with `revision: -1` whatever
    // fails `engine::wire_plan::validate_agent_plan` — terrain-weighted sum
    // exact and every move legal — so the mock cannot be satisfied by an
    // element-count-only plan.
    let submission_day = Arc::new(AtomicI32::new(0));
    let sd = submission_day.clone();
    let vs = validation_setting.clone();
    let day_budgets = Arc::new(day_steps.clone());
    let m_actions = server
        .mock("POST", "/")
        .match_request(move |req| {
            if let Ok(body) = req.body() {
                if let Ok(parsed) = serde_json::from_slice::<Vec<Vec<i32>>>(body) {
                    cb.lock().expect("captured_bodies lock poisoned").push(parsed);
                }
            }
            true
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |req| {
            let day = sd.fetch_add(1, Ordering::SeqCst);
            let budget = day_budgets.get(usize::try_from(day).unwrap_or(0)).copied().unwrap_or(20);
            let accepted = req
                .body()
                .ok()
                .and_then(|raw| serde_json::from_slice::<Vec<Vec<i32>>>(raw).ok())
                .is_some_and(|body| {
                    body.iter().enumerate().all(|(idx, plan)| {
                        vs.agents.get(idx).is_some_and(|agent| {
                            engine::wire_plan::validate_agent_plan(
                                agent,
                                &vs.map,
                                u32::try_from(budget).unwrap_or(0),
                                plan,
                            )
                            .is_ok()
                        })
                    })
                });
            let body = if accepted {
                json!({ "revision": rc.fetch_add(1, Ordering::SeqCst) })
            } else {
                json!({ "revision": -1 })
            };
            body.to_string().into_bytes()
        })
        .expect_at_least(day_count)
        .create_async()
        .await;

    // ----- 3. Run the loop -----
    let client = MatchClient::builder(server.url(), "mvp-test-token")
        .retry_policy(RetryPolicy { max_retries: 0, backoff_millis: &[] })
        .request_timeout(Duration::from_secs(5))
        .build()
        .expect("match client builds");
    // Drive the loop with the production solver
    // (`crates/solver-loop::LoopSolver`) so the E2E proves the wiring
    // landed — see issue #152. `GreedyNearestNeighbor` is the documented
    // default in `solver_loop::LoopSolver`.
    let solver: Box<dyn engine::solver::Solver> =
        Box::new(EngineLoopAdapter::new(solver_loop::InitialStrategy::GreedyNearestNeighbor, 32));
    let cfg = CoordinatorLoopConfig {
        team_id: "mvp-player".to_owned(),
        setting_poll_interval: Duration::from_millis(50),
        solver_name: "loop".to_owned(),
        // Issue #156: pull the kind-window retry defaults from
        // `CoordinatorLoopConfig::default()`. The mockito-based server
        // here returns 200 immediately, so the budget is effectively
        // unused — keeping it at the production default keeps the
        // single source of truth in one place.
        ..Default::default()
    };

    let outcome: CoordinatorOutcome =
        run_coordinator_loop(client, solver, cfg).await.expect("loop should complete");

    assert_eq!(outcome.days_completed, day_count, "all days completed");
    assert_eq!(outcome.revisions.len(), day_count, "one revision per day");
    assert!(!outcome.revisions.is_empty(), "must have submitted at least one plan");
    assert!(
        outcome.revisions.iter().all(|&r| r >= 0),
        "all revisions non-negative: {:?}",
        outcome.revisions
    );

    // ----- 4. POST / body assertion: each inner Vec<i32> satisfies the
    //         official terrain-weighted rule (Issue #226). Replaying the
    //         plan from the agent's start cell, every entered cell's step
    //         cost (平地 2 / 山 3 / 道路 1-2-4) plus waits must equal
    //         `day_steps[day]` exactly — an element sum alone is NOT the
    //         contract (probe evidence: `[2, -19]` element-20 rejected,
    //         `[2, -18]` weighted-20 accepted). `EngineLoopAdapter` must
    //         still emit at least one move, proving the planner is not
    //         just waiting.
    //
    // Drop the guard before any `.await` calls below to satisfy
    // `clippy::await_holding_lock`.
    {
        let bodies = captured_bodies.lock().expect("captured_bodies lock poisoned");
        assert_eq!(bodies.len(), day_count, "captured one POST / body per day");
        let mut saw_move = false;
        for (day, body) in bodies.iter().enumerate() {
            assert!(!body.is_empty(), "day {day}: at least one inner list");
            let budget = u32::try_from(day_steps[day]).expect("day budget fits u32");
            for (agent_index, inner) in body.iter().enumerate() {
                let agent = validation_setting
                    .agents
                    .get(agent_index)
                    .unwrap_or_else(|| panic!("day {day}: agent {agent_index} missing"));
                let weighted = engine::wire_plan::validate_agent_plan(
                    agent,
                    &validation_setting.map,
                    budget,
                    inner,
                )
                .unwrap_or_else(|e| panic!("day {day} agent {agent_index}: {e}"));
                assert_eq!(
                    weighted, budget,
                    "day {day} agent {agent_index}: weighted step sum must be exactly day_steps"
                );
                if inner.iter().any(|&x| x >= 0) {
                    saw_move = true;
                }
            }
        }
        assert!(
            saw_move,
            "EngineLoopAdapter must produce at least one Move action \
             across the match; otherwise the wiring of crates/solver-loop \
             is not actually driving the coordinator"
        );
    }

    // ----- 5. Drift guard re-check: OpenAPI paths cover all 4 endpoints
    //         with 200/401/403 response shapes.
    for (path, verb) in &[("/", "get"), ("/", "post"), ("/setting", "get"), ("/agent", "post")] {
        let op = fixture
            .openapi_paths
            .get(*path)
            .and_then(|p| p.get(*verb))
            .unwrap_or_else(|| panic!("openapi missing {path} {verb}"));
        let responses =
            op.get("responses").unwrap_or_else(|| panic!("openapi {path} {verb} has no responses"));
        for code in &["200", "401", "403"] {
            assert!(
                responses.get(*code).is_some(),
                "{} {} missing {}",
                verb.to_uppercase(),
                path,
                code
            );
        }
    }

    // ----- 6. Mockito mock-hit assertions (LIFO assertion order). -----
    m_setting.assert_async().await;
    m_agent.assert_async().await;
    m_state.assert_async().await;
    m_actions.assert_async().await;
}
