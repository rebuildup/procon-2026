//! End-to-end test: official 4-endpoint match protocol against mockito.
//!
//! Verifies that `run_coordinator_loop` from `crates/cli` drives a full match
//! of `match-config.example.json`'s `daySeconds.len()` days using a
//! mockito-backed `MatchClient`, with each `POST /` body satisfying the
//! **official terrain-weighted step rule** (Issue #226: origin-cell step
//! costs plus waits `== day_steps[day]`; the mock rejects
//! element-count-only plans with `revision: -1`, exactly like the real
//! server) and final `revision >= 0`.
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
use cli::solver_adapter::{EngineLoopAdapter, WaitSolver};
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
    //
    // The body is pinned too (Issues #238/#243): one agent, the `loop`
    // solver and **no** `--kinds` flag must POST exactly what the
    // coordinator has always POSTed — `[0]`, i.e. even slot -> patrol. A
    // regression to the default composition makes this mock unmatched and
    // the loop fails instead of passing silently.
    let m_agent = server
        .mock("POST", "/agent")
        .match_body(mockito::Matcher::Json(json!([0])))
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
    //         plan from the agent's start cell, each move's **origin**
    //         cell step cost (平地 2 / 山 3 / 道路 1-2-4) plus waits must
    //         equal `day_steps[day]` exactly — an element sum alone is NOT
    //         the contract (probe evidence: `[2, -19]` rejected,
    //         `[2, -18]` accepted). `EngineLoopAdapter` must still emit at
    //         least one move, proving the planner is not just waiting.
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

// ---------------------------------------------------------------------------
// Issues #238 / #243 — the live `--kinds` composition, on the wire.
// ---------------------------------------------------------------------------

/// Drive a 2-agent, 2-day match against a fresh mock server and return the
/// outcome. The mock answers `POST /agent` **only** when the body equals
/// `expect_agent_body`, so a composition that never reached the server
/// fails the run (mockito 501s unmatched requests) instead of passing
/// silently.
///
/// The solver object is always `WaitSolver`: `solver_name` is what selects
/// the kinds rule (exactly as in the CLI), which keeps these runs about the
/// composition and independent of any planner.
///
/// # Panics
/// When the loop, or the wire assertion, does not hold.
async fn run_two_agent_match(
    agent_kinds: Option<Vec<engine::model::AgentKind>>,
    solver_name: &str,
    expect_agent_body: serde_json::Value,
) -> CoordinatorOutcome {
    let mut server = Server::new_async().await;

    // Two agents so the default alternation (`[0, 1]`), the `wait`
    // placeholder (`[0, 0]`) and an explicit pattern (`[1, 1]`) are all
    // distinguishable in the request body.
    let mut synthesized = synthesized_setting(&[30, 30], &[20, 20]);
    synthesized.agents = vec![0, 1];
    let setting_body = serde_json::to_value(&synthesized).expect("serialise setting");
    let m_setting = server
        .mock("GET", "/setting")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(setting_body.to_string())
        .expect_at_least(1)
        .create_async()
        .await;

    let m_agent = server
        .mock("POST", "/agent")
        .match_body(mockito::Matcher::Json(expect_agent_body))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("")
        .expect_at_least(1)
        .create_async()
        .await;

    // `endsAt` in the past skips the between-day sleep; the day counter
    // makes every `GET /` hand back the next day.
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
        .expect_at_least(2)
        .create_async()
        .await;

    let revision_counter = Arc::new(AtomicI32::new(0));
    let rc = revision_counter.clone();
    let m_actions = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |_req| {
            json!({ "revision": rc.fetch_add(1, Ordering::SeqCst) }).to_string().into_bytes()
        })
        .expect_at_least(2)
        .create_async()
        .await;

    let client = MatchClient::builder(server.url(), "mvp-test-token")
        .retry_policy(RetryPolicy { max_retries: 0, backoff_millis: &[] })
        .request_timeout(Duration::from_secs(5))
        .build()
        .expect("match client builds");

    let cfg = CoordinatorLoopConfig {
        solver_name: solver_name.to_owned(),
        agent_kinds,
        setting_poll_interval: Duration::from_millis(20),
        ..CoordinatorLoopConfig::default()
    };

    let outcome = run_coordinator_loop(client, Box::new(WaitSolver), cfg)
        .await
        .expect("loop should complete");

    m_setting.assert_async().await;
    m_agent.assert_async().await;
    m_state.assert_async().await;
    m_actions.assert_async().await;
    outcome
}

