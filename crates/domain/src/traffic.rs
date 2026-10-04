//! Traffic accumulation per road cell.
//!
//! Spec trace:
//!
//! - 募集要項「移動と燃料」「混雑」「渋滞」
//! - Q&A Q6 (traffic update order: movement → traffic accumulation)
//! - Q&A Q13 (busy threshold 1-5, jammed threshold 2-10)
//! - Q&A Q14 / Q19 (match-configurable thresholds)
//! - `docs/contest/strategy.md` §4 道路渋滞
//!
//! ## Design notes
//!
//! `TrafficState` is re-declared here independently of any other worktree
//! (e.g. #12 `cell.rs`). The other branches are not yet merged, so duplication
//! is intentional and avoids cross-branch coupling. The post-merge integration
//! will collapse this into the canonical cell-side definition.
//!
//! The traffic volume stored in [`TrafficCell`] is the **2-day sliding max**
//! of the previous two days' traversals (Q&A Q6 処理順序). The "today" count
//! accumulates during the day via [`TrafficMap::record_traversal`]; the phase
//! transition at end-of-day calls [`TrafficMap::phase_traffic_update`], which
//! promotes "today" → "yesterday", inserts a fresh "today" of zero, and
//! re-classifies the state from `max(count_yesterday, count_today)`.
//!
//! Position is stored as `(col, row)` `(u8, u8)` rather than a richer hex
//! coordinate type because this worktree does not yet have `coordinate.rs`
//! (branch #11, not merged). The flat tuple keeps the cross-branch integration
//! minimal.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Congestion state of a road cell.
///
/// Classification is decided by [`TrafficCell::classify`] using [`Thresholds`]
/// and the cell's current 2-day traffic volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrafficState {
    /// 順調 — low congestion, lowest move-step cost.
    Smooth,
    /// 混雑 — moderate congestion, mid move-step cost.
    Busy,
    /// 渋滞 — heavy congestion, highest move-step cost.
    Jammed,
}

/// Threshold pair used to classify traffic volume into [`TrafficState`].
///
/// Q&A Q13 documents the legal ranges (`busy ∈ [1, 5]`, `jammed ∈ [2, 10]`).
/// Q&A Q14 / Q19 lets the match configuration choose concrete values inside
/// those ranges. Defaults are the mid-range values used in this module's
/// tests; the match configuration is expected to override them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Thresholds {
    /// Minimum traffic volume that promotes a cell to [`TrafficState::Busy`].
    pub busy: u8,
    /// Minimum traffic volume that promotes a cell to [`TrafficState::Jammed`].
    /// Must be strictly greater than `busy`.
    pub jammed: u8,
}

impl Thresholds {
    /// Lower bound of the busy threshold range (Q&A Q13).
    pub const BUSY_MIN: u8 = 1;
    /// Upper bound of the busy threshold range (Q&A Q13).
    pub const BUSY_MAX: u8 = 5;
    /// Lower bound of the jammed threshold range (Q&A Q13).
    pub const JAMMED_MIN: u8 = 2;
    /// Upper bound of the jammed threshold range (Q&A Q13).
    pub const JAMMED_MAX: u8 = 10;
    /// Default mid-range thresholds used by the in-module tests. Match
    /// configurations should override these.
    pub const DEFAULT: Self = Self { busy: 3, jammed: 7 };

