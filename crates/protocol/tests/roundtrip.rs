//! Round-trip integration tests for the `protocol` crate.
//!
//! These tests cover three layers:
//!
//! 1. **Local `MatchConfig` stub** — mirrors the camelCase wire format that
//!    the Procon 37 `OpenAPI` spec uses. The full mirror lives in
//!    `crates/protocol/src/match_config.rs` on the `#14` branch; this test
//!    uses a minimal local stub so we can property-test the camelCase
//!    round-trip independently of the (much larger) `#14` fixture set.
//!    When `#14` lands, this stub can be deleted.
//! 2. **Existing wire types** — `Hello`, `Envelope`, `MatchSnapshot`,
//!    `MatchSummary`, `ClientEvent`, `ServerEvent`, `ClientFrame`,
//!    `ServerFrame`. All of these are part of the protocol v1 surface
//!    today, so a property-based round-trip keeps serde regressions out.
//! 3. **Fixture integrity** — the bundled OpenAPI-like fixture under
//!    `tests/fixtures/` must be valid JSON and contain the schemas we
//!    expect to cover.
//!
//! The `--ignored` tests in `heavy_*` exercise larger inputs and are only
//! run via `cargo test --workspace -- --ignored`.

use std::fs;

use proptest::prelude::*;
use protocol::{
    ClientEvent, ClientFrame, Envelope, Hello, MatchSnapshot, MatchSummary, PROTOCOL_VERSION,
    ProtocolVersion, ServerEvent, ServerFrame,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Local MatchConfig stub (camelCase wire format)
// ---------------------------------------------------------------------------
//
// This stub intentionally shadows the (much larger) `match_config`
// module that lands in `#14`. Its purpose is solely to give the property
// tests here a small, controllable `MatchConfig` whose wire format
// matches the official `OpenAPI` JSON.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MatchConfig {
    starts_at: i64,
    day_seconds: Vec<i64>,
    day_steps: Vec<i64>,
    fuel_limits: i64,
    players: u32,
    busy_threshold: i32,
    jammed_threshold: i32,
}

/// `(day_seconds, day_steps)` are constrained to have the same length so
/// the resulting `MatchConfig` is a structurally valid input. We pack the
/// length and emit both vectors together to avoid the all-combinations
/// explosion of two independent ranges.
fn arb_day_vectors() -> impl Strategy<Value = (Vec<i64>, Vec<i64>)> {
    (1usize..=7).prop_flat_map(|len| {
        (proptest::collection::vec(1i64..=3600, len), proptest::collection::vec(1i64..=1000, len))
    })
}

fn arb_match_config() -> impl Strategy<Value = MatchConfig> {
    (0i64..=2_000_000_000, arb_day_vectors(), 1i64..=10_000, 1u32..=8, 0i32..=20, 0i32..=40)
        .prop_map(|(starts_at, (day_seconds, day_steps), fuel_limits, players, busy, jammed)| {
            MatchConfig {
                starts_at,
                day_seconds,
                day_steps,
                fuel_limits,
                players,
                busy_threshold: busy,
                jammed_threshold: jammed,
            }
        })
}

// ---------------------------------------------------------------------------
// Strategies for existing wire types
// ---------------------------------------------------------------------------

fn arb_protocol_version() -> impl Strategy<Value = ProtocolVersion> {
    (0u16..=u16::MAX, 0u16..=u16::MAX).prop_map(|(major, minor)| ProtocolVersion { major, minor })
}

fn arb_hello() -> impl Strategy<Value = Hello> {
    (
        arb_protocol_version(),
        proptest::string::string_regex("[a-z0-9-]{1,32}").expect("valid regex"),
        proptest::collection::vec(
            proptest::string::string_regex("[a-z0-9_-]{1,16}").expect("valid regex"),
            0..=4,
        ),
    )
        .prop_map(|(protocol, server_id, supported_features)| Hello {
            protocol,
            server_id,
            supported_features,
        })
}

fn arb_envelope() -> impl Strategy<Value = Envelope> {
    (
        arb_protocol_version(),
        proptest::string::string_regex("[a-z_]{1,32}").expect("valid regex"),
        proptest::collection::vec(any::<i64>(), 0..=4),
    )
        .prop_map(|(protocol, kind, nums)| Envelope {
            protocol,
            kind,
            body: serde_json::Value::Array(nums.into_iter().map(serde_json::Value::from).collect()),
        })
}

fn arb_client_event() -> impl Strategy<Value = ClientEvent> {
    prop_oneof![
        ("p[0-9]{1,4}").prop_map(|player| ClientEvent::Hello {
            player: domain::PlayerId::new(player).expect("non-empty")
        }),
        ("m[0-9]{1,4}", 1u16..=u16::MAX, proptest::collection::vec(any::<i64>(), 0..=4),).prop_map(
            |(match_id, action_index, payload)| ClientEvent::SubmitTurn {
                match_id: domain::MatchId::new(match_id).expect("non-empty"),
                action_index,
                payload: serde_json::Value::Array(
                    payload.into_iter().map(serde_json::Value::from).collect(),
                ),
            }
        ),
        ("m[0-9]{1,4}").prop_map(|match_id| ClientEvent::RequestSnapshot {
            match_id: domain::MatchId::new(match_id).expect("non-empty"),
        }),
        any::<u64>().prop_map(|nonce| ClientEvent::Ping { nonce }),
    ]
}

