//! Action, Direction, and move-validation primitives.
//!
//! ## Specification trace
//!
//! - **募集要項「移動と燃料」**: agents traverse the hex grid one step at a
//!   time; each step costs fuel depending on the destination terrain (see
//!   [`crate::cell::Cell`] for the cost table).
//! - **Q&A Q1** (枝1): the map is **even-row right-offset** — even `row`
//!   values sit half a cell to the right of the odd `row` above. This is
//!   the only layout the contest allows, so [`Direction`] neighbor offsets
//!   branch on `row % 2`. `crates/engine`'s `Direction::step` is the
//!   authority: its offsets, not this module's, decide real matches.
//! - **Q&A Q6**: input answers are validated **before** they are reflected
//!   into the match state. A single invalid action rejects the **entire**
//!   answer batch, never just the offending action. [`Action::validate`]
//!   captures this contract at the action level; the batch layer is
//!   responsible for "all-or-nothing" propagation.
//! - **Direction wire order** (clockwise from upper-left):
//!   `0=UpperLeft, 1=UpperRight, 2=Right, 3=LowerRight, 4=LowerLeft, 5=Left`.
//!   Wait is encoded outside the enum as `direction <= -1` in the wire
//!   format (see [`ActionKind::from_wire`]).
//!
//! ## Scope (Issue #53)
//!
//! In: action data types, `move_to` boundary/terrain validation, wire-format
//! decoders, batch-level validation semantics.
//! Out: action generation (lives in solver crates), fuel accounting, agent
//! state lookup, terrain layout construction.

use serde::{Deserialize, Serialize};

use crate::coordinate::{Bounds, HexCoord, MapLayout};
use crate::{CoreError, CoreResult};

// =============================================================================
// Direction
// =============================================================================

/// Direction for a single move step.
///
/// Variants are declared in the wire-encoded order (clockwise from
/// upper-left) so `#[repr(i8)]` plus sequential `as i8` casts give the
/// spec values `0..=5`. Wait is **not** part of [`Direction`] — it lives
/// at the action level ([`ActionKind::Wait`]) and is encoded as `-1` in
/// the wire format.
///
/// Spec: 募集要項「移動と燃料」 / Direction 順序.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i8)]
pub enum Direction {
    /// 左上 (Upper-left). Wire: `0`.
    UpperLeft = 0,
    /// 右上 (Upper-right). Wire: `1`.
    UpperRight = 1,
    /// 右 (Right / East). Wire: `2`.
    Right = 2,
    /// 右下 (Lower-right). Wire: `3`.
    LowerRight = 3,
    /// 左下 (Lower-left). Wire: `4`.
    LowerLeft = 4,
    /// 左 (Left / West). Wire: `5`.
    Left = 5,
}

impl Direction {
    /// All six directions, in wire order (clockwise from upper-left).
    ///
    /// Useful for solver enumeration; matches the spec ordering exactly.
    pub const ALL: [Direction; 6] = [
        Direction::UpperLeft,
        Direction::UpperRight,
        Direction::Right,
        Direction::LowerRight,
        Direction::LowerLeft,
        Direction::Left,
    ];

    /// Wire-encoded integer for this direction (`0..=5`). Wait is `-1`
    /// and lives outside this enum.
    #[must_use]
    pub fn as_wire(self) -> i8 {
        self as i8
    }

    /// Decode a wire value into a [`Direction`]. Returns `None` for
    /// `-1` (Wait, not a direction) and any value outside `0..=5`.
    #[must_use]
    pub fn from_wire(value: i8) -> Option<Self> {
        match value {
            0 => Some(Direction::UpperLeft),
            1 => Some(Direction::UpperRight),
            2 => Some(Direction::Right),
            3 => Some(Direction::LowerRight),
            4 => Some(Direction::LowerLeft),
            5 => Some(Direction::Left),
            _ => None,
        }
    }
}

impl core::fmt::Display for Direction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let label = match self {
            Direction::UpperLeft => "upper-left",
            Direction::UpperRight => "upper-right",
            Direction::Right => "right",
            Direction::LowerRight => "lower-right",
            Direction::LowerLeft => "lower-left",
            Direction::Left => "left",
        };
        core::fmt::Display::fmt(label, f)
    }
}

// =============================================================================
// ActionKind
// =============================================================================

