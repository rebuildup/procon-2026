//! Canonical **static** configuration of a single match.
//!
//! [`GameModel`] is the single value that summarises the spec-bound,
//! pre-match parameters of a ヘキサうどん match: map size, turn count,
//! day budget (steps + wall-clock seconds), agent / spot counts, the
//! Patrol fuel cap, and the traffic-congestion thresholds. It is the
//! "model summary" type referenced by Issue #4.
//!
//! ## Why a separate "static" type?
//!
//! A match has two clearly distinct layers of state:
//!
//! - **Static configuration** — immutable once the match starts:
//!   map size, turn count, fuel cap, traffic thresholds, ...
//! - **Dynamic state** — evolves per day / per step: agent positions,
//!   fuel counters, traffic volumes, per-day variety, ...
//!
//! Bundling both into a single struct makes it hard to enforce that
//! the static parameters stay inside the documented contest ranges
//! while the dynamic state evolves. [`GameModel`] therefore only
//! carries the static layer; the dynamic state lives on
//! `MatchState` ([`crate::agent::MatchState`]) and the per-step
//! snapshots live in the engine / solver crates.
//!
//! ## Validation
//!
//! [`GameModel::validate`] re-checks every spec-bound invariant after
//! construction (including after deserialisation) and returns a
//! [`GameModelError`] describing the first violation. This makes the
//! type safe to receive from JSON / network inputs without trusting
//! the producer.
//!
//! ## Spec sources
//!
//! | Field / invariant | Source |
//! |---|---|
//! | `map_size ∈ [8, 32]²` | 募集要項 §マップ / Q&A **Q2** |
//! | `agent_count ∈ [3, 8]` | 募集要項 §エージェント |
//! | `spot_count ≤ 32`     | Q&A **Q15** |
//! | `patrol_fuel_limit ≤ day_steps * 3` | Q&A **Q14** |
//! | `busy_threshold ∈ [1, 5]`, `jammed_threshold ∈ [2, 10]`, `jammed > busy` | Q&A **Q13**, **Q19** |
//! | `day_steps ∈ [1, 1024]` (domain-side bound) | Q&A **Q6** + 募集要項 §行動 |
//! | `day_seconds ∈ [1, 3600]` | 募集要項 §行動生成 (リアルタイム要件) |
//! | `turn_count ≥ 1` | match termination rule (募集要項 §試合進行) |
//!
//! Per-map, the official `OpenAPI` bound on `daySteps` is
//! `[W + H, (W + H) * 4]`. [`GameModel::validate`] uses the looser
//! domain-side upper bound (`1024`) so the same static config can be
//! re-used across map sizes; tighter per-map checks belong to the
//! protocol layer that knows the exact `W + H`.

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::agent::{DaySeconds, DaySteps};
use crate::cell::validate_traffic_thresholds;
use crate::coordinate::MapSize;

// =============================================================================
// Error type
// =============================================================================

