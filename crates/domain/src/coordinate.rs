//! Hex grid primitives for the "ヘキサうどん" contest map.
//!
//! ## Specification trace
//!
//! - **募集要項 §マップ / 競技部門「ヘキサうどん」のフォーマットについて.pdf**: the
//!   playable surface is a rectangular grid of hexagonal cells, with
//!   `width ∈ [8, 32]` and `height ∈ [8, 32]`.
//! - **第37回大会競技部門問い合わせと回答その１.pdf Q1**: even-numbered rows are
//!   fixed to the right-offset variant (commonly called *even-r* offset). This is
//!   the only layout the contest rules allow, so [`MapLayout`] is a single-variant
//!   enum with `EvenRowRightOffset` as the canonical choice (kept as an enum for
//!   future-compat with renderer / variant research).
//! - **Q2**: the map is **not** a torus. Coordinates outside `MapSize` are invalid
//!   and must not wrap. [`Bounds::clamp`] saturates to the last in-range cell
//!   and [`Bounds::check`] returns `None` for out-of-range coordinates.
//!   See `docs/adr/0004-hex-coord-and-non-torus.md`.
//!
//! ## Scope (Issue #11)
//!
//! In: type definitions, validation, spec-trace documentation.
//! Out: path-finding algorithms, render logic, neighbor/reachability computation
//! (deferred to a follow-up issue — the [`HexCoord`] layout is intentionally
//! minimal here so the rest of the engine can compile against stable names).

use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreResult};

/// Width / height of the playable hex grid.
///
/// Cell count `W * H` lives in `[64, 1024]`. Values are stored as `u8` because
/// the contest caps both dimensions at 32 (see [`MapSize::MAX`]).
///
/// Spec: 募集要項 §マップ — "W, H ∈ [8, 32]".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MapSize {
    /// Number of columns (0-indexed, inclusive).
    pub width: u8,
    /// Number of rows (0-indexed, inclusive).
    pub height: u8,
}

impl MapSize {
    /// Minimum allowed side length (contest rule).
    pub const MIN: u8 = 8;
    /// Maximum allowed side length (contest rule).
    pub const MAX: u8 = 32;

    /// Construct a [`MapSize`] from raw side lengths after validating the
    /// contest bounds `8 <= W, H <= 32`.
    ///
    /// Returns [`CoreError::InvariantViolation`] if either dimension is out of
    /// range.
    pub fn new(width: u8, height: u8) -> CoreResult<Self> {
        Self::validate(width, height)?;
        Ok(Self { width, height })
    }

    /// Validate the contest size bounds without allocating the value.
    pub fn validate(width: u8, height: u8) -> CoreResult<()> {
        let bad = !Self::in_range(width) || !Self::in_range(height);
        if bad {
            return Err(CoreError::InvariantViolation(format!(
                "map size {width}x{height} out of range [{}, {}]",
                Self::MIN,
                Self::MAX
            )));
        }
        Ok(())
    }

    /// Pure boolean range check suitable for `const fn` / hot-path callers.
    #[must_use]
    pub const fn in_range(dim: u8) -> bool {
        matches!(dim, Self::MIN..=Self::MAX)
    }

    /// Total cell count (`width * height`). Saturates at `u8::MAX` even though
    /// the validated maximum is `32 * 32 = 1024`.
    #[must_use]
    pub fn cell_count(self) -> u32 {
        u32::from(self.width) * u32::from(self.height)
    }
}

/// Hex cell coordinate in the contest's offset coordinate system.
///
/// `(col, row)` address space: `col ∈ [0, width)`, `row ∈ [0, height)`.
/// The actual geometric position depends on [`MapLayout`]; this type only
/// encodes the (col, row) address.
///
/// Spec: 募集要項 §マップ / Q1 (偶数行右ずれ固定).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexCoord {
    /// Column index, 0-indexed.
    pub col: u8,
    /// Row index, 0-indexed.
    pub row: u8,
}

impl HexCoord {
    /// Construct a coordinate from raw `(col, row)` without bounds-checking.
    /// Use [`Bounds::check`] or [`Bounds::clamp`] when range is uncertain.
    #[must_use]
    pub const fn new(col: u8, row: u8) -> Self {
        Self { col, row }
    }

    /// Zero-origin coordinate `(0, 0)`.
    pub const ORIGIN: Self = Self::new(0, 0);
}