/// Action payload: Wait or Move in a [`Direction`].
///
/// Spec: 募集要項「移動と燃料」/「待機」.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionKind {
    /// 待機 (Wait). No movement, no fuel cost. Wire: `-1`.
    Wait,
    /// 移動 (Move) in the given direction. Wire: `0..=5`.
    Move(Direction),
}

impl ActionKind {
    /// Decode an action kind from the wire format.
    ///
    /// - `value < 0` → [`ActionKind::Wait`]
    /// - `value ∈ 0..=5` → [`ActionKind::Move`]
    /// - `value > 5` → `None` (invalid; will cause whole-batch rejection per Q6)
    #[must_use]
    pub fn from_wire(value: i8) -> Option<Self> {
        if value < 0 {
            Some(ActionKind::Wait)
        } else {
            Direction::from_wire(value).map(ActionKind::Move)
        }
    }

    /// Encode this action kind to the wire format.
    ///
    /// - [`ActionKind::Wait`] → `-1`
    /// - [`ActionKind::Move(d)`] → `d as i8` (`0..=5`)
    #[must_use]
    pub fn to_wire(self) -> i8 {
        match self {
            ActionKind::Wait => -1,
            ActionKind::Move(d) => d as i8,
        }
    }
}

impl core::fmt::Display for ActionKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ActionKind::Wait => core::fmt::Display::fmt("wait", f),
            ActionKind::Move(d) => core::fmt::Display::fmt(d, f),
        }
    }
}

// =============================================================================
// Action
// =============================================================================

/// An action issued by an agent for a single turn step.
///
/// Spec: 募集要項「行動」節 (行動 = 移動 or 待機).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Action {
    /// Stable agent identifier within a match.
    pub agent_id: u32,
    /// What the agent does this step.
    pub kind: ActionKind,
}

impl Action {
    /// Construct a new action.
    #[must_use]
    pub const fn new(agent_id: u32, kind: ActionKind) -> Self {
        Self { agent_id, kind }
    }

    /// Construct a Wait action for the given agent.
    #[must_use]
    pub const fn wait(agent_id: u32) -> Self {
        Self { agent_id, kind: ActionKind::Wait }
    }

    /// Construct a Move action for the given agent in `direction`.
    #[must_use]
    pub const fn move_in(agent_id: u32, direction: Direction) -> Self {
        Self { agent_id, kind: ActionKind::Move(direction) }
    }

    /// Validate this action from `pos` against the map bounds and a
    /// passability predicate.
    ///
    /// Returns:
    /// - `Ok(pos)` for [`ActionKind::Wait`] (no constraints to check)
    /// - `Ok(next)` if the move lands inside `bounds` on a passable cell
    /// - `Err(CoreError::IllegalMove)` if the move goes off-map or hits
    ///   impassable terrain (e.g. [`crate::cell::Cell::Pond`])
    ///
    /// Q&A Q6: this is the **per-action** primitive. The batch layer
    /// must call this for every action and reject the **entire** batch
    /// if **any** action fails.
    pub fn validate(
        self,
        pos: HexCoord,
        bounds: Bounds,
        is_passable: impl Fn(HexCoord) -> bool,
    ) -> CoreResult<HexCoord> {
        match self.kind {
            ActionKind::Wait => Ok(pos),
            ActionKind::Move(d) => move_to(pos, d, bounds, &is_passable).ok_or_else(|| {
                CoreError::IllegalMove(format!(
                    "agent {} cannot move {d} from ({}, {})",
                    self.agent_id, pos.col, pos.row
                ))
            }),
        }
    }
}

// =============================================================================
// Move validation
// =============================================================================

/// Compute the next coordinate by applying `dir` to `coord` under the
/// given `bounds`, rejecting off-map and impassable destinations.
///
/// Returns `None` if:
///
/// - the neighbor lies outside `bounds` (Q&A Q2: the map is not a torus,
///   no wrap-around), or
/// - `is_passable(next)` returns `false` (e.g. a [`crate::cell::Cell::Pond`]
///   at the destination).
///
/// The function does **not** check whether `coord` itself is passable —
/// call sites are expected to maintain that invariant.
#[must_use]
pub fn move_to(
    coord: HexCoord,
    dir: Direction,
    bounds: Bounds,
    is_passable: impl Fn(HexCoord) -> bool,
) -> Option<HexCoord> {
    let next = neighbor(coord, dir, bounds.layout)?;
    let bounded = bounds.check(next)?;
    if is_passable(bounded) { Some(bounded) } else { None }
}

