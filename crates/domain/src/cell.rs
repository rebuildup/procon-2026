//! Terrain / Spot / Traffic primitives for the procon-2026 game.
//!
//! Spec sources (see `docs/pdf/` for the original PDFs):
//!
//! - 募集要項 表1 「地形タイプ」「スポット」「道路セルと交通量」
//! - Q&A Q11 (燃量 = 燃料消費量)
//! - Q&A Q13 (混雑閾値 1-5, 渋滞閾値 2-10)
//! - Q&A Q15 (スポット数)
//! - Q&A Q18 (1セル1スポット)
//!
//! ## Design note: Spot is parallel to Cell, not a variant on Cell
//!
//! `Cell` stays a flat 4-variant enum (`Plain`, `Mountain`, `Pond`, `Road`)
//! because (a) the move/fuel cost table is easier to maintain without
//! per-spot variants, and (b) Q&A Q18 ("1セル1スポット") means the
//! `Spot` lives *next to* a Cell at a coordinate, not *inside* the
//! terrain discriminator. The same parallel arrangement is documented
//! in ADR-0004 (`docs/adr/0004-cell-spot-flat-enum.md`).

use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreResult};

// =============================================================================
// Terrain cells
// =============================================================================

/// A single map cell. The variant determines movement cost and (where
/// applicable) fuel consumption.
///
/// `Road` carries a [`TrafficState`] because its move-step cost is
/// state-dependent; its fuel cost is constant across all three states.
///
/// Q&A Q18 guarantees that a spot (when present) is 1-per-cell, so the
/// terrain here intentionally does *not* embed any spot payload — see
/// [`Spot`] for the parallel type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cell {
    /// 平地 (Plain). 2 move-steps, 1 fuel per step.
    Plain,
    /// 山地 (Mountain). 3 move-steps, 2 fuel per step.
    Mountain,
    /// 池 (Pond). Impassable; both costs return [`u8::MAX`].
    Pond,
    /// 道路 (Road). Move-step cost depends on `TrafficState`; fuel cost is
    /// always 2 (Q&A Q11).
    Road(TrafficState),
}

/// Traffic congestion state of a [`Cell::Road`].
///
/// The actual transition is `volume >= JAMMED_THRESHOLD -> Jammed`, otherwise
/// `volume >= BUSY_THRESHOLD -> Busy`, otherwise `Smooth`. The thresholds are
/// documented in Q&A Q13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrafficState {
    /// 順調 (Smooth). Lowest move-step cost on a road.
    Smooth,
    /// 混雑 (Busy). Mid move-step cost.
    Busy,
    /// 渋滞 (Jammed). Highest move-step cost.
    Jammed,
}

/// Default move-step cost table per cell variant.
///
/// Pond is impassable and uses [`u8::MAX`] as a sentinel; callers should
/// check [`Cell::is_passable`] first rather than comparing to this value.
///
/// Spec: 募集要項 表1 + Q&A Q11.
pub const MOVE_STEPS: &[(Cell, u8)] = &[
    (Cell::Plain, 2),
    (Cell::Mountain, 3),
    (Cell::Pond, u8::MAX),
    (Cell::Road(TrafficState::Smooth), 1),
    (Cell::Road(TrafficState::Busy), 2),
    (Cell::Road(TrafficState::Jammed), 4),
];

/// Default fuel cost table per cell variant.
///
/// Pond is impassable (sentinel [`u8::MAX`]). All three traffic states of a
/// road share the same fuel cost (Q&A Q11).
pub const FUEL_COSTS: &[(Cell, u8)] = &[
    (Cell::Plain, 1),
    (Cell::Mountain, 2),
    (Cell::Pond, u8::MAX),
    (Cell::Road(TrafficState::Smooth), 2),
    (Cell::Road(TrafficState::Busy), 2),
    (Cell::Road(TrafficState::Jammed), 2),
];

/// Lower bound of the 混雑 (busy) threshold range from Q&A Q13.
pub const BUSY_THRESHOLD_MIN: u8 = 1;
/// Upper bound of the 混雑 (busy) threshold range from Q&A Q13.
pub const BUSY_THRESHOLD_MAX: u8 = 5;
/// Lower bound of the 渋滞 (jammed) threshold range from Q&A Q13.
pub const JAMMED_THRESHOLD_MIN: u8 = 2;
/// Upper bound of the 渋滞 (jammed) threshold range from Q&A Q13.
pub const JAMMED_THRESHOLD_MAX: u8 = 10;