/// The three compositions the live `--kinds` flag has to tell apart,
/// asserted on the actual `POST /agent` body (Issues #238/#243):
///
/// 1. flag omitted + real solver → today's alternation, byte for byte;
/// 2. flag omitted + `wait`      → today's all-`KIND_0` placeholder;
/// 3. `--kinds S` + `wait`       → the explicit pattern wins over the
///    `wait` special case.
///
/// Each run also checks [`CoordinatorOutcome::kinds`] — the field the arena
/// evidence record copies — because the composition has to be *visible*, not
/// just submitted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kinds_composition_reaches_the_wire_and_the_outcome() {
    use engine::model::AgentKind;

    let outcome = run_two_agent_match(None, "loop", json!([0, 1])).await;
    assert_eq!(
        outcome.kinds,
        vec![AgentKind::Patrol, AgentKind::Supply],
        "omitted --kinds must keep the historical alternation"
    );
    assert_eq!(outcome.days_completed, 2, "the match itself must still run");

    let outcome = run_two_agent_match(None, "wait", json!([0, 0])).await;
    assert_eq!(
        outcome.kinds,
        vec![AgentKind::Patrol, AgentKind::Patrol],
        "wait keeps its all-KIND_0 placeholder when --kinds is omitted"
    );

    let outcome = run_two_agent_match(Some(vec![AgentKind::Supply]), "wait", json!([1, 1])).await;
    assert_eq!(
        outcome.kinds,
        vec![AgentKind::Supply, AgentKind::Supply],
        "an explicit --kinds wins for every solver, wait included"
    );
}

// ---------------------------------------------------------------------------
// Issue #245 — low thresholds, 7 agents, multi-day traffic accumulation.
// ---------------------------------------------------------------------------

/// Fixed, step-major plan for the Issue #245 regression. Agent 0 steps off
/// the road it is parked on (the move the official *departed-cell* fuel
/// rule refuses while the tank holds 1), agent 1 walks onto a road cell and
/// steps back (so the day-start state stays self-consistent across days)
/// while the daily traffic flips those cells between smooth / busy / jammed,
/// and everyone else waits.
struct RoadWalkerSolver;

impl engine::solver::Solver for RoadWalkerSolver {
    fn name(&self) -> &'static str {
        "road-walker"
    }

    fn decide(&self, ctx: &engine::solver::SolverContext) -> engine::solver::SolverDecision {
        use engine::solver::{Action, Direction};
        let mut actions = Vec::with_capacity(ctx.expected_action_count());
        for step in 0..ctx.day_steps {
            for (idx, id) in ctx.my_agent_ids.iter().enumerate() {
                actions.push(match (step, idx) {
                    (0, 1) => Action::step(*id, Direction::Right),
                    // agent 0 leaves the road it parks on; agent 1 walks
                    // onto a road cell and steps straight back, so the
                    // day-start state repeats itself across days.
                    (0, 0) | (1, 1) => Action::step(*id, Direction::Left),
                    _ => Action::wait(*id),
                });
            }
        }
        engine::solver::SolverDecision::new(actions)
    }
}