/// Compute the neighbor coordinate for `dir` under the given layout, **without**
/// bounds or passability checks. Returns `None` only if the signed arithmetic
/// would produce a negative coordinate that cannot be represented as `u8`
/// (out-of-bounds for the underlying type).
///
/// Spec: Q&A Q1 (even-row right-offset). The neighbor offset depends on the
/// parity of `coord.row`.
#[must_use]
pub fn neighbor(coord: HexCoord, dir: Direction, layout: MapLayout) -> Option<HexCoord> {
    // The current MapLayout is single-variant (EvenRowRightOffset) but we
    // dispatch on it so future layouts don't silently use the wrong offsets.
    let _ = layout;
    let even_row = coord.row % 2 == 0;
    // Mirrors `engine::model::Direction::step` exactly: on an even (right-
    // shifted) row the upper-left / lower-left neighbour keeps the column and
    // the right-hand pair gains one; on an odd row it is the other way round.
    // The previous arms were that table's mirror image, i.e. odd-r.
    let (dr, dc) = match (even_row, dir) {
        (true, Direction::UpperLeft) => (-1i16, 0i16),
        (true, Direction::UpperRight) => (-1, 1),
        (false, Direction::UpperLeft) => (-1, -1),
        (false, Direction::UpperRight) => (-1, 0),
        (_, Direction::Right) => (0, 1),
        (_, Direction::Left) => (0, -1),
        (true, Direction::LowerRight) => (1, 1),
        (true, Direction::LowerLeft) => (1, 0),
        (false, Direction::LowerRight) => (1, 0),
        (false, Direction::LowerLeft) => (1, -1),
    };
    let new_row = i16::from(coord.row) + dr;
    let new_col = i16::from(coord.col) + dc;
    if new_row < 0 || new_col < 0 {
        return None;
    }
    // MapSize caps at 32 so the upper bound fits in u8; saturate to u8::MAX
    // is not needed because `bounds.check` performs the real upper check.
    Some(HexCoord::new(u8::try_from(new_col).ok()?, u8::try_from(new_row).ok()?))
}

// =============================================================================
// Batch validation (Q&A Q6 all-or-nothing)
// =============================================================================