fn arb_server_event() -> impl Strategy<Value = ServerEvent> {
    prop_oneof![
        (
            proptest::string::string_regex("[a-z0-9-]{1,16}").expect("valid regex"),
            proptest::collection::vec(
                proptest::string::string_regex("[a-z0-9_-]{1,8}").expect("valid regex"),
                0..=3,
            ),
        )
            .prop_map(|(server_id, features)| ServerEvent::Welcome { server_id, features }),
        ("m[0-9]{1,4}", any::<u32>()).prop_map(|(match_id, turn)| ServerEvent::Snapshot {
            match_id: domain::MatchId::new(match_id).expect("non-empty"),
            turn,
        }),
        ("m[0-9]{1,4}", proptest::string::string_regex("[a-z0-9_-]{1,8}").expect("valid regex"),)
            .prop_map(|(match_id, turn_id)| ServerEvent::TurnAccepted {
                match_id: domain::MatchId::new(match_id).expect("non-empty"),
                turn_id,
            }),
        ("m[0-9]{1,4}", proptest::string::string_regex("[a-z0-9 ]{1,16}").expect("valid regex"),)
            .prop_map(|(match_id, reason)| ServerEvent::TurnRejected {
                match_id: domain::MatchId::new(match_id).expect("non-empty"),
                reason,
            }),
        any::<u64>().prop_map(|nonce| ServerEvent::Pong { nonce }),
        proptest::string::string_regex("[a-z0-9 ]{1,16}")
            .expect("valid regex")
            .prop_map(|message| ServerEvent::Error { message }),
    ]
}

// ---------------------------------------------------------------------------
// MatchConfig (camelCase) round-trip property tests
// ---------------------------------------------------------------------------

proptest! {
    /// The full `MatchConfig` payload round-trips through JSON without
    /// loss when the `camelCase` rename rule is in effect.
    #[test]
    fn match_config_round_trip(cfg in arb_match_config()) {
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: MatchConfig = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&cfg, &back);
    }

    /// Field names on the wire must be camelCase, NOT snake_case. A
    /// future change to `#[serde(rename_all)]` would silently break the
    /// protocol contract — this test catches it.
    #[test]
    fn match_config_uses_camel_case_on_the_wire(cfg in arb_match_config()) {
        let json = serde_json::to_string(&cfg).expect("serialize");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("deserialize back to value");

        for key in [
            "startsAt",
            "daySeconds",
            "daySteps",
            "fuelLimits",
            "busyThreshold",
            "jammedThreshold",
            "players",
        ] {
            prop_assert!(
                value.get(key).is_some(),
                "missing camelCase key {key} in {json}"
            );
        }
        for forbidden in [
            "starts_at",
            "day_seconds",
            "day_steps",
            "fuel_limits",
            "busy_threshold",
            "jammed_threshold",
        ] {
            prop_assert!(
                value.get(forbidden).is_none(),
                "unexpected snake_case key {forbidden} in {json}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Existing wire type round-trips
// ---------------------------------------------------------------------------

/// `PROTOCOL_VERSION` (the global constant) is a stable contract.
#[test]
fn protocol_version_constant_round_trips() {
    let json = serde_json::to_string(&PROTOCOL_VERSION).expect("serialize");
    let back: ProtocolVersion = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(PROTOCOL_VERSION, back);
    assert_eq!(PROTOCOL_VERSION.major, 1);
    assert_eq!(PROTOCOL_VERSION.minor, 0);
}

proptest! {
    #[test]
    fn hello_round_trip(hello in arb_hello()) {
        let json = serde_json::to_string(&hello).expect("serialize");
        let back: Hello = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&hello, &back);
    }

    #[test]
    fn envelope_round_trip(env in arb_envelope()) {
        let json = serde_json::to_string(&env).expect("serialize");
        let back: Envelope = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&env, &back);
    }

    #[test]
    fn client_event_round_trip(ev in arb_client_event()) {
        let json = serde_json::to_string(&ev).expect("serialize");
        let back: ClientEvent = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&ev, &back);
        let v: serde_json::Value = serde_json::from_str(&json).expect("value");
        let kind = v.get("type").and_then(|s| s.as_str()).expect("type tag");
        prop_assert!(
            ["hello", "submit_turn", "request_snapshot", "ping"].contains(&kind),
            "unknown ClientEvent type tag: {kind}"
        );
    }

    #[test]
    fn server_event_round_trip(ev in arb_server_event()) {
        let json = serde_json::to_string(&ev).expect("serialize");
        let back: ServerEvent = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&ev, &back);
    }

    #[test]
    fn match_snapshot_round_trip(
        match_id_str in "m[0-9]{1,4}",
        turn in any::<u32>(),
        outcome_kind in 0u8..3,
    ) {
        use domain::state::{GameState, Outcome, Score};
        let match_id = domain::MatchId::new(match_id_str).expect("non-empty");
        let author = domain::PlayerId::new("p-test").expect("non-empty");
        let outcome = match outcome_kind {
            0 => Outcome::Pending,
            1 => Outcome::Draw,
            _ => Outcome::Winner(author.clone()),
        };
        let snap = MatchSnapshot {
            match_id: match_id.clone(),
            state: GameState {
                match_id,
                turn,
                scores: vec![(author, Score(0))],
            },
            outcome,
            server_time_unix_ms: 1_700_000_000,
        };

        let json = serde_json::to_string(&snap).expect("serialize");
        let back: MatchSnapshot = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&snap, &back);
    }

    #[test]
    fn match_summary_round_trip(
        outcome_kind in 0u8..3,
        turn in any::<u32>(),
    ) {
        use domain::state::{GameState, Outcome, Score};
        let match_id = domain::MatchId::new("m-summary").expect("non-empty");
        let author = domain::PlayerId::new("p-test").expect("non-empty");
        let outcome = match outcome_kind {
            0 => Outcome::Pending,
            1 => Outcome::Draw,
            _ => Outcome::Winner(author.clone()),
        };
        let snap = MatchSnapshot {
            match_id: match_id.clone(),
            state: GameState {
                match_id: match_id.clone(),
                turn,
                scores: vec![(author.clone(), Score(0))],
            },
            outcome,
            server_time_unix_ms: 0,
        };

        let summary = MatchSummary::from(&snap);
        let json = serde_json::to_string(&summary).expect("serialize");
        let back: MatchSummary = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&summary, &back);
    }

    #[test]
    fn client_frame_round_trip(events in proptest::collection::vec(arb_client_event(), 0..=4)) {
        let frame = ClientFrame { events };
        let json = serde_json::to_string(&frame).expect("serialize");
        let back: ClientFrame = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&frame, &back);
    }

    #[test]
    fn server_frame_round_trip(events in proptest::collection::vec(arb_server_event(), 0..=4)) {
        let frame = ServerFrame { events };
        let json = serde_json::to_string(&frame).expect("serialize");
        let back: ServerFrame = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&frame, &back);
    }
}