/// Layout variant for the offset coordinate system.
///
/// Q1 fixes the contest map to **even-row right-offset** (a.k.a. *even-r*):
/// cells whose `row` is even sit at their nominal `col`; cells whose `row` is
/// odd shift by half a cell-width to the right of the even row above them.
///
/// The enum is kept single-variant for the contest rules. New variants (e.g.
/// for renderer-only debug views) should be added behind an explicit feature
/// gate rather than changing the canonical layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MapLayout {
    /// Even-numbered rows sit at their nominal column; odd rows are offset to
    /// the right. This is the only layout the contest allows (Q1).
    #[default]
    EvenRowRightOffset,
}

/// Bounded view over a [`MapSize`] under a specific [`MapLayout`].
///
/// `Bounds` answers "is this coord on the map?" in two styles:
///
/// - [`Bounds::clamp`] — saturate an arbitrary `(col, row)` to the largest
///   in-range cell (useful for UI / drag handlers that must always render
///   *something*).
/// - [`Bounds::check`] — return `Some(coord)` if in range, `None` otherwise
///   (useful for game-logic that must reject out-of-range moves).
///
/// Spec: Q2 — the map is **not** a torus; out-of-range coords do not wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bounds {
    pub size: MapSize,
    pub layout: MapLayout,
}

impl Bounds {
    /// Build bounds over `size` under the given layout.
    #[must_use]
    pub const fn new(size: MapSize, layout: MapLayout) -> Self {
        Self { size, layout }
    }

    /// Returns `true` iff `coord` lies inside the map. The layout is irrelevant
    /// for the offset-coordinate system because `(col, row)` is always inside
    /// the axis-aligned rectangle; we accept `layout` as a parameter so the
    /// bounds remain correct when more exotic layouts land.
    #[must_use]
    pub const fn contains(self, coord: HexCoord) -> bool {
        coord.col < self.size.width && coord.row < self.size.height
    }

    /// Returns `Some(coord)` if in range, `None` otherwise. This is the
    /// preferred entry point for game-logic that must reject moves outside the
    /// map (Q2: the map is not a torus).
    #[must_use]
    pub const fn check(self, coord: HexCoord) -> Option<HexCoord> {
        if self.contains(coord) { Some(coord) } else { None }
    }