/// Validate a batch of actions from their starting positions, rejecting
/// the **entire** batch if any action fails (Q&A Q6).
///
/// On success returns `Ok` with the resolved next-coordinate for each
/// action, in input order. On failure returns `Err` carrying the index
/// of the **first** offending action so callers can surface a useful
/// error message.
pub fn validate_batch(
    batch: &[(Action, HexCoord)],
    bounds: Bounds,
    is_passable: &dyn Fn(HexCoord) -> bool,
) -> CoreResult<Vec<HexCoord>> {
    let mut out = Vec::with_capacity(batch.len());
    for (i, (action, pos)) in batch.iter().enumerate() {
        match action.validate(*pos, bounds, is_passable) {
            Ok(next) => out.push(next),
            Err(e) => {
                return Err(CoreError::IllegalMove(format!(
                    "batch rejected at index {i} (Q&A Q6 all-or-nothing): {e}"
                )));
            }
        }
    }
    Ok(out)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bounds() -> Bounds {
        Bounds::new(
            crate::coordinate::MapSize::new(10, 10).expect("10x10 valid"),
            MapLayout::default(),
        )
    }

    // ---------- Direction: wire round-trip ----------

    #[test]
    fn direction_wire_order_matches_spec() {
        // Spec order: 0=UpperLeft, 1=UpperRight, 2=Right, 3=LowerRight,
        // 4=LowerLeft, 5=Left (clockwise from upper-left).
        assert_eq!(Direction::UpperLeft.as_wire(), 0);
        assert_eq!(Direction::UpperRight.as_wire(), 1);
        assert_eq!(Direction::Right.as_wire(), 2);
        assert_eq!(Direction::LowerRight.as_wire(), 3);
        assert_eq!(Direction::LowerLeft.as_wire(), 4);
        assert_eq!(Direction::Left.as_wire(), 5);
    }

    #[test]
    fn direction_from_wire_accepts_zero_through_five() {
        for (i, expected) in Direction::ALL.iter().enumerate() {
            let wire = i8::try_from(i).expect("ALL has at most 6 entries");
            assert_eq!(Direction::from_wire(wire), Some(*expected));
        }
    }

    #[test]
    fn direction_from_wire_rejects_wait_and_out_of_range() {
        // -1 means Wait (not a Direction) and >5 is invalid.
        assert_eq!(Direction::from_wire(-1), None);
        assert_eq!(Direction::from_wire(-2), None);
        assert_eq!(Direction::from_wire(6), None);
        assert_eq!(Direction::from_wire(127), None);
        assert_eq!(Direction::from_wire(-128), None);
    }

    #[test]
    fn direction_round_trip_through_wire() {
        for d in Direction::ALL {
            assert_eq!(Direction::from_wire(d.as_wire()), Some(d));
        }
    }

    // ---------- ActionKind: wire round-trip ----------

    #[test]
    fn action_kind_from_wire_wait_for_negative() {
        assert_eq!(ActionKind::from_wire(-1), Some(ActionKind::Wait));
        assert_eq!(ActionKind::from_wire(-100), Some(ActionKind::Wait));
        assert_eq!(ActionKind::from_wire(i8::MIN), Some(ActionKind::Wait));
    }

    #[test]
    fn action_kind_from_wire_move_for_zero_to_five() {
        for (i, d) in Direction::ALL.iter().enumerate() {
            let wire = i8::try_from(i).expect("ALL has at most 6 entries");
            assert_eq!(ActionKind::from_wire(wire), Some(ActionKind::Move(*d)));
        }
    }

    #[test]
    fn action_kind_from_wire_rejects_out_of_range_positive() {
        assert_eq!(ActionKind::from_wire(6), None);
        assert_eq!(ActionKind::from_wire(127), None);
    }

    #[test]
    fn action_kind_to_wire_round_trip() {
        assert_eq!(ActionKind::Wait.to_wire(), -1);
        for d in Direction::ALL {
            let k = ActionKind::Move(d);
            let w = k.to_wire();
            assert_eq!(ActionKind::from_wire(w), Some(k));
        }
    }

    // ---------- Action serde round-trip ----------

    #[test]
    fn action_serde_round_trip_wait() {
        let a = Action::wait(7);
        let s = serde_json::to_string(&a).expect("serialize");
        let back: Action = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn action_serde_round_trip_move_all_directions() {
        for d in Direction::ALL {
            let a = Action::move_in(3, d);
            let s = serde_json::to_string(&a).expect("serialize");
            let back: Action = serde_json::from_str(&s).expect("deserialize");
            assert_eq!(a, back);
            assert_eq!(back.kind, ActionKind::Move(d));
        }
    }

    #[test]
    fn action_kind_serde_round_trip() {
        for k in [ActionKind::Wait, ActionKind::Move(Direction::Right)] {
            let s = serde_json::to_string(&k).expect("serialize");
            let back: ActionKind = serde_json::from_str(&s).expect("deserialize");
            assert_eq!(k, back);
        }
    }

    // ---------- Neighbor geometry ----------

    #[test]
    fn neighbor_even_row_upper_left_keeps_the_column() {
        // Even rows are the shifted ones, so their upper-left sits at the same
        // column: (0,0) -> (0,-1) is off-map -> None.
        assert_eq!(neighbor(HexCoord::new(0, 0), Direction::UpperLeft, MapLayout::default()), None);
        let n = neighbor(HexCoord::new(2, 4), Direction::UpperLeft, MapLayout::default())
            .expect("in range");
        assert_eq!(n, HexCoord::new(2, 3));
    }

    #[test]
    fn neighbor_even_row_upper_right_shifts_right() {
        let n = neighbor(HexCoord::new(2, 4), Direction::UpperRight, MapLayout::default())
            .expect("in range");
        assert_eq!(n, HexCoord::new(3, 3));
    }

    #[test]
    fn neighbor_odd_row_upper_left_shifts_left() {
        let n = neighbor(HexCoord::new(2, 3), Direction::UpperLeft, MapLayout::default())
            .expect("in range");
        assert_eq!(n, HexCoord::new(1, 2));
    }

    #[test]
    fn neighbor_odd_row_upper_right_keeps_the_column() {
        let n = neighbor(HexCoord::new(2, 3), Direction::UpperRight, MapLayout::default())
            .expect("in range");
        assert_eq!(n, HexCoord::new(2, 2));
    }

    #[test]
    fn neighbor_right_and_left_are_row_independent() {
        for row in [0u8, 1, 2, 3, 4] {
            let r = neighbor(HexCoord::new(2, row), Direction::Right, MapLayout::default())
                .expect("in range");
            assert_eq!(r, HexCoord::new(3, row));
            let l = neighbor(HexCoord::new(2, row), Direction::Left, MapLayout::default())
                .expect("in range");
            assert_eq!(l, HexCoord::new(1, row));
        }
    }

    #[test]
    fn neighbor_lower_left_and_lower_right_branch_on_row_parity() {
        // Even row 4, col 2 (shifted right): lower-left keeps the column,
        // lower-right gains one — same table as engine `Direction::step`.
        assert_eq!(
            neighbor(HexCoord::new(2, 4), Direction::LowerLeft, MapLayout::default()),
            Some(HexCoord::new(2, 5))
        );
        assert_eq!(
            neighbor(HexCoord::new(2, 4), Direction::LowerRight, MapLayout::default()),
            Some(HexCoord::new(3, 5))
        );
        // Odd row 3, col 2: lower-left loses a column, lower-right keeps it.
        assert_eq!(
            neighbor(HexCoord::new(2, 3), Direction::LowerLeft, MapLayout::default()),
            Some(HexCoord::new(1, 4))
        );
        assert_eq!(
            neighbor(HexCoord::new(2, 3), Direction::LowerRight, MapLayout::default()),
            Some(HexCoord::new(2, 4))
        );
    }

    #[test]
    fn neighbor_returns_none_on_negative_overflow() {
        // Row 0, upper-left on even row gives (-1, -1) -> None.
        assert_eq!(neighbor(HexCoord::new(0, 0), Direction::UpperLeft, MapLayout::default()), None);
        // Row 0, upper-right on even row gives (-1, 0) -> None.
        assert_eq!(
            neighbor(HexCoord::new(0, 0), Direction::UpperRight, MapLayout::default()),
            None
        );
        // Col 0, left always gives (col=-1) -> None.
        assert_eq!(neighbor(HexCoord::new(0, 5), Direction::Left, MapLayout::default()), None);
    }

    // ---------- move_to: bounds ----------

    fn always_passable(_: HexCoord) -> bool {
        true
    }

    fn pond_at(target: HexCoord) -> impl Fn(HexCoord) -> bool {
        move |c| c != target
    }

    #[test]
    fn move_to_all_six_directions_succeed_in_interior() {
        let b = sample_bounds();
        let interior = HexCoord::new(5, 5);
        for d in Direction::ALL {
            assert!(
                move_to(interior, d, b, always_passable).is_some(),
                "expected {d:?} from interior to succeed",
            );
        }
    }

    #[test]
    fn move_to_top_left_corner_rejects_outbound() {
        let b = sample_bounds();
        // From (0,0) upper-left and upper-right and left are all off-map.
        assert_eq!(move_to(HexCoord::new(0, 0), Direction::UpperLeft, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(0, 0), Direction::UpperRight, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(0, 0), Direction::Left, b, always_passable), None);
    }

    #[test]
    fn move_to_top_right_corner_rejects_outbound() {
        let b = sample_bounds();
        // From (9, 0) upper-left and upper-right and right are all off-map.
        assert_eq!(move_to(HexCoord::new(9, 0), Direction::UpperLeft, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(9, 0), Direction::UpperRight, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(9, 0), Direction::Right, b, always_passable), None);
    }

    #[test]
    fn move_to_bottom_left_corner_rejects_outbound() {
        let b = sample_bounds();
        // From (0, 9) lower-left and lower-right and left are all off-map.
        assert_eq!(move_to(HexCoord::new(0, 9), Direction::LowerLeft, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(0, 9), Direction::LowerRight, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(0, 9), Direction::Left, b, always_passable), None);
    }

    #[test]
    fn move_to_bottom_right_corner_rejects_outbound() {
        let b = sample_bounds();
        // From (9, 9) lower-left and lower-right and right are all off-map.
        assert_eq!(move_to(HexCoord::new(9, 9), Direction::LowerLeft, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(9, 9), Direction::LowerRight, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(9, 9), Direction::Right, b, always_passable), None);
    }

    #[test]
    fn move_to_does_not_wrap_around_map() {
        // Q&A Q2: out-of-range coords must NOT wrap.
        let b = sample_bounds();
        assert_eq!(move_to(HexCoord::new(0, 0), Direction::UpperLeft, b, always_passable), None);
        assert_eq!(move_to(HexCoord::new(0, 0), Direction::Left, b, always_passable), None);
    }

    // ---------- move_to: ponds ----------

    #[test]
    fn move_to_rejects_pond_at_destination() {
        let b = sample_bounds();
        let origin = HexCoord::new(5, 5);
        let pond = HexCoord::new(6, 5); // east of origin
        assert_eq!(move_to(origin, Direction::Right, b, pond_at(pond)), None);
    }

    #[test]
    fn move_to_accepts_passable_destination() {
        let b = sample_bounds();
        let origin = HexCoord::new(5, 5);
        // No pond; move east succeeds.
        assert_eq!(
            move_to(origin, Direction::Right, b, always_passable),
            Some(HexCoord::new(6, 5))
        );
    }

    #[test]
    fn move_to_passes_through_when_destination_is_passable_but_intermediate_not_checked() {
        // This test guards the contract that move_to inspects ONLY the
        // destination cell. (Intermediate cells don't exist in a 1-step
        // move; this is purely documentation.)
        let b = sample_bounds();
        let origin = HexCoord::new(5, 5);
        let pond = HexCoord::new(0, 0); // far away
        assert_eq!(move_to(origin, Direction::Right, b, pond_at(pond)), Some(HexCoord::new(6, 5)));
    }

    // ---------- Action::validate ----------

    #[test]
    fn validate_wait_always_succeeds() {
        let b = sample_bounds();
        let a = Action::wait(0);
        assert!(a.validate(HexCoord::new(0, 0), b, always_passable).is_ok());
    }

    #[test]
    fn validate_move_returns_next_coord_on_success() {
        let b = sample_bounds();
        let a = Action::move_in(0, Direction::Right);
        let next = a.validate(HexCoord::new(3, 3), b, always_passable).expect("right is valid");
        assert_eq!(next, HexCoord::new(4, 3));
    }

    #[test]
    fn validate_move_returns_illegal_move_off_map() {
        let b = sample_bounds();
        let a = Action::move_in(0, Direction::UpperLeft);
        let err = a.validate(HexCoord::new(0, 0), b, always_passable).unwrap_err();
        assert!(matches!(err, CoreError::IllegalMove(_)));
    }

    #[test]
    fn validate_move_returns_illegal_move_into_pond() {
        let b = sample_bounds();
        let pond = HexCoord::new(6, 5);
        let a = Action::move_in(0, Direction::Right);
        let err = a.validate(HexCoord::new(5, 5), b, pond_at(pond)).unwrap_err();
        assert!(matches!(err, CoreError::IllegalMove(_)));
    }

    // ---------- validate_batch (Q&A Q6) ----------

    #[test]
    fn validate_batch_all_valid_returns_resolved_positions() {
        let b = sample_bounds();
        let batch = vec![
            (Action::wait(0), HexCoord::new(0, 0)),
            (Action::move_in(1, Direction::Right), HexCoord::new(2, 2)),
            (Action::move_in(2, Direction::LowerRight), HexCoord::new(4, 4)),
        ];
        let out = validate_batch(&batch, b, &always_passable).expect("all valid");
        // LowerRight from an even row (4) gains a column: (4,4) -> (5,5).
        assert_eq!(out, vec![HexCoord::new(0, 0), HexCoord::new(3, 2), HexCoord::new(5, 5)]);
    }

    #[test]
    fn validate_batch_rejects_entire_batch_on_first_failure() {
        // Q&A Q6: a single bad action rejects the WHOLE batch.
        let b = sample_bounds();
        let batch = vec![
            (Action::move_in(0, Direction::Right), HexCoord::new(2, 2)),
            // This action is invalid (off-map).
            (Action::move_in(1, Direction::UpperLeft), HexCoord::new(0, 0)),
            (Action::wait(2), HexCoord::new(5, 5)),
        ];
        let err = validate_batch(&batch, b, &always_passable).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("batch rejected at index 1"), "got: {msg}");
        assert!(msg.contains("Q6"), "got: {msg}");
    }

    #[test]
    fn validate_batch_rejects_entire_batch_on_pond() {
        let b = sample_bounds();
        let pond = HexCoord::new(3, 2);
        let batch = vec![
            (Action::wait(0), HexCoord::new(0, 0)),
            (Action::move_in(1, Direction::Right), HexCoord::new(2, 2)),
            (Action::wait(2), HexCoord::new(5, 5)),
        ];
        let err = validate_batch(&batch, b, &pond_at(pond)).unwrap_err();
        assert!(matches!(err, CoreError::IllegalMove(_)));
    }

    #[test]
    fn validate_batch_empty_returns_empty_vec() {
        let b = sample_bounds();
        let out = validate_batch(&[], b, &always_passable).expect("empty batch is trivially valid");
        assert!(out.is_empty());
    }
}
