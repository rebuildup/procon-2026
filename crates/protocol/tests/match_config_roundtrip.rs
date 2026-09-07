//! Round-trip integration test for the `match_config` types.
//!
//! Loads the bundled Procon 37 `OpenAPI` spec fixture, constructs representative
//! sample payloads that conform to the schema, and verifies that each struct
//! serialises / deserialises without loss.

use std::fs;

use protocol::match_config::{
    AccessTimeError, AccessTimeErrorCode, ActionAccepted, AgentKind, AgentKindsBody, AgentState,
    AuthError, AuthErrorCode, CellType, Map, MatchSetting, MatchState, OtherPlayer, Spot,
    TrafficInfo, TrafficStatus,
};

const FIXTURE: &str = "tests/fixtures/procon37-openapi.json";

#[test]
fn fixture_is_valid_openapi_doc() {
    let raw = fs::read_to_string(FIXTURE).expect("fixture must exist");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("OpenAPI fixture must be JSON");

    assert_eq!(v["openapi"].as_str(), Some("3.1.0"));
    let schemas = v["components"]["schemas"].as_object().expect("schemas object");
    for expected in [
        "AccessTimeError",
        "ActionAccepted",
        "AgentKind",
        "AgentState",
        "AuthError",
        "CellType",
        "Map",
        "MatchSetting",
        "MatchState",
        "OtherPlayer",
        "Spot",
        "TrafficInfo",
        "TrafficStatus",
    ] {
        assert!(schemas.contains_key(expected), "missing schema {expected}");
    }
}

fn sample_map() -> Map {
    let cells = (0..8)
        .map(|i| {
            (0..8)
                .map(|j| match (i + j) % 4 {
                    0 => CellType::TYPE_0,
                    1 => CellType::TYPE_1,
                    2 => CellType::TYPE_2,
                    _ => CellType::TYPE_3,
                })
                .collect()
        })
        .collect();
    Map { height: 8, width: 8, cells }
}

#[test]
fn match_setting_round_trips() {
    let setting = MatchSetting {
        starts_at: 0,
        day_seconds: vec![60, 60],
        day_steps: vec![20, 24],
        map: sample_map(),
        spots: vec![Spot { brand: 0, pos: 7, stocks: 5 }, Spot { brand: 1, pos: 56, stocks: 3 }],
        agents: vec![3, 12, 21, 30],
        fuel_limits: 60,
        players: 2,
        busy_threshold: 3,
        jammed_threshold: 5,
    };

    let json = serde_json::to_string(&setting).unwrap();
    let back: MatchSetting = serde_json::from_str(&json).unwrap();
    assert_eq!(setting, back);

    // Spot-check camelCase field renames against the OpenAPI spec.
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v.get("startsAt").is_some());
    assert!(v.get("daySeconds").is_some());
    assert!(v.get("daySteps").is_some());
    assert!(v.get("fuelLimits").is_some());
    assert!(v.get("busyThreshold").is_some());
    assert!(v.get("jammedThreshold").is_some());
}

#[test]
fn match_state_round_trips() {
    let state = MatchState {
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
        traffics: vec![
            TrafficInfo { pos: 10, status: TrafficStatus::STATUS_0 },
            TrafficInfo { pos: 11, status: TrafficStatus::STATUS_1 },
            TrafficInfo { pos: 12, status: TrafficStatus::STATUS_2 },
        ],
    };

    let json = serde_json::to_string(&state).unwrap();
    let back: MatchState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);

    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v.get("endsAt").is_some());
}

#[test]
fn auth_and_access_time_errors_round_trip() {
    let auth = AuthError { code: AuthErrorCode::AuthError, message: "token is invalid".to_owned() };
    let access = AccessTimeError {
        code: AccessTimeErrorCode::AccessTimeError,
        message: "match has not started".to_owned(),
    };

    let auth_json = serde_json::to_string(&auth).unwrap();
    assert!(auth_json.contains("\"code\":\"AuthError\""));
    let auth_back: AuthError = serde_json::from_str(&auth_json).unwrap();
    assert_eq!(auth, auth_back);

    let access_json = serde_json::to_string(&access).unwrap();
    assert!(access_json.contains("\"code\":\"AccessTimeError\""));
    let access_back: AccessTimeError = serde_json::from_str(&access_json).unwrap();
    assert_eq!(access, access_back);
}

#[test]
fn action_accepted_round_trips_negative_revision() {
    let ok = ActionAccepted { revision: 7 };
    let json = serde_json::to_string(&ok).unwrap();
    let back: ActionAccepted = serde_json::from_str(&json).unwrap();
    assert_eq!(ok, back);

    let reject = ActionAccepted { revision: -1 };
    let json = serde_json::to_string(&reject).unwrap();
    let back: ActionAccepted = serde_json::from_str(&json).unwrap();
    assert_eq!(reject, back);
}

#[test]
fn agent_kinds_body_round_trips() {
    let body: AgentKindsBody = vec![AgentKind::KIND_0, AgentKind::KIND_1, AgentKind::KIND_0];
    let json = serde_json::to_string(&body).unwrap();
    // The OpenAPI spec requires a flat array of integers (no object wrapper).
    assert_eq!(json, "[0,1,0]");
    let back: AgentKindsBody = serde_json::from_str(&json).unwrap();
    assert_eq!(body, back);
}

#[test]
fn invalid_agent_kind_value_is_rejected() {
    // Spec: AgentKind is restricted to 0..=1. A bogus value must fail.
    let bad = serde_json::json!([2]);
    let result: Result<AgentKindsBody, _> = serde_json::from_value(bad);
    assert!(result.is_err(), "value 2 must be rejected for AgentKind");
}

#[test]
fn missing_required_field_is_rejected() {
    // `MatchSetting` requires `busyThreshold`; an object lacking it must fail.
    let bad = serde_json::json!({
        "startsAt": 0,
        "daySeconds": [],
        "daySteps": [],
        "map": { "height": 8, "width": 8, "cells": [] },
        "spots": [],
        "agents": [],
        "fuelLimits": 1,
        "players": 2,
        "jammedThreshold": 5
    });
    let result: Result<MatchSetting, _> = serde_json::from_value(bad);
    assert!(result.is_err(), "missing busyThreshold must fail");
}

#[test]
fn agent_state_nested_in_match_state_round_trips() {
    let original = MatchState {
        ends_at: 42,
        day: 1,
        agents: vec![AgentState { kind: AgentKind::KIND_0, pos: 5, fuel: 9 }],
        others: vec![],
        traffics: vec![],
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["agents"][0]["kind"].as_i64(), Some(0));
    assert_eq!(parsed["traffics"].as_array().map(Vec::len), Some(0));
}