    /// Saturate `coord` to the largest in-range cell. Out-of-range dimensions
    /// collapse to `width - 1` / `height - 1` independently; negative overflow
    /// is impossible because coordinates are `u8`.
    #[must_use]
    pub const fn clamp(self, coord: HexCoord) -> HexCoord {
        HexCoord::new(
            if coord.col >= self.size.width { self.size.width - 1 } else { coord.col },
            if coord.row >= self.size.height { self.size.height - 1 } else { coord.row },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // -- MapSize::validate ------------------------------------------------

    #[test]
    fn map_size_validate_accepts_min_boundary() {
        assert!(MapSize::validate(8, 8).is_ok());
    }

    #[test]
    fn map_size_validate_accepts_max_boundary() {
        assert!(MapSize::validate(32, 32).is_ok());
    }

    #[test]
    fn map_size_validate_accepts_asymmetric() {
        assert!(MapSize::validate(8, 32).is_ok());
        assert!(MapSize::validate(32, 8).is_ok());
        assert!(MapSize::validate(15, 24).is_ok());
    }

    #[test]
    fn map_size_validate_rejects_below_min() {
        let err = MapSize::validate(7, 8).unwrap_err();
        assert!(matches!(err, CoreError::InvariantViolation(_)));
        assert!(MapSize::validate(8, 7).is_err());
        assert!(MapSize::validate(0, 8).is_err());
    }

    #[test]
    fn map_size_validate_rejects_above_max() {
        assert!(MapSize::validate(33, 32).is_err());
        assert!(MapSize::validate(32, 33).is_err());
        assert!(MapSize::validate(255, 32).is_err());
    }

    #[test]
    fn map_size_new_round_trips_validated_values() {
        let m = MapSize::new(12, 16).expect("12x16 is in range");
        assert_eq!(m.width, 12);
        assert_eq!(m.height, 16);
        assert_eq!(m.cell_count(), 192);
    }

    #[test]
    fn map_size_in_range_is_const_compatible() {
        const OK: bool = MapSize::in_range(20);
        const BAD: bool = !MapSize::in_range(0);
        assert!(OK);
        assert!(BAD);
    }

    // -- HexCoord ---------------------------------------------------------

    #[test]
    fn hex_coord_origin_is_zero() {
        assert_eq!(HexCoord::ORIGIN, HexCoord::new(0, 0));
    }

    #[test]
    fn hex_coord_eq_and_hash_distinguish_fields() {
        let a = HexCoord::new(1, 2);
        let b = HexCoord::new(1, 2);
        let c = HexCoord::new(2, 1);
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn hex_coord_is_copy() {
        let a = HexCoord::new(3, 5);
        let b = a; // Copy, not move.
        assert_eq!(a, b);
    }

    #[test]
    fn hex_coord_serde_round_trip() {
        let c = HexCoord::new(3, 5);
        let s = serde_json::to_string(&c).expect("serialize");
        assert_eq!(s, r#"{"col":3,"row":5}"#);
        let back: HexCoord = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(c, back);
    }

    #[test]
    fn map_size_serde_round_trip() {
        let m = MapSize::new(10, 12).expect("10x12 is in range");
        let s = serde_json::to_string(&m).expect("serialize");
        let back: MapSize = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(m, back);
    }

    // -- MapLayout --------------------------------------------------------

    #[test]
    fn map_layout_default_is_q1_fixed() {
        assert_eq!(MapLayout::default(), MapLayout::EvenRowRightOffset);
    }

    // -- Bounds ------------------------------------------------------------

    fn sample_bounds() -> Bounds {
        Bounds::new(MapSize::new(10, 12).expect("10x12 valid"), MapLayout::default())
    }

    #[test]
    fn bounds_contains_inside_rectangle() {
        let b = sample_bounds();
        assert!(b.contains(HexCoord::new(0, 0)));
        assert!(b.contains(HexCoord::new(9, 11)));
        assert!(b.contains(HexCoord::new(5, 5)));
    }

    #[test]
    fn bounds_contains_rejects_outside_rectangle() {
        let b = sample_bounds();
        assert!(!b.contains(HexCoord::new(10, 0)), "col == width is out");
        assert!(!b.contains(HexCoord::new(0, 12)), "row == height is out");
        assert!(!b.contains(HexCoord::new(255, 255)));
    }

    #[test]
    fn bounds_check_returns_some_inside() {
        let b = sample_bounds();
        assert_eq!(b.check(HexCoord::new(5, 5)), Some(HexCoord::new(5, 5)));
    }

    #[test]
    fn bounds_check_returns_none_outside() {
        let b = sample_bounds();
        assert_eq!(b.check(HexCoord::new(10, 5)), None);
        assert_eq!(b.check(HexCoord::new(5, 12)), None);
        assert_eq!(b.check(HexCoord::new(255, 255)), None);
    }

    #[test]
    fn bounds_clamp_passes_through_in_range() {
        let b = sample_bounds();
        assert_eq!(b.clamp(HexCoord::new(3, 5)), HexCoord::new(3, 5));
        assert_eq!(b.clamp(HexCoord::new(0, 0)), HexCoord::new(0, 0));
        assert_eq!(b.clamp(HexCoord::new(9, 11)), HexCoord::new(9, 11));
    }

    #[test]
    fn bounds_clamp_saturates_to_last_cell_per_axis() {
        let b = sample_bounds();
        assert_eq!(b.clamp(HexCoord::new(10, 0)), HexCoord::new(9, 0));
        assert_eq!(b.clamp(HexCoord::new(0, 12)), HexCoord::new(0, 11));
        assert_eq!(b.clamp(HexCoord::new(255, 255)), HexCoord::new(9, 11));
    }

    #[test]
    fn bounds_does_not_wrap_around_map() {
        // Q2: the map is not a torus; over-saturated inputs must NOT wrap to
        // the opposite side.
        let b = sample_bounds();
        assert_ne!(b.clamp(HexCoord::new(255, 0)), HexCoord::new(0, 0));
        assert_eq!(b.clamp(HexCoord::new(255, 0)), HexCoord::new(9, 0));
    }

    #[test]
    fn bounds_serde_round_trip() {
        let b = sample_bounds();
        let s = serde_json::to_string(&b).expect("serialize");
        let back: Bounds = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(b, back);
    }
}