/// Canonical mid-point thresholds chosen by the contest ruleset. The match
/// configuration may override these within the Q13 ranges (see
/// [`validate_traffic_thresholds`]).
pub const BUSY_THRESHOLD_DEFAULT: u8 = 1;
pub const JAMMED_THRESHOLD_DEFAULT: u8 = 2;

impl Cell {
    /// Move-steps required to traverse this cell.
    ///
    /// Returns [`u8::MAX`] for [`Cell::Pond`]; callers should check
    /// [`Cell::is_passable`] before scheduling movement.
    #[must_use]
    pub fn move_steps(self) -> u8 {
        match self {
            Cell::Mountain => 3,
            Cell::Pond => u8::MAX,
            Cell::Road(TrafficState::Smooth) => 1,
            Cell::Road(TrafficState::Jammed) => 4,
            Cell::Plain | Cell::Road(TrafficState::Busy) => 2,
        }
    }

    /// Fuel consumed per move-step on this cell.
    ///
    /// Returns [`u8::MAX`] for [`Cell::Pond`]; same caveat as
    /// [`Cell::move_steps`].
    #[must_use]
    pub fn fuel_cost(self) -> u8 {
        match self {
            Cell::Plain => 1,
            Cell::Pond => u8::MAX,
            Cell::Mountain | Cell::Road(_) => 2,
        }
    }

    /// Whether an agent can ever stand on or move through this cell.
    #[must_use]
    pub fn is_passable(self) -> bool {
        !matches!(self, Cell::Pond)
    }
}

impl TrafficState {
    /// Classify a traffic volume into a [`TrafficState`].
    ///
    /// Boundaries:
    /// - `volume >= jammed_threshold` → [`TrafficState::Jammed`]
    /// - `volume >= busy_threshold` → [`TrafficState::Busy`]
    /// - otherwise → [`TrafficState::Smooth`]
    ///
    /// Spec: Q&A Q13 (busy 1-5, jammed 2-10).
    #[must_use]
    pub fn from_volume(volume: u8, busy_threshold: u8, jammed_threshold: u8) -> Self {
        if volume >= jammed_threshold {
            TrafficState::Jammed
        } else if volume >= busy_threshold {
            TrafficState::Busy
        } else {
            TrafficState::Smooth
        }
    }
}

/// Validate the traffic thresholds against Q&A Q13 ranges.
///
/// Returns [`CoreError::InvariantViolation`] if either threshold falls
/// outside the documented range or if the jammed threshold is not
/// strictly greater than the busy threshold (so the state ordering is
/// non-degenerate).
pub fn validate_traffic_thresholds(busy: u8, jammed: u8) -> CoreResult<()> {
    if !(BUSY_THRESHOLD_MIN..=BUSY_THRESHOLD_MAX).contains(&busy) {
        return Err(CoreError::InvariantViolation(format!(
            "busy threshold {busy} out of Q13 range [{BUSY_THRESHOLD_MIN}, {BUSY_THRESHOLD_MAX}]"
        )));
    }
    if !(JAMMED_THRESHOLD_MIN..=JAMMED_THRESHOLD_MAX).contains(&jammed) {
        return Err(CoreError::InvariantViolation(format!(
            "jammed threshold {jammed} out of Q13 range \
             [{JAMMED_THRESHOLD_MIN}, {JAMMED_THRESHOLD_MAX}]"
        )));
    }
    if jammed <= busy {
        return Err(CoreError::InvariantViolation(format!(
            "jammed threshold {jammed} must be strictly greater than busy threshold {busy}"
        )));
    }
    Ok(())
}

// =============================================================================
// Spots and series
// =============================================================================

/// A うどん系列 (udon series) identifier.
///
/// Stored as a `u16` because the documented upper bound on the number of
/// distinct series per match is small (Q&A Q15: スポット数 ≤ 32; series ≤
/// spots), but using `u16` leaves headroom and keeps the wire format
/// stable across match configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Series(pub u16);

impl Series {
    /// Construct a new series identifier.
    #[must_use]
    pub fn new(value: u16) -> Self {
        Self(value)
    }

    /// Inner value.
    #[must_use]
    pub fn as_u16(self) -> u16 {
        self.0
    }
}