/// Errors returned by [`GameModel::validate`].
///
/// Each variant carries the offending value(s) so callers can surface a
/// useful diagnostic without having to re-parse the error message.
#[derive(Debug, thiserror::Error)]
pub enum GameModelError {
    /// The map size is outside the contest range `[8, 32]` per side.
    #[error("map size {width}x{height} out of contest range [8, 32]")]
    MapSizeOutOfRange {
        /// Supplied width.
        width: u8,
        /// Supplied height.
        height: u8,
    },
    /// The number of turns is outside the legal range.
    #[error("turn count {turns} out of range [1, 365]")]
    InvalidTurnCount {
        /// Supplied turn count.
        turns: u32,
    },
    /// The number of agents is outside `[3, 8]`.
    #[error("agent count {count} out of contest range [3, 8]")]
    AgentCountOutOfRange {
        /// Supplied agent count.
        count: u8,
    },
    /// The number of spots exceeds the documented maximum.
    #[error("spot count {count} exceeds contest maximum {max}")]
    SpotCountOutOfRange {
        /// Supplied spot count.
        count: u32,
        /// Documented upper bound (Q&A Q15).
        max: u32,
    },
    /// The Patrol fuel limit exceeds `day_steps * 3` (Q&A Q14).
    #[error("patrol fuel limit {fuel} exceeds day_steps*3 = {cap}")]
    FuelLimitExceedsCap {
        /// Supplied fuel limit.
        fuel: u32,
        /// Day-step-derived upper bound.
        cap: u32,
    },
    /// Traffic thresholds violate the Q&A Q13 ranges or ordering.
    #[error("traffic thresholds invalid: {0}")]
    TrafficThresholdsInvalid(String),
    /// `day_steps` is outside the spec range (1..=1024).
    #[error("day_steps {steps} out of range [1, 1024]")]
    DayStepsOutOfRange {
        /// Supplied step count.
        steps: u32,
    },
    /// `day_seconds` is outside the spec range (1..=3600).
    #[error("day_seconds {secs} out of range [1, 3600]")]
    DaySecondsOutOfRange {
        /// Supplied seconds.
        secs: u32,
    },
    /// Wraps the underlying [`CoreError`] (e.g. for [`MapSize`] /
    /// [`DaySteps`] / [`DaySeconds`] sub-validators).
    #[error(transparent)]
    Core(#[from] CoreError),
}

// =============================================================================
// GameModel
// =============================================================================

/// Canonical static configuration of a single match.
///
/// `GameModel` collects every spec-bound static parameter the contest
/// server publishes via `GET /setting`. It is intentionally
/// side-effect-free and serialisable so it can be persisted alongside
/// match telemetry and replayed deterministically.
///
/// ## Invariants (enforced by [`GameModel::validate`])
///
/// - `8 ≤ map_size.width, map_size.height ≤ 32`
/// - `1 ≤ turn_count ≤ 365`
/// - `1 ≤ day_steps.get() ≤ 1024`
/// - `1 ≤ day_seconds.get() ≤ 3600`
/// - `3 ≤ agent_count ≤ 8`
/// - `spot_count ≤ 32`
/// - `patrol_fuel_limit ≤ day_steps * 3`
/// - `1 ≤ busy_threshold ≤ 5`, `2 ≤ jammed_threshold ≤ 10`,
///   `jammed_threshold > busy_threshold`
///
/// See the module-level rustdoc for the spec trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameModel {
    /// Grid size, each side ∈ `[8, 32]` (Q&A Q2 / 募集要項 §マップ).
    pub map_size: MapSize,
    /// Number of days (turns) in the match. Matches always run for
    /// exactly this many days; there is no early termination.
    pub turn_count: u32,
    /// Move-step budget per day (Q&A Q6 / 募集要項 §行動).
    pub day_steps: DaySteps,
    /// Wall-clock seconds available per day.
    pub day_seconds: DaySeconds,
    /// Number of agents on the team (募集要項 §エージェント: 3..=8).
    pub agent_count: u8,
    /// Number of spots on the map (Q&A Q15: ≤ 32).
    pub spot_count: u32,
    /// Patrol fuel cap (Q&A Q14: ≤ `day_steps * 3`).
    pub patrol_fuel_limit: u32,
    /// 混雑 (busy) threshold (Q&A Q13: 1..=5).
    pub busy_threshold: u8,
    /// 渋滞 (jammed) threshold (Q&A Q13: 2..=10).
    pub jammed_threshold: u8,
}

impl GameModel {
    // -- Spec-bound constants (re-exposed for ergonomic call sites) --

    /// Minimum days per match. Matches must always run at least one day.
    pub const TURN_COUNT_MIN: u32 = 1;
    /// Maximum days per match (loose upper bound; typical matches use 3).
    pub const TURN_COUNT_MAX: u32 = 365;

    /// Minimum agent count per team (募集要項 §エージェント).
    pub const AGENT_COUNT_MIN: u8 = 3;
    /// Maximum agent count per team (募集要項 §エージェント).
    pub const AGENT_COUNT_MAX: u8 = 8;

    /// Maximum spot count per map (Q&A Q15).
    pub const SPOT_COUNT_MAX: u32 = 32;

    // -----------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------

    /// Build a canonical, mid-range default [`GameModel`] useful for
    /// tests, fixtures, and smoke tests.
    ///
    /// The defaults are deliberately in the middle of every legal range
    /// so that any individual value can be tweaked without immediately
    /// tripping the validator.
    #[must_use]
    pub fn canonical() -> Self {
        let day_steps = DaySteps::new(64).expect("64 is in [MIN, MAX]");
        let day_seconds = DaySeconds::new(60).expect("60 is in [MIN, MAX]");
        let map_size = MapSize::new(16, 16).expect("16x16 is in range");
        Self {
            map_size,
            turn_count: 3,
            day_steps,
            day_seconds,
            agent_count: 4,
            spot_count: 4,
            patrol_fuel_limit: day_steps.get().saturating_mul(3),
            busy_threshold: 1,
            jammed_threshold: 2,
        }
    }