// ---------------------------------------------------------------------------
// Fixture integrity
// ---------------------------------------------------------------------------

const FIXTURE: &str = "tests/fixtures/procon37-openapi-minimal.json";

/// The bundled fixture must exist and be valid JSON.
#[test]
fn fixture_is_valid_json() {
    let raw = fs::read_to_string(FIXTURE).expect("fixture must exist on disk");
    let _value: serde_json::Value =
        serde_json::from_str(&raw).expect("OpenAPI fixture must parse as JSON");
}

/// The fixture must declare the schemas we test against.
#[test]
fn fixture_lists_expected_schemas() {
    let raw = fs::read_to_string(FIXTURE).expect("fixture must exist on disk");
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("OpenAPI fixture must parse as JSON");

    let schemas =
        value["components"]["schemas"].as_object().expect("components.schemas must be an object");

    for expected in ["MatchConfig", "MatchState", "AgentState", "CellType", "TrafficStatus"] {
        assert!(schemas.contains_key(expected), "missing schema {expected} in fixture");
    }
}

// ---------------------------------------------------------------------------
// Heavy / long-running property tests (run via `cargo test -- --ignored`)
// ---------------------------------------------------------------------------

#[ignore = "heavy property test, run via `cargo test -- --ignored`"]
#[test]
fn heavy_match_config_round_trip_thousands() {
    let cfgs = proptest::collection::vec(arb_match_config(), 1..=16);
    proptest!(ProptestConfig::with_cases(512), |(cfgs in cfgs)| {
        for cfg in &cfgs {
            let json = serde_json::to_string(cfg).expect("serialize");
            let back: MatchConfig = serde_json::from_str(&json).expect("deserialize");
            prop_assert_eq!(cfg, &back);
        }
    });
}

#[ignore = "heavy property test, run via `cargo test -- --ignored`"]
#[test]
fn heavy_event_streams_round_trip() {
    let client = proptest::collection::vec(arb_client_event(), 1..=32);
    let server = proptest::collection::vec(arb_server_event(), 1..=32);
    proptest!(
        ProptestConfig::with_cases(64),
        |(client in client, server in server)| {
            let cf = ClientFrame { events: client };
            let sf = ServerFrame { events: server };

            let cf_json = serde_json::to_string(&cf).expect("serialize");
            let sf_json = serde_json::to_string(&sf).expect("serialize");

            let cf_back: ClientFrame =
                serde_json::from_str(&cf_json).expect("deserialize");
            let sf_back: ServerFrame =
                serde_json::from_str(&sf_json).expect("deserialize");

            prop_assert_eq!(&cf, &cf_back);
            prop_assert_eq!(&sf, &sf_back);
        }
    );
}