/// A スポット (spot) that dispenses one 玉 (ball) per agent per day.
///
/// Spec references:
/// - 募集要項「スポット」節
/// - Q&A Q15 (スポット数の上下限)
/// - Q&A Q18 (1セル1スポット)
///
/// `max_stock` is the per-day replenishment capacity (1 per agent in the
/// team). Construction validates `1 <= max_stock <= agent_count`; see
/// [`Spot::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Spot {
    /// Which series this spot yields.
    pub series: Series,
    /// Per-day stock capacity. Must satisfy `1 <= max_stock <= agent_count`.
    pub max_stock: u8,
}

impl Spot {
    /// Construct a spot, validating that `1 <= max_stock <= agent_count`.
    ///
    /// Returns [`CoreError::InvariantViolation`] if the bound is violated.
    pub fn new(series: Series, max_stock: u8, agent_count: u8) -> CoreResult<Self> {
        if agent_count == 0 {
            return Err(CoreError::InvariantViolation(
                "agent_count must be >= 1 to validate spot stock".into(),
            ));
        }
        if max_stock < 1 {
            return Err(CoreError::InvariantViolation(format!(
                "spot max_stock {max_stock} must be >= 1"
            )));
        }
        if max_stock > agent_count {
            return Err(CoreError::InvariantViolation(format!(
                "spot max_stock {max_stock} exceeds agent_count {agent_count}"
            )));
        }
        Ok(Self { series, max_stock })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    // -------- Cell: cost table --------

    #[test]
    fn plain_cell_uses_table_one_costs() {
        assert_eq!(Cell::Plain.move_steps(), 2);
        assert_eq!(Cell::Plain.fuel_cost(), 1);
        assert!(Cell::Plain.is_passable());
    }

    #[test]
    fn mountain_cell_uses_table_one_costs() {
        assert_eq!(Cell::Mountain.move_steps(), 3);
        assert_eq!(Cell::Mountain.fuel_cost(), 2);
        assert!(Cell::Mountain.is_passable());
    }

    #[test]
    fn pond_is_impassable_and_returns_sentinel_cost() {
        assert!(!Cell::Pond.is_passable());
        assert_eq!(Cell::Pond.move_steps(), u8::MAX);
        assert_eq!(Cell::Pond.fuel_cost(), u8::MAX);
    }

    #[test]
    fn road_step_cost_depends_on_traffic_state() {
        // Q&A Q11: fuel cost is 2 for all traffic states.
        for state in [TrafficState::Smooth, TrafficState::Busy, TrafficState::Jammed] {
            assert_eq!(Cell::Road(state).fuel_cost(), 2);
            assert!(Cell::Road(state).is_passable());
        }
        assert_eq!(Cell::Road(TrafficState::Smooth).move_steps(), 1);
        assert_eq!(Cell::Road(TrafficState::Busy).move_steps(), 2);
        assert_eq!(Cell::Road(TrafficState::Jammed).move_steps(), 4);
    }

    #[test]
    fn const_tables_cover_every_variant() {
        // The const tables are spec artefacts: every variant must appear
        // exactly once. This guards against accidental drift if a new
        // variant is added without updating the tables.
        for (cell, steps) in MOVE_STEPS {
            assert_eq!(cell.move_steps(), *steps, "MOVE_STEPS table out of sync for {cell:?}");
        }
        for (cell, fuel) in FUEL_COSTS {
            assert_eq!(cell.fuel_cost(), *fuel, "FUEL_COSTS table out of sync for {cell:?}");
        }
    }

    // -------- TrafficState --------

    #[test]
    fn traffic_state_thresholds_default_classification() {
        // Defaults from Q13 lower bounds.
        assert_eq!(
            TrafficState::from_volume(0, BUSY_THRESHOLD_DEFAULT, JAMMED_THRESHOLD_DEFAULT),
            TrafficState::Smooth
        );
        assert_eq!(
            TrafficState::from_volume(1, BUSY_THRESHOLD_DEFAULT, JAMMED_THRESHOLD_DEFAULT),
            TrafficState::Busy
        );
        assert_eq!(
            TrafficState::from_volume(2, BUSY_THRESHOLD_DEFAULT, JAMMED_THRESHOLD_DEFAULT),
            TrafficState::Jammed
        );
    }

    #[test]
    fn traffic_state_thresholds_upper_bound_classification() {
        // Upper bounds from Q13 (busy=5, jammed=10). Volume = 9 must still be Busy.
        assert_eq!(TrafficState::from_volume(9, 5, 10), TrafficState::Busy);
        assert_eq!(TrafficState::from_volume(10, 5, 10), TrafficState::Jammed);
    }

    #[test]
    fn validate_traffic_thresholds_accepts_lower_bounds() {
        assert!(validate_traffic_thresholds(BUSY_THRESHOLD_MIN, JAMMED_THRESHOLD_MIN).is_ok());
    }

    #[test]
    fn validate_traffic_thresholds_accepts_upper_bounds() {
        assert!(validate_traffic_thresholds(BUSY_THRESHOLD_MAX, JAMMED_THRESHOLD_MAX).is_ok());
    }

    #[test]
    fn validate_traffic_thresholds_rejects_busy_below_q13_range() {
        assert!(validate_traffic_thresholds(0, JAMMED_THRESHOLD_DEFAULT).is_err());
    }

    #[test]
    fn validate_traffic_thresholds_rejects_busy_above_q13_range() {
        assert!(validate_traffic_thresholds(BUSY_THRESHOLD_MAX + 1, JAMMED_THRESHOLD_MAX).is_err());
    }

    #[test]
    fn validate_traffic_thresholds_rejects_jammed_below_q13_range() {
        assert!(validate_traffic_thresholds(BUSY_THRESHOLD_DEFAULT, 1).is_err());
    }

    #[test]
    fn validate_traffic_thresholds_rejects_jammed_above_q13_range() {
        assert!(
            validate_traffic_thresholds(BUSY_THRESHOLD_DEFAULT, JAMMED_THRESHOLD_MAX + 1).is_err()
        );
    }

    #[test]
    fn validate_traffic_thresholds_rejects_inverted_ordering() {
        // jammed == busy: degenerate state ordering.
        assert!(validate_traffic_thresholds(3, 3).is_err());
        // jammed < busy: would make Busy/Jammed collapse.
        assert!(validate_traffic_thresholds(5, 2).is_err());
    }

    // -------- Series --------

    #[test]
    fn series_newtype_round_trip() {
        let s = Series::new(7);
        assert_eq!(s.as_u16(), 7);
        assert_eq!(s.0, 7);
    }

    #[test]
    fn series_is_comparable_and_hashable() {
        let a = Series::new(1);
        let b = Series::new(2);
        let c = Series::new(1);
        assert!(a < b);
        assert_eq!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        set.insert(b);
        set.insert(c);
        assert_eq!(set.len(), 2);
    }

    // -------- Spot --------

    #[test]
    fn spot_accepts_lower_bound_max_stock() {
        // 1 <= max_stock <= agent_count with agent_count = 4.
        let s = Spot::new(Series::new(0), 1, 4).expect("max_stock=1 should validate");
        assert_eq!(s.max_stock, 1);
        assert_eq!(s.series, Series::new(0));
    }

    #[test]
    fn spot_accepts_upper_bound_max_stock() {
        // max_stock == agent_count is the documented upper bound.
        let s = Spot::new(Series::new(3), 8, 8).expect("max_stock == agent_count should validate");
        assert_eq!(s.max_stock, 8);
    }

    #[test]
    fn spot_rejects_zero_max_stock() {
        let err = Spot::new(Series::new(0), 0, 4).expect_err("max_stock=0 must fail");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn spot_rejects_max_stock_above_agent_count() {
        // 9 > 8 agent_count.
        let err = Spot::new(Series::new(0), 9, 8).expect_err("max_stock > agent_count must fail");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn spot_rejects_when_agent_count_is_zero() {
        // Defensive: validating against a zero agent count must fail loudly.
        let err = Spot::new(Series::new(0), 1, 0).expect_err("agent_count=0 must fail");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn spot_with_min_agent_count_accepts_max_stock_one() {
        // agent_count = 1 is the smallest team. Only max_stock=1 is valid.
        assert!(Spot::new(Series::new(0), 1, 1).is_ok());
        assert!(Spot::new(Series::new(0), 2, 1).is_err());
    }

    #[test]
    fn spot_serde_round_trip() {
        let original = Spot::new(Series::new(5), 3, 8).unwrap();
        let json = serde_json::to_string(&original).expect("serialize");
        let back: Spot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, back);
    }
}