    /// Construct a [`Thresholds`] from raw values, validating against the
    /// Q&A Q13 ranges and the strict `jammed > busy` invariant.
    pub fn new(busy: u8, jammed: u8) -> Result<Self, String> {
        if !(Self::BUSY_MIN..=Self::BUSY_MAX).contains(&busy) {
            return Err(format!(
                "busy threshold {busy} out of Q13 range [{}, {}]",
                Self::BUSY_MIN,
                Self::BUSY_MAX
            ));
        }
        if !(Self::JAMMED_MIN..=Self::JAMMED_MAX).contains(&jammed) {
            return Err(format!(
                "jammed threshold {jammed} out of Q13 range [{}, {}]",
                Self::JAMMED_MIN,
                Self::JAMMED_MAX
            ));
        }
        if jammed <= busy {
            return Err(format!(
                "jammed threshold {jammed} must be strictly greater than busy threshold {busy}"
            ));
        }
        Ok(Self { busy, jammed })
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Per-cell traffic counter and current state.
///
/// `pos` is `(col, row)`. `count_yesterday` is the day-prior snapshot;
/// `count_today` is the in-progress accumulator. The actual displayed state
/// is re-derived from `max(count_yesterday, count_today)` via
/// [`TrafficCell::classify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficCell {
    /// Cell coordinate in `(col, row)` order.
    pub pos: (u8, u8),
    /// Number of agent traversals recorded so far today.
    pub count_today: u32,
    /// Number of agent traversals recorded yesterday (snapshot from the
    /// previous day's end-of-phase update).
    pub count_yesterday: u32,
    /// Current displayed congestion state.
    pub state: TrafficState,
}

impl TrafficCell {
    /// Construct a fresh cell at `pos` with zero counters and a
    /// [`TrafficState::Smooth`] default.
    #[must_use]
    pub const fn new(pos: (u8, u8)) -> Self {
        Self { pos, count_today: 0, count_yesterday: 0, state: TrafficState::Smooth }
    }

    /// Classify the current 2-day max volume into a [`TrafficState`].
    ///
    /// Boundaries:
    /// - `volume >= jammed` → [`TrafficState::Jammed`]
    /// - `volume >= busy` → [`TrafficState::Busy`]
    /// - otherwise → [`TrafficState::Smooth`]
    #[must_use]
    pub fn classify(&self, thresholds: Thresholds) -> TrafficState {
        let volume = self.count_yesterday.max(self.count_today);
        if volume >= u32::from(thresholds.jammed) {
            TrafficState::Jammed
        } else if volume >= u32::from(thresholds.busy) {
            TrafficState::Busy
        } else {
            TrafficState::Smooth
        }
    }
}

/// Map-wide traffic snapshot keyed by `(col, row)`.
///
/// The map only stores cells that have been observed (created on the first
/// [`TrafficMap::record_traversal`] call for that position). Cells never
/// touched by any agent are simply absent from the map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficMap {
    cells: HashMap<(u8, u8), TrafficCell>,
    thresholds: Thresholds,
}

