//! Typed mirror of the Procon 37 official `OpenAPI` schema.
//!
//! These types describe the wire payload exchanged with the contest server.
//! They are kept distinct from the engine's internal domain model
//! (`crates/domain`) so that the official spec can evolve independently of
//! our internal representation.
//!
//! Source: `.reference/procon37/openapi.json` (extracted from the official
//! 「回答システムに関する情報」 ZIP distributed by the contest organisers).
//! All field names match the JSON wire format verbatim — every struct uses
//! `#[serde(rename_all = "camelCase")]`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Error returned with HTTP 403 when the request is made outside the
/// scheduled match window (before start or after finish).
///
/// Spec: `AccessTimeError` schema, code literal `"AccessTimeError"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessTimeError {
    /// Discriminator literal — always `"AccessTimeError"`.
    pub code: AccessTimeErrorCode,
    /// Human-readable explanation, e.g. `"試合がまだ開始されていません"`.
    pub message: String,
}

/// Discriminator for [`AccessTimeError`]. Only one literal variant exists in
/// the `OpenAPI` schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessTimeErrorCode {
    #[serde(rename = "AccessTimeError")]
    AccessTimeError,
}

/// Response of `POST /` when an action plan is accepted.
///
/// Spec: `ActionAccepted` schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionAccepted {
    /// Revision number. Negative values mean rejection; non-negative values
    /// indicate acceptance. Within a single day, a higher value represents a
    /// later submission (relevant when parallel requests race).
    pub revision: i32,
}

/// Which agent archetype occupies a given slot.
///
/// Spec: `AgentKind` schema — JSON number in `{0, 1}`.
///
/// We use a thin newtype instead of a serde enum so that the value
/// serialises as a JSON **number** rather than a quoted string. Only the two
/// documented values are accepted; any other integer is rejected by serde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentKind(i32);

impl AgentKind {
    /// Agent kind `0`.
    pub const KIND_0: Self = Self(0);
    /// Agent kind `1`.
    pub const KIND_1: Self = Self(1);

    /// Raw discriminant value (`0` or `1`).
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }
}

impl Serialize for AgentKind {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_i32(self.0)
    }
}

impl<'de> Deserialize<'de> for AgentKind {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let v = i32::deserialize(de)?;
        match v {
            0 => Ok(Self::KIND_0),
            1 => Ok(Self::KIND_1),
            other => {
                Err(serde::de::Error::custom(format!("AgentKind must be 0 or 1, got {other}")))
            }
        }
    }
}

/// Per-agent runtime state.
///
/// Spec: `AgentState` schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    pub kind: AgentKind,
    /// Cell index on the board (row-major linearisation of `Map.cells`).
    pub pos: i32,
    /// Remaining fuel for the current day.
    pub fuel: i32,
}

/// Error returned with HTTP 401 when authentication fails.
///
/// Spec: `AuthError` schema, code literal `"AuthError"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthError {
    pub code: AuthErrorCode,
    /// Human-readable authentication failure message.
    pub message: String,
}

/// Discriminator for [`AuthError`]. Only one literal variant exists in the
/// `OpenAPI` schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthErrorCode {
    #[serde(rename = "AuthError")]
    AuthError,
}

/// Cell terrain type. The mapping from integer to terrain is documented in
/// `docs/pdf/競技部門「ヘキサうどん_」のフォーマットについて.pdf`.
///
/// Spec: `CellType` schema — JSON number in `{0, 1, 2, 3}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellType(i32);

impl CellType {
    /// Cell type `0`.
    pub const TYPE_0: Self = Self(0);
    /// Cell type `1`.
    pub const TYPE_1: Self = Self(1);
    /// Cell type `2`.
    pub const TYPE_2: Self = Self(2);
    /// Cell type `3`.
    pub const TYPE_3: Self = Self(3);

    /// Raw discriminant value (`0..=3`).
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }
}

impl Serialize for CellType {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_i32(self.0)
    }
}

impl<'de> Deserialize<'de> for CellType {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let v = i32::deserialize(de)?;
        match v {
            0 => Ok(Self::TYPE_0),
            1 => Ok(Self::TYPE_1),
            2 => Ok(Self::TYPE_2),
            3 => Ok(Self::TYPE_3),
            other => Err(serde::de::Error::custom(format!("CellType must be 0..=3, got {other}"))),
        }
    }
}

/// Hexagonal board layout. `cells[y][x]` gives the terrain of the cell at
/// `(x, y)`; cells are row-major.
///
/// Spec: `Map` schema. Constraints: `8 <= height <= 32`, `8 <= width <= 32`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Map {
    pub height: i32,
    pub width: i32,
    /// 2-D row-major cell array of shape `[height][width]`.
    pub cells: Vec<Vec<CellType>>,
}

/// Static configuration returned by `GET /setting` before the match begins.
///
/// Spec: `MatchSetting` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSetting {
    /// UNIX-time match start. `0` while not yet determined.
    pub starts_at: i64,
    /// Per-day answer window in seconds, indexed by day.
    pub day_seconds: Vec<i32>,
    /// Per-day step count. Range is `W+H ..= (W+H)*4`.
    pub day_steps: Vec<i32>,
    pub map: Map,
    /// Spot definitions (うどんの販売所).
    pub spots: Vec<Spot>,
    /// Initial cell indices of the calling player's agents.
    pub agents: Vec<i32>,
    /// Maximum fuel load (positive; 1x..=3x of day-1 step count).
    pub fuel_limits: i32,
    /// Number of participating players.
    pub players: i32,
    /// "Busy" traffic threshold (1..=5).
    pub busy_threshold: i32,
    /// "Jammed" traffic threshold (2..=10).
    pub jammed_threshold: i32,
}