    /// Construct a [`GameModel`] from raw values, validating every
    /// spec-bound invariant up-front. Prefer this constructor when
    /// building from untrusted input (network / JSON); the
    /// `pub`-field constructor is suitable when the values are
    /// already known to be valid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        map_size: MapSize,
        turn_count: u32,
        day_steps: DaySteps,
        day_seconds: DaySeconds,
        agent_count: u8,
        spot_count: u32,
        patrol_fuel_limit: u32,
        busy_threshold: u8,
        jammed_threshold: u8,
    ) -> Result<Self, GameModelError> {
        let m = Self {
            map_size,
            turn_count,
            day_steps,
            day_seconds,
            agent_count,
            spot_count,
            patrol_fuel_limit,
            busy_threshold,
            jammed_threshold,
        };
        m.validate()?;
        Ok(m)
    }

    // -----------------------------------------------------------------
    // Validation
    // -----------------------------------------------------------------

    /// Validate every spec-bound invariant of this model.
    ///
    /// The check is idempotent and cheap (no allocation, no I/O), so
    /// callers are encouraged to call it after every deserialisation
    /// and at the boundary between untrusted input and internal
    /// state.
    ///
    /// Returns the **first** violation found; the validator short-
    /// circuits on failure. The order of checks is stable across
    /// versions so tests can pin a specific variant.
    pub fn validate(&self) -> Result<(), GameModelError> {
        // -- Map size --
        MapSize::validate(self.map_size.width, self.map_size.height)?;

        // -- Turn count --
        if !(Self::TURN_COUNT_MIN..=Self::TURN_COUNT_MAX).contains(&self.turn_count) {
            return Err(GameModelError::InvalidTurnCount { turns: self.turn_count });
        }

        // -- Day steps / seconds (re-validate after deserialisation) --
        DaySteps::validate(self.day_steps.get())?;
        DaySeconds::validate(self.day_seconds.get())?;

        // -- Agent count --
        if !(Self::AGENT_COUNT_MIN..=Self::AGENT_COUNT_MAX).contains(&self.agent_count) {
            return Err(GameModelError::AgentCountOutOfRange { count: self.agent_count });
        }

        // -- Spot count (upper bound only — 0 spots is legal) --
        if self.spot_count > Self::SPOT_COUNT_MAX {
            return Err(GameModelError::SpotCountOutOfRange {
                count: self.spot_count,
                max: Self::SPOT_COUNT_MAX,
            });
        }

        // -- Patrol fuel cap (Q14) --
        let cap = self.day_steps.get().saturating_mul(3);
        if self.patrol_fuel_limit > cap {
            return Err(GameModelError::FuelLimitExceedsCap { fuel: self.patrol_fuel_limit, cap });
        }

        // -- Traffic thresholds (Q13) --
        validate_traffic_thresholds(self.busy_threshold, self.jammed_threshold)
            .map_err(|e| GameModelError::TrafficThresholdsInvalid(e.to_string()))?;

        Ok(())
    }

    // -----------------------------------------------------------------
    // Derived helpers
    // -----------------------------------------------------------------

    /// Total number of cells on the map (`width * height`).
    #[must_use]
    pub fn cell_count(&self) -> u32 {
        self.map_size.cell_count()
    }

    /// Per-day Patrol fuel cap, derived from `day_steps` (Q&A Q14).
    #[must_use]
    pub fn derived_patrol_fuel_cap(&self) -> u32 {
        self.day_steps.get().saturating_mul(3)
    }

    /// `true` iff the team is allowed at least one Supply agent.
    ///
    /// The contest rules do not require any Supply agent, but teams
    /// with no Supply cannot refuel Patrol agents mid-match.
    #[must_use]
    pub fn supports_supply_refuel(&self) -> bool {
        self.agent_count >= 2
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn model_with_agent_count(count: u8) -> GameModel {
        let mut m = GameModel::canonical();
        m.agent_count = count;
        m
    }

    // -- canonical --

    #[test]
    fn canonical_validates() {
        let m = GameModel::canonical();
        assert!(m.validate().is_ok(), "canonical defaults must validate");
    }

    #[test]
    fn canonical_has_in_range_thresholds() {
        let m = GameModel::canonical();
        assert_eq!(m.busy_threshold, 1);
        assert_eq!(m.jammed_threshold, 2);
        assert!(m.jammed_threshold > m.busy_threshold);
    }

    #[test]
    fn canonical_patrol_fuel_matches_derived_cap() {
        let m = GameModel::canonical();
        assert_eq!(m.patrol_fuel_limit, m.derived_patrol_fuel_cap());
    }

    // -- turn count --

    #[test]
    fn turn_count_zero_is_rejected() {
        let mut m = GameModel::canonical();
        m.turn_count = 0;
        assert!(matches!(m.validate(), Err(GameModelError::InvalidTurnCount { turns: 0 })));
    }

    #[test]
    fn turn_count_max_accepted() {
        let mut m = GameModel::canonical();
        m.turn_count = GameModel::TURN_COUNT_MAX;
        assert!(m.validate().is_ok());
    }

    // -- agent count boundaries --

    #[test]
    fn agent_count_min_accepted() {
        let m = model_with_agent_count(GameModel::AGENT_COUNT_MIN);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn agent_count_max_accepted() {
        let m = model_with_agent_count(GameModel::AGENT_COUNT_MAX);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn agent_count_below_min_rejected() {
        let m = model_with_agent_count(GameModel::AGENT_COUNT_MIN - 1);
        assert!(matches!(m.validate(), Err(GameModelError::AgentCountOutOfRange { .. })));
    }

    #[test]
    fn agent_count_above_max_rejected() {
        let m = model_with_agent_count(GameModel::AGENT_COUNT_MAX + 1);
        assert!(matches!(m.validate(), Err(GameModelError::AgentCountOutOfRange { .. })));
    }

    // -- spot count --

    #[test]
    fn spot_count_max_accepted() {
        let mut m = GameModel::canonical();
        m.spot_count = GameModel::SPOT_COUNT_MAX;
        assert!(m.validate().is_ok());
    }

    #[test]
    fn spot_count_above_max_rejected() {
        let mut m = GameModel::canonical();
        m.spot_count = GameModel::SPOT_COUNT_MAX + 1;
        assert!(matches!(m.validate(), Err(GameModelError::SpotCountOutOfRange { .. })));
    }

    #[test]
    fn spot_count_zero_accepted() {
        let mut m = GameModel::canonical();
        m.spot_count = 0;
        assert!(m.validate().is_ok(), "0 spots is legal (no spots -> 0 score)");
    }

    // -- fuel cap --

    #[test]
    fn fuel_cap_at_q14_boundary_accepted() {
        let mut m = GameModel::canonical();
        m.patrol_fuel_limit = m.derived_patrol_fuel_cap();
        assert!(m.validate().is_ok());
    }

    #[test]
    fn fuel_cap_above_q14_boundary_rejected() {
        let mut m = GameModel::canonical();
        m.patrol_fuel_limit = m.derived_patrol_fuel_cap() + 1;
        assert!(matches!(m.validate(), Err(GameModelError::FuelLimitExceedsCap { .. })));
    }

    // -- traffic thresholds --

    #[test]
    fn traffic_thresholds_q13_upper_accepted() {
        let mut m = GameModel::canonical();
        m.busy_threshold = 5;
        m.jammed_threshold = 10;
        assert!(m.validate().is_ok());
    }

    #[test]
    fn traffic_thresholds_busy_above_q13_rejected() {
        let mut m = GameModel::canonical();
        m.busy_threshold = 6;
        m.jammed_threshold = 10;
        assert!(matches!(m.validate(), Err(GameModelError::TrafficThresholdsInvalid(_))));
    }

    #[test]
    fn traffic_thresholds_jammed_not_above_busy_rejected() {
        let mut m = GameModel::canonical();
        m.busy_threshold = 5;
        m.jammed_threshold = 5;
        assert!(matches!(m.validate(), Err(GameModelError::TrafficThresholdsInvalid(_))));
    }

    // -- map size --

    #[test]
    fn map_size_below_min_rejected() {
        // MapSize::new validates up-front, so to exercise GameModel's
        // own re-validation path we must bypass the constructor and
        // assign a known-bad value directly. validate() must then
        // reject it via the Core error path.
        let mut m = GameModel::canonical();
        m.map_size = MapSize { width: 7, height: 16 };
        assert!(m.validate().is_err());
    }

    #[test]
    fn map_size_above_max_rejected() {
        let mut m = GameModel::canonical();
        // Directly assign a known-bad size; MapSize's own constructor
        // would have caught this, so we go through field assignment
        // for the unit test of GameModel::validate.
        m.map_size = MapSize { width: 33, height: 16 };
        assert!(m.validate().is_err());
    }

    // -- serde round-trip --

    #[test]
    fn serde_round_trip_preserves_all_fields() {
        let m = GameModel::canonical();
        let json = serde_json::to_string(&m).expect("serialize");
        let back: GameModel = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(m, back);
    }

    #[test]
    fn new_with_invalid_inputs_returns_error() {
        // agent_count below MIN must fail at construction time.
        let map = MapSize::new(16, 16).unwrap();
        let day_steps = DaySteps::new(64).unwrap();
        let day_seconds = DaySeconds::new(60).unwrap();
        let err = GameModel::new(map, 3, day_steps, day_seconds, 0, 4, 64, 1, 2)
            .expect_err("0 agents must fail");
        assert!(matches!(err, GameModelError::AgentCountOutOfRange { count: 0 }));
    }

    #[test]
    fn new_with_valid_inputs_returns_model() {
        let map = MapSize::new(16, 16).unwrap();
        let day_steps = DaySteps::new(64).unwrap();
        let day_seconds = DaySeconds::new(60).unwrap();
        let m = GameModel::new(map, 3, day_steps, day_seconds, 4, 4, 192, 1, 2).expect("all valid");
        assert_eq!(m.agent_count, 4);
    }
}