/// Issue #245: `busyThreshold 1`, 7 agents, 4 days, and a road segment whose
/// status walks smooth → busy → jammed must be accepted on **every** day.
///
/// The double reports exactly what the official localhost binary reports —
/// one `traffics` entry per road cell, all four of them, every day — and
/// validates each `POST /` against the same day-start map/agents it just
/// handed out (`engine::wire_plan::validate_agent_plan`, Issue #226). Two
/// things are pinned:
///
/// 1. the traffic overlay keeps the weighted budget exact as a road flips
///    from 1 to 2 to 4 steps, and
/// 2. agent 0 — parked on a 道路 with `fuel 1` — must **not** ship the step
///    off it: leaving a road costs 2 fuel, so the official server rejects
///    the whole day for that one action (match550 day 4, #245).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn low_threshold_traffic_and_departed_cell_fuel_accept_every_day() {
    const DAYS: usize = 4;
    const BUDGET: i32 = 16;
    /// Wire positions of the road segment on row 2, columns 4..=7.
    const ROAD_POS: [i32; 4] = [20, 21, 22, 23];

    // Day-start status of those roads: 0 smooth, 1 busy, 2 jammed. With
    // `busyThreshold 1 / jammedThreshold 3` the official day-end traffic
    // update walks a heavily used cell through smooth → busy → jammed, i.e.
    // step cost 1 → 2 → 4 for a departure from it.
    let traffic_by_day: [[i32; 4]; DAYS] = [[0, 0, 0, 0], [0, 1, 1, 0], [0, 2, 1, 0], [0, 2, 2, 1]];
    // Agent 1 pays road fuel 2 twice a day (right, then back), so its tank
    // falls by 4 per day — the same timeline a real server would report.
    let walker_fuel: [u32; DAYS] = [30, 26, 22, 18];

    let mut cells = vec![vec![CellType::TYPE_0; 8]; 8];
    for cell in cells[2].iter_mut().skip(4) {
        *cell = CellType::TYPE_1;
    }
    let setting = MatchSetting {
        starts_at: 1,
        day_seconds: vec![30; DAYS],
        day_steps: vec![BUDGET; DAYS],
        map: Map { height: 8, width: 8, cells },
        spots: vec![],
        agents: vec![20, 21, 32, 33, 34, 35, 36],
        fuel_limits: 30,
        players: 2,
        busy_threshold: 1,
        jammed_threshold: 3,
    };
    let base = cli::protocol_to_engine::protocol_setting_to_engine(
        &setting,
        domain::PlayerId::new("low-threshold").expect("valid team id"),
    )
    .expect("setting converts");

    // The validator's per-day view: the same map and agents `GET /` serves.
    let mut day_maps = Vec::with_capacity(DAYS);
    let mut day_agents = Vec::with_capacity(DAYS);
    let mut day_states = Vec::with_capacity(DAYS);
    for (day, statuses) in traffic_by_day.iter().enumerate() {
        let mut map = base.map.clone();
        for (&pos, &status) in ROAD_POS.iter().zip(statuses) {
            let coord = engine::model::HexCoord::new(pos % 8, pos / 8);
            let state = match status {
                1 => engine::model::TrafficState::Busy,
                2 => engine::model::TrafficState::Jammed,
                _ => engine::model::TrafficState::Smooth,
            };
            map.get_mut(coord).expect("road cell").terrain = engine::model::Terrain::Road(state);
        }
        let mut agents = base.agents.clone();
        agents[0].fuel = 1; // cannot pay the 2 fuel a road departure costs
        agents[1].fuel = walker_fuel[day];
        for agent in agents.iter_mut().skip(2) {
            agent.fuel = 30;
        }
        let traffics: Vec<serde_json::Value> = ROAD_POS
            .iter()
            .zip(statuses)
            .map(|(&pos, &status)| json!({ "pos": pos, "status": status }))
            .collect();
        day_states.push(json!({
            "endsAt": 0_i64,
            "day": i64::try_from(day).unwrap_or(0),
            "agents": [
                { "kind": 0, "pos": 20, "fuel": 1 },
                { "kind": 0, "pos": 21, "fuel": walker_fuel[day] },
                { "kind": 0, "pos": 32, "fuel": 30 },
                { "kind": 0, "pos": 33, "fuel": 30 },
                { "kind": 0, "pos": 34, "fuel": 30 },
                { "kind": 0, "pos": 35, "fuel": 30 },
                { "kind": 0, "pos": 36, "fuel": 30 },
            ],
            "others": [],
            "traffics": traffics,
        }));
        day_maps.push(map);
        day_agents.push(agents);
    }
    let day_states = Arc::new(day_states);
    let day_maps = Arc::new(day_maps);
    let day_agents = Arc::new(day_agents);

    let mut server = Server::new_async().await;
    let setting_body = serde_json::to_value(&setting).expect("serialise setting");
    let m_setting = server
        .mock("GET", "/setting")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(setting_body.to_string())
        .expect_at_least(1)
        .create_async()
        .await;
    let m_agent = server
        .mock("POST", "/agent")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("")
        .expect_at_least(1)
        .create_async()
        .await;

    // `endsAt` in the past skips the between-day sleep. The day the last
    // `GET /` served is remembered so `POST /` validates against exactly
    // the map/agents it was handed.
    let day_counter = Arc::new(AtomicUsize::new(0));
    let current_day = Arc::new(AtomicUsize::new(0));
    let dc = day_counter.clone();
    let cd = current_day.clone();
    let ds = day_states.clone();
    let m_state = server
        .mock("GET", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |_req| {
            let d = dc.fetch_add(1, Ordering::SeqCst);
            cd.store(d, Ordering::SeqCst);
            ds.get(d).or_else(|| ds.last()).expect("day state").to_string().into_bytes()
        })
        .expect_at_least(DAYS)
        .create_async()
        .await;

    let captured_bodies: Arc<Mutex<Vec<Vec<Vec<i32>>>>> =
        Arc::new(Mutex::new(Vec::with_capacity(DAYS)));
    let cb = captured_bodies.clone();
    let cd_for_post = current_day.clone();
    let maps = day_maps.clone();
    let agents = day_agents.clone();
    let revision_counter = Arc::new(AtomicI32::new(1));
    let rc = revision_counter.clone();
    // Official-rule gate (Issue #226 + #245): reject with `revision: -1`
    // whatever fails `engine::wire_plan::validate_agent_plan` — weighted
    // budget exact, every move legal under the official fuel rule.
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
            let day = cd_for_post.load(Ordering::SeqCst).min(maps.len() - 1);
            let map = &maps[day];
            let day_agents = &agents[day];
            let budget = u32::try_from(BUDGET).unwrap_or(0);
            let accepted = req
                .body()
                .ok()
                .and_then(|raw| serde_json::from_slice::<Vec<Vec<i32>>>(raw).ok())
                .is_some_and(|body| {
                    body.len() == day_agents.len()
                        && body.iter().enumerate().all(|(idx, plan)| {
                            engine::wire_plan::validate_agent_plan(
                                &day_agents[idx],
                                map,
                                budget,
                                plan,
                            )
                            .is_ok()
                        })
                });
            let body = if accepted {
                json!({ "revision": rc.fetch_add(1, Ordering::SeqCst) })
            } else {
                json!({ "revision": -1 })
            };
            body.to_string().into_bytes()
        })
        .expect_at_least(DAYS)
        .create_async()
        .await;

    let client = MatchClient::builder(server.url(), "low-threshold-token")
        .retry_policy(RetryPolicy { max_retries: 0, backoff_millis: &[] })
        .request_timeout(Duration::from_secs(5))
        .build()
        .expect("match client builds");
    let cfg = CoordinatorLoopConfig {
        team_id: "low-threshold".to_owned(),
        setting_poll_interval: Duration::from_millis(20),
        solver_name: "road-walker".to_owned(),
        agent_kinds: Some(vec![engine::model::AgentKind::Patrol; 7]),
        ..CoordinatorLoopConfig::default()
    };
    let outcome: CoordinatorOutcome = run_coordinator_loop(client, Box::new(RoadWalkerSolver), cfg)
        .await
        .expect("every day of the low-threshold match must be accepted");

    assert_eq!(outcome.days_completed, DAYS, "all days completed");
    assert!(outcome.revisions.iter().all(|&r| r >= 0), "revisions: {:?}", outcome.revisions);

    {
        let bodies = captured_bodies.lock().expect("captured_bodies lock poisoned");
        assert_eq!(bodies.len(), DAYS, "one POST / body per day");
        let budget = u32::try_from(BUDGET).expect("budget fits u32");
        for (day, body) in bodies.iter().enumerate() {
            assert_eq!(body.len(), 7, "day {day}: one inner list per agent");
            // Issue #245: agent 0 stands on a 道路 with `fuel 1`, and
            // leaving a road costs 2 fuel. The official server rejects the
            // whole day for that one action, so the boundary must drop the
            // step and pad with waits instead of shipping it.
            assert!(
                body[0].iter().all(|&code| code < 0),
                "day {day}: the unaffordable road departure must be dropped, got {:?}",
                body[0]
            );
            // Agent 1's two road steps must fit the day's traffic-weighted
            // budget exactly — the mock already enforced it, this pins the
            // value and the failure message.
            let weighted = engine::wire_plan::validate_agent_plan(
                &day_agents[day][1],
                &day_maps[day],
                budget,
                &body[1],
            )
            .unwrap_or_else(|e| panic!("day {day} agent 1: {e}"));
            assert_eq!(weighted, budget, "day {day}: agent 1 weighted sum");
            assert!(
                body[1].iter().any(|&code| code >= 0),
                "day {day}: agent 1 must keep its road walk"
            );
        }
        // Accumulated traffic must actually move the plan: a busy road bills
        // 2 steps and a jammed one 4, so day 1 and day 2 cannot share a body.
        assert_ne!(bodies[1][1], bodies[2][1], "busy vs jammed roads must split agent 1's plan");
    }

    m_setting.assert_async().await;
    m_agent.assert_async().await;
    m_state.assert_async().await;
    m_actions.assert_async().await;
}