impl TrafficMap {
    /// Construct an empty [`TrafficMap`] with the default mid-range
    /// thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self { cells: HashMap::new(), thresholds: Thresholds::DEFAULT }
    }

    /// Construct an empty [`TrafficMap`] with caller-supplied thresholds.
    #[must_use]
    pub fn with_thresholds(thresholds: Thresholds) -> Self {
        Self { cells: HashMap::new(), thresholds }
    }

    /// Current thresholds in effect.
    #[must_use]
    pub const fn thresholds(&self) -> Thresholds {
        self.thresholds
    }

    /// Replace the thresholds in effect. Existing cells are *not* reclassified
    /// — callers should invoke [`Self::phase_traffic_update`] if they want the
    /// display state to reflect the new thresholds.
    pub fn set_thresholds(&mut self, thresholds: Thresholds) {
        self.thresholds = thresholds;
    }

    /// Number of distinct cells with recorded traffic.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Look up a cell by position. Returns `None` if the position has never
    /// been touched.
    #[must_use]
    pub fn get(&self, pos: (u8, u8)) -> Option<&TrafficCell> {
        self.cells.get(&pos)
    }

    /// Record a single agent traversal of `pos`. Increments `count_today`.
    ///
    /// Idempotent over the position: a fresh cell is created on first call.
    /// The displayed `state` is *not* updated here — that happens at the end
    /// of the day via [`Self::phase_traffic_update`] so the sliding window
    /// picks up the full day's accumulation before reclassification.
    pub fn record_traversal(&mut self, pos: (u8, u8)) {
        let cell = self.cells.entry(pos).or_insert_with(|| TrafficCell::new(pos));
        cell.count_today = cell.count_today.saturating_add(1);
    }

    /// End-of-phase update (Q&A Q6 処理順序).
    ///
    /// For every tracked cell, in this order:
    /// 1. Re-derive `state` from `max(count_yesterday, count_today)` using the
    ///    current thresholds — i.e. from the two days that just ended.
    /// 2. Promote `count_today` → `count_yesterday`.
    /// 3. Reset `count_today` to 0.
    ///
    /// Classifying first is what makes the documented 2-day sliding window
    /// real: rotating first destroyed the older day before `classify` could
    /// read it, so a cell that jammed yesterday fell straight back to Smooth
    /// as soon as today had zero traffic. `crates/engine`'s
    /// `phase_traffic_update` classifies from `traffic_today` +
    /// `traffic_yesterday` before `begin_day` rotates, and this has to agree
    /// with it.
    ///
    /// After this call, `count_today` is zero everywhere and `count_yesterday`
    /// holds the previous day's total, so the very next day's
    /// [`Self::record_traversal`] starts the new accumulation window.
    pub fn phase_traffic_update(&mut self) {
        let thresholds = self.thresholds;
        for cell in self.cells.values_mut() {
            cell.state = cell.classify(thresholds);
            cell.count_yesterday = cell.count_today;
            cell.count_today = 0;
        }
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Boundary matrix taken from the Issue #58 acceptance criteria.
    ///
    /// With `Thresholds { busy: 3, jammed: 7 }`:
    /// - `count < 3` → Smooth
    /// - `3 <= count < 7` → Busy
    /// - `count >= 7` → Jammed
    #[test]
    fn threshold_boundaries() {
        let thresholds = Thresholds::new(3, 7).expect("mid-range thresholds are valid");

        // count < busy → Smooth (covers 0..=2, including the boundary-1 case)
        let cell = TrafficCell {
            pos: (0, 0),
            count_today: 2,
            count_yesterday: 0,
            state: TrafficState::Smooth,
        };
        assert_eq!(cell.classify(thresholds), TrafficState::Smooth);

        // count == busy → Busy (lower boundary of Busy)
        let cell = TrafficCell {
            pos: (0, 0),
            count_today: 3,
            count_yesterday: 0,
            state: TrafficState::Smooth,
        };
        assert_eq!(cell.classify(thresholds), TrafficState::Busy);

        // count == jammed - 1 → Busy (just inside Busy range)
        let cell = TrafficCell {
            pos: (0, 0),
            count_today: 6,
            count_yesterday: 0,
            state: TrafficState::Smooth,
        };
        assert_eq!(cell.classify(thresholds), TrafficState::Busy);

        // count == jammed → Jammed (lower boundary of Jammed)
        let cell = TrafficCell {
            pos: (0, 0),
            count_today: 7,
            count_yesterday: 0,
            state: TrafficState::Smooth,
        };
        assert_eq!(cell.classify(thresholds), TrafficState::Jammed);

        // count > jammed → Jammed
        let cell = TrafficCell {
            pos: (0, 0),
            count_today: 100,
            count_yesterday: 0,
            state: TrafficState::Smooth,
        };
        assert_eq!(cell.classify(thresholds), TrafficState::Jammed);
    }

    /// 2-day sliding max: yesterday's count dominates today's small count.
    ///
    /// `count_yesterday = 5`, `count_today = 1` → max = 5.
    /// With defaults (`busy = 3`), volume 5 classifies as Busy.
    #[test]
    fn two_day_sliding_max() {
        let cell = TrafficCell {
            pos: (1, 2),
            count_today: 1,
            count_yesterday: 5,
            state: TrafficState::Smooth,
        };
        assert_eq!(cell.classify(Thresholds::DEFAULT), TrafficState::Busy);
        // Symmetric: yesterday small, today large still picks up the max.
        let cell = TrafficCell {
            pos: (1, 2),
            count_today: 5,
            count_yesterday: 1,
            state: TrafficState::Smooth,
        };
        assert_eq!(cell.classify(Thresholds::DEFAULT), TrafficState::Busy);
    }

    /// Recording multiple traversals on the same cell aggregates into
    /// `count_today` and is idempotent over distinct positions.
    #[test]
    fn record_multiple_traversals() {
        let mut map = TrafficMap::new();

        // Three distinct cells.
        map.record_traversal((0, 0));
        map.record_traversal((0, 0));
        map.record_traversal((1, 2));
        assert_eq!(map.len(), 2);

        // (0, 0) saw 2 traversals, (1, 2) saw 1.
        assert_eq!(map.get((0, 0)).expect("cell exists").count_today, 2);
        assert_eq!(map.get((1, 2)).expect("cell exists").count_today, 1);

        // record_traversal does *not* touch state — it waits for the phase
        // update so the sliding window sees the full day.
        assert_eq!(map.get((0, 0)).expect("cell exists").state, TrafficState::Smooth);
    }

    /// `phase_traffic_update` promotes today → yesterday, resets today, and
    /// re-derives the state from the 2-day max.
    #[test]
    fn phase_update_promotes_and_reclassifies() {
        let mut map = TrafficMap::new();
        // 5 traversals today, 0 yesterday → max 5, default busy = 3 → Busy.
        for _ in 0..5 {
            map.record_traversal((4, 4));
        }
        map.phase_traffic_update();

        let cell = map.get((4, 4)).expect("cell exists");
        assert_eq!(cell.count_yesterday, 5);
        assert_eq!(cell.count_today, 0);
        assert_eq!(cell.state, TrafficState::Busy);

        // Day 2: zero traversals → the window still holds yesterday's 5, so
        // the cell stays Busy. (Classifying after the rotation used to drop
        // that 5 first and report Smooth — the 2-day window never happened.)
        map.phase_traffic_update();
        let cell = map.get((4, 4)).expect("cell exists");
        assert_eq!(cell.count_yesterday, 0, "previous yesterday is dropped");
        assert_eq!(cell.count_today, 0);
        assert_eq!(cell.state, TrafficState::Busy, "max(5 from yesterday, 0) stays Busy");

        // Day 3 with *no* traffic either: the window is now empty, so Smooth
        // is correct — and only now.
        map.phase_traffic_update();
        let cell = map.get((4, 4)).expect("cell exists");
        assert_eq!(cell.state, TrafficState::Smooth, "empty 2-day window is Smooth");

        // Day 3: 4 traversals → max 4, default busy = 3 → Busy.
        for _ in 0..4 {
            map.record_traversal((4, 4));
        }
        map.phase_traffic_update();
        let cell = map.get((4, 4)).expect("cell exists");
        assert_eq!(cell.count_yesterday, 4);
        assert_eq!(cell.state, TrafficState::Busy);
    }

    /// `Thresholds::new` rejects out-of-range or non-strict values.
    #[test]
    fn threshold_validation() {
        // Q13 ranges.
        assert!(Thresholds::new(0, 2).is_err(), "busy below Q13 range");
        assert!(Thresholds::new(6, 7).is_err(), "busy above Q13 range");
        assert!(Thresholds::new(1, 1).is_err(), "jammed below Q13 range");
        assert!(Thresholds::new(3, 11).is_err(), "jammed above Q13 range");

        // Strict ordering invariant.
        assert!(Thresholds::new(3, 3).is_err(), "jammed must be strictly > busy");
        assert!(Thresholds::new(4, 3).is_err(), "jammed must be strictly > busy");

        // Valid mid-range pair.
        assert!(Thresholds::new(3, 7).is_ok());
        // Valid edge of Q13 range.
        assert!(Thresholds::new(1, 10).is_ok());
    }
}
