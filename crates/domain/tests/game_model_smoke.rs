//! Smoke tests for the [`domain::game_model::GameModel`] static
//! configuration type.
//!
//! These tests are intentionally limited to the "smoke" gate (≥3
//! cases): valid + invalid + serde round-trip. Deeper coverage (each
//! validator boundary, every error variant) lives next to the type in
//! `crates/domain/src/game_model.rs`.
//!
//! See `docs/contest/api.md` §5.1 for the spec fields this type
//! mirrors (`MatchSetting`).

use domain::coordinate::MapSize;
use domain::game_model::{GameModel, GameModelError};

// ---------------------------------------------------------------------------
// Acceptance
// ---------------------------------------------------------------------------

/// A [`GameModel`] built via [`GameModel::canonical`] must pass
/// [`GameModel::validate`] unchanged.
#[test]
fn valid_game_model_validates() {
    let m = GameModel::canonical();
    assert!(m.validate().is_ok(), "canonical defaults must validate");
}

/// Constructing a [`GameModel`] with `agent_count == 0` (below the
/// 募集要項 §エージェント minimum of 3) must fail validation with a
/// clearly-typed error variant — not a panic, not a generic
/// `InvariantViolation`, but the [`GameModelError::AgentCountOutOfRange`]
/// case so callers can branch on it.
#[test]
fn invalid_zero_agents_fails_with_clear_error() {
    let mut m = GameModel::canonical();
    m.agent_count = 0;

    let err = m.validate().expect_err("0 agents must fail validation");
    match err {
        GameModelError::AgentCountOutOfRange { count } => {
            assert_eq!(count, 0, "error must report the offending count");
        }
        other => panic!("expected AgentCountOutOfRange, got {other:?}"),
    }
}

/// A round-trip through `serde_json` must reproduce a [`GameModel`]
/// field-for-field so it can be persisted and replayed.
#[test]
fn round_trip_serde_preserves_all_fields() {
    let m = GameModel::canonical();

    let json = serde_json::to_string(&m).expect("serialize");
    let back: GameModel = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(m, back, "serialised GameModel must round-trip");
}

// ---------------------------------------------------------------------------
// Extra coverage (still smoke-level): exercise the most common
// invalid-input branches so a regression shows up immediately.
// ---------------------------------------------------------------------------

/// `patrol_fuel_limit > day_steps * 3` is the canonical Q14 violation.
/// The error must carry both the offending fuel value and the cap so
/// the diagnostic is self-explanatory.
#[test]
fn fuel_limit_above_q14_cap_is_rejected() {
    let mut m = GameModel::canonical();
    let cap = m.derived_patrol_fuel_cap();
    m.patrol_fuel_limit = cap + 1;

    match m.validate() {
        Err(GameModelError::FuelLimitExceedsCap { fuel, cap: reported }) => {
            assert_eq!(fuel, cap + 1);
            assert_eq!(reported, cap);
        }
        other => panic!("expected FuelLimitExceedsCap, got {other:?}"),
    }
}

/// Spot count > 32 (Q&A Q15 upper bound) must be rejected.
#[test]
fn spot_count_above_q15_max_is_rejected() {
    let mut m = GameModel::canonical();
    m.spot_count = GameModel::SPOT_COUNT_MAX + 1;

    let err = m.validate().expect_err("33 spots must fail Q&A Q15");
    assert!(
        matches!(err, GameModelError::SpotCountOutOfRange { .. }),
        "expected SpotCountOutOfRange, got {err:?}"
    );
}

/// Custom but spec-legal values must validate. This guards against
/// regression in the validation ordering (e.g. accidentally short-
/// circuiting before checking all fields).
#[test]
fn custom_valid_model_validates() {
    let m = GameModel::new(
        MapSize::new(20, 24).expect("20x24 in [8, 32]"),
        5, // turn_count
        domain::agent::DaySteps::new(176).expect("176 in [1, 1024]"),
        domain::agent::DaySeconds::new(120).expect("120 in [1, 3600]"),
        6,       // agent_count
        16,      // spot_count
        176 * 3, // patrol_fuel_limit at Q14 cap
        3,       // busy_threshold
        7,       // jammed_threshold
    )
    .expect("all values are in spec range");

    assert_eq!(m.map_size.width, 20);
    assert_eq!(m.map_size.height, 24);
    assert_eq!(m.turn_count, 5);
    assert_eq!(m.agent_count, 6);
    assert_eq!(m.spot_count, 16);
    assert_eq!(m.busy_threshold, 3);
    assert_eq!(m.jammed_threshold, 7);
    assert!(m.validate().is_ok());
}

/// Round-tripping a custom (non-canonical) model must reproduce it.
#[test]
fn custom_model_serde_round_trip() {
    let m = GameModel::new(
        MapSize::new(10, 10).unwrap(),
        2,
        domain::agent::DaySteps::new(40).unwrap(),
        domain::agent::DaySeconds::new(60).unwrap(),
        5,
        8,
        120,
        2,
        5,
    )
    .expect("valid");

    let json = serde_json::to_string(&m).unwrap();
    let back: GameModel = serde_json::from_str(&json).unwrap();
    assert_eq!(m, back);
}