/// State snapshot returned by `GET /` at the start of each day.
///
/// Spec: `MatchState` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchState {
    /// UNIX-time deadline for the current day's submission.
    pub ends_at: i64,
    /// Day index (0-based; day 0 is the first day).
    pub day: i32,
    /// Calling player's agents.
    pub agents: Vec<AgentState>,
    /// Other players' agents (id + their agents).
    pub others: Vec<OtherPlayer>,
    /// Current road-status information.
    pub traffics: Vec<TrafficInfo>,
}

/// Minimal projection of another player's agents.
///
/// Spec: `OtherPlayer` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherPlayer {
    /// Player id assigned by the contest server.
    pub id: i32,
    pub agents: Vec<AgentState>,
}

/// A point of interest (うどん販売所) on the board.
///
/// Spec: `Spot` schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spot {
    /// Brand identifier.
    pub brand: i32,
    /// Cell index on the board.
    pub pos: i32,
    /// Maximum stock capacity (`>= 1`).
    pub stocks: i32,
}

/// Road congestion at a single edge / cell.
///
/// Spec: `TrafficInfo` schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficInfo {
    pub pos: i32,
    pub status: TrafficStatus,
}

/// Discrete traffic status.
///
/// Spec: `TrafficStatus` schema — JSON number in `{0, 1, 2}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrafficStatus(i32);

impl TrafficStatus {
    /// Traffic status `0`.
    pub const STATUS_0: Self = Self(0);
    /// Traffic status `1`.
    pub const STATUS_1: Self = Self(1);
    /// Traffic status `2`.
    pub const STATUS_2: Self = Self(2);

    /// Raw discriminant value (`0..=2`).
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }
}

impl Serialize for TrafficStatus {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_i32(self.0)
    }
}

impl<'de> Deserialize<'de> for TrafficStatus {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let v = i32::deserialize(de)?;
        match v {
            0 => Ok(Self::STATUS_0),
            1 => Ok(Self::STATUS_1),
            2 => Ok(Self::STATUS_2),
            other => {
                Err(serde::de::Error::custom(format!("TrafficStatus must be 0..=2, got {other}")))
            }
        }
    }
}

/// Body posted to `POST /` (full action plan for all agents).
///
/// Each inner list is one agent's action sequence for the day:
///
/// - `-N` (N > 0): wait N steps.
/// - `0..=5`: move in the matching hex direction
///   (`0:左上, 1:右上, 2:右, 3:右下, 4:左下, 5:左`).
///
/// Validation (per the `OpenAPI` description):
///
/// - the sum of steps must equal the day's step budget;
/// - moves must respect adjacency rules and remaining fuel;
/// - any invalid agent rejects the whole submission.
pub type ActionPlanBody = Vec<Vec<i32>>;

/// Body posted to `POST /agent` — one [`AgentKind`] per agent.
pub type AgentKindsBody = Vec<AgentKind>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_kind_serialises_as_number() {
        let json = serde_json::to_string(&AgentKind::KIND_0).unwrap();
        assert_eq!(json, "0");
        let json = serde_json::to_string(&AgentKind::KIND_1).unwrap();
        assert_eq!(json, "1");
    }

    #[test]
    fn agent_kind_round_trips() {
        for k in [AgentKind::KIND_0, AgentKind::KIND_1] {
            let json = serde_json::to_string(&k).unwrap();
            let back: AgentKind = serde_json::from_str(&json).unwrap();
            assert_eq!(k, back);
        }
    }

    #[test]
    fn cell_type_serialises_as_number() {
        let json = serde_json::to_string(&CellType::TYPE_2).unwrap();
        assert_eq!(json, "2");
        let back: CellType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CellType::TYPE_2);
    }

    #[test]
    fn traffic_status_serialises_as_number() {
        let json = serde_json::to_string(&TrafficStatus::STATUS_1).unwrap();
        assert_eq!(json, "1");
        let back: TrafficStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TrafficStatus::STATUS_1);
    }

    #[test]
    fn auth_error_round_trips() {
        let err = AuthError { code: AuthErrorCode::AuthError, message: "invalid token".to_owned() };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"AuthError\""));
        let back: AuthError = serde_json::from_str(&json).unwrap();
        assert_eq!(err, back);
    }

    #[test]
    fn access_time_error_round_trips() {
        let err = AccessTimeError {
            code: AccessTimeErrorCode::AccessTimeError,
            message: "before match".to_owned(),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"AccessTimeError\""));
        let back: AccessTimeError = serde_json::from_str(&json).unwrap();
        assert_eq!(err, back);
    }

    #[test]
    fn camel_case_fields_are_used() {
        let m = MatchSetting {
            starts_at: 0,
            day_seconds: vec![60],
            day_steps: vec![10],
            map: Map { height: 8, width: 8, cells: vec![vec![CellType::TYPE_0; 8]; 8] },
            spots: vec![],
            agents: vec![0],
            fuel_limits: 30,
            players: 2,
            busy_threshold: 3,
            jammed_threshold: 5,
        };
        let v: serde_json::Value = serde_json::to_value(&m).unwrap();
        assert!(v.get("startsAt").is_some());
        assert!(v.get("daySeconds").is_some());
        assert!(v.get("fuelLimits").is_some());
        assert!(v.get("busyThreshold").is_some());
        assert!(v.get("jammedThreshold").is_some());
        assert!(v.get("starts_at").is_none());
    }
}
