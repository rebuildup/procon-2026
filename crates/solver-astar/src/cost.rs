//! Cost model for the A* solver.
//!
//! Spec anchor: 募集要項「移動と燃料」. The base step costs and the three
//! road-traffic values come straight from the recruitment document and Q&A
//! Q13:
//!
//! | Cell              | Step cost | Fuel cost |
//! |-------------------|-----------|-----------|
//! | Plain             | 1         | 1         |
//! | Mountain          | 2         | 2         |
//! | Road (Smooth)     | 1         | 2         |
//! | Road (Busy)       | 2         | 2         |
//! | Road (Jammed)     | 4         | 2         |
//! | Pond              | — (impassable) | — |
//!
//! A* minimises the weighted step cost. The weight is multiplied by a
//! fuel-remaining factor so the solver prefers "save the last drops of fuel"
//! when configured for conservation; see [`CostPolicy::fuel_scaling`].
//!
//! The same [`CostModel`] is used for both 経路長 comparison (the A* g/h
//! scores) and 燃料 consumption accounting; the two never diverge because
//! every cell has a fixed fuel cost from the table above.

use crate::hex::{CellKind, TrafficState};

/// Base step cost of a cell. Matches the recruitment document's table.
///
/// **Known unit divergence:** the official `daySteps` budget counts the
/// *origin* cell's cost with `domain::cell::MOVE_STEPS` (plain 2 / mountain 3
/// / road 1·2·4 — cross-check `engine::wire_plan::terrain_step_cost`), while
/// this table bills the *entered* cell as plain 1 / mountain 2.
/// [`SolveContext::max_steps`] mirrors `daySteps`, so every `<= max_steps`
/// budget check in this crate runs in mixed units and can hand back a path
/// the wire-level `normalize_agent_plan` later truncates mid-walk.
/// `budget=official` fixes that for the `loop` / `multiloop` candidates; the
/// `astar` candidate has no such knob yet. Aligning the table re-costs every
/// A* candidate (scores shift, `plain_step_cost_is_one` and
/// `mountain_step_cost_is_two` pin the current values), so it is tracked as a
/// follow-up instead of landing with the fuel-scaling fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BaseStepCost(pub u32);

impl BaseStepCost {
    /// Plain cell: 1 step.
    pub const PLAIN: Self = Self(1);
    /// Mountain cell: 2 steps.
    pub const MOUNTAIN: Self = Self(2);

    /// Step cost of entering `cell`; `None` for ponds (impassable).
    ///
    /// This is the *unscaled* cost — the unit [`SolveContext::max_steps`] is
    /// expressed in. Fuel scaling (see [`CostPolicy::conservative`]) applies
    /// on top and must never be compared against the day budget.
    #[must_use]
    pub const fn for_cell(cell: CellKind) -> Option<Self> {
        match cell {
            CellKind::Plain => Some(Self::PLAIN),
            CellKind::Mountain => Some(Self::MOUNTAIN),
            CellKind::Road { traffic } => Some(Self::road(traffic)),
            CellKind::Pond => None,
        }
    }

    /// Step cost for a road cell in the given traffic state.
    #[must_use]
    pub const fn road(traffic: TrafficState) -> Self {
        match traffic {
            TrafficState::Smooth => Self(1),
            TrafficState::Busy => Self(2),
            TrafficState::Jammed => Self(4),
        }
    }
}

/// Base fuel cost of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BaseFuelCost(pub u32);

impl BaseFuelCost {
    /// Plain cell: 1 fuel.
    pub const PLAIN: Self = Self(1);
    /// Mountain cell: 2 fuel.
    pub const MOUNTAIN: Self = Self(2);
    /// Road cell: 2 fuel regardless of traffic state.
    pub const ROAD: Self = Self(2);

    /// Fuel cost of the given cell kind. Returns `None` for [`CellKind::Pond`].
    #[must_use]
    pub fn for_cell(cell: CellKind) -> Option<Self> {
        match cell {
            CellKind::Plain => Some(Self::PLAIN),
            CellKind::Mountain => Some(Self::MOUNTAIN),
            CellKind::Road { .. } => Some(Self::ROAD),
            CellKind::Pond => None,
        }
    }
}

/// Pluggable A* cost policy.
///
/// Two knobs are exposed to the caller so that the coordinator can pick a
/// behaviour without recompiling the crate:
///
/// - `fuel_scaling` penalises moves made when fuel is already low (encourages
///   conservation; the `variety`-first heuristic in the strategy doc).
/// - `forbid_when_empty` shuts down all movement when fuel reaches zero. The
///   A* expansion treats such cells as if they were ponds.
///
/// All policies always forbid entering a pond, regardless of the knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostPolicy {
    /// Scale the step cost by a factor that grows as fuel drops.
    ///
    /// Effective weight = base step cost × `1 + scale × (1 − fuel/max_fuel)`.
    /// When `fuel == max_fuel`, the weight is unchanged; when `fuel == 0` it
    /// is `1 + scale` times the base. Setting this to 0 disables scaling.
    pub fuel_scaling: u32,

    /// Threshold (in fuel units) below which a step is treated as "out of
    /// fuel" and rejected if [`Self::forbid_when_empty`] is set.
    pub fuel_low_threshold: u32,

    /// If true, refuse to spend the last `fuel_low_threshold` units. Used to
    /// preserve enough fuel for the return trip to the supply truck.
    pub forbid_when_empty: bool,

    /// Maximum fuel the planner expects the agent to carry. Used together
    /// with `fuel_scaling` to compute the remaining-fuel ratio.
    pub max_fuel: u32,
}

impl Default for CostPolicy {
    fn default() -> Self {
        Self { fuel_scaling: 0, fuel_low_threshold: 0, forbid_when_empty: false, max_fuel: 1 }
    }
}

impl CostPolicy {
    /// A conservation-friendly default: heavily penalise fuel use, but
    /// never outright forbid movement.
    #[must_use]
    pub fn conservative(max_fuel: u32) -> Self {
        Self {
            fuel_scaling: 4,
            fuel_low_threshold: 0,
            forbid_when_empty: false,
            max_fuel: max_fuel.max(1),
        }
    }

    /// An aggressive default that minimises weighted step cost and treats
    /// zero-fuel cells as walls.
    #[must_use]
    pub fn strict(max_fuel: u32) -> Self {
        Self {
            fuel_scaling: 0,
            fuel_low_threshold: 1,
            forbid_when_empty: true,
            max_fuel: max_fuel.max(1),
        }
    }

    /// Compute the weighted step cost for entering `cell` from a state with
    /// `fuel_remaining` fuel left.
    ///
    /// Returns `None` when the move is impossible (pond, out-of-fuel when
    /// [`Self::forbid_when_empty`] is on).
    #[must_use]
    pub fn weighted_cost(&self, cell: CellKind, fuel_remaining: u32) -> Option<u32> {
        let base = BaseStepCost::for_cell(cell)?.0;
        if self.forbid_when_empty && fuel_remaining <= self.fuel_low_threshold {
            return None;
        }
        // Saturating arithmetic — the multiplier is `1 + scale * (1 - f/max)`,
        // expressed in integer hundredths to avoid floats in the hot loop.
        let max = self.max_fuel.max(1);
        let remaining = fuel_remaining.min(max);
        let depleted_ratio_hundredths = (max - remaining) * 100 / max; // 0..=100
        let multiplier_hundredths = 100 + self.fuel_scaling * depleted_ratio_hundredths;
        // Ceiling division so we never accidentally return 0 for a non-trivial move.
        let cost = u64::from(base) * u64::from(multiplier_hundredths) / 100;
        let cost = cost.max(1);
        u32::try_from(cost).ok()
    }

    /// Whether a cell is allowed as a neighbour for expansion.
    ///
    /// Ponds are always rejected. With `forbid_when_empty`, moves that would
    /// drop fuel below `fuel_low_threshold` are also rejected.
    #[must_use]
    pub fn allows(&self, cell: CellKind, fuel_remaining: u32) -> bool {
        if !cell.is_passable() {
            return false;
        }
        if self.forbid_when_empty && fuel_remaining <= self.fuel_low_threshold {
            return false;
        }
        true
    }
}

/// Trait for pluggable cost models.
///
/// The default [`DefaultCostModel`] is sufficient for the issue #25 spec,
/// but downstream solvers (e.g. strategy-3 ones) can swap in a model that
/// targets a different objective.
pub trait CostModel {
    /// Weighted step cost for entering `cell` with the given remaining fuel.
    fn weighted_cost(
        &self,
        cell: CellKind,
        fuel_remaining: u32,
        policy: &CostPolicy,
    ) -> Option<u32>;

    /// Fuel consumed by the move. `None` for impassable cells.
    fn fuel_cost(&self, cell: CellKind) -> Option<u32>;

    /// Whether the cell may be expanded as a neighbour.
    fn allows(&self, cell: CellKind, fuel_remaining: u32, policy: &CostPolicy) -> bool;
}

/// The default cost model. Delegates everything to [`CostPolicy`].
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultCostModel;

impl CostModel for DefaultCostModel {
    fn weighted_cost(
        &self,
        cell: CellKind,
        fuel_remaining: u32,
        policy: &CostPolicy,
    ) -> Option<u32> {
        policy.weighted_cost(cell, fuel_remaining)
    }

    fn fuel_cost(&self, cell: CellKind) -> Option<u32> {
        BaseFuelCost::for_cell(cell).map(|c| c.0)
    }

    fn allows(&self, cell: CellKind, fuel_remaining: u32, policy: &CostPolicy) -> bool {
        policy.allows(cell, fuel_remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_fuel: u32) -> CostPolicy {
        CostPolicy { fuel_scaling: 0, fuel_low_threshold: 0, forbid_when_empty: false, max_fuel }
    }

    #[test]
    fn plain_step_cost_is_one() {
        assert_eq!(policy(10).weighted_cost(CellKind::Plain, 10), Some(1));
    }

    #[test]
    fn mountain_step_cost_is_two() {
        assert_eq!(policy(10).weighted_cost(CellKind::Mountain, 10), Some(2));
    }

    #[test]
    fn road_step_cost_reflects_traffic() {
        let p = policy(10);
        assert_eq!(p.weighted_cost(CellKind::Road { traffic: TrafficState::Smooth }, 10), Some(1));
        assert_eq!(p.weighted_cost(CellKind::Road { traffic: TrafficState::Busy }, 10), Some(2));
        assert_eq!(p.weighted_cost(CellKind::Road { traffic: TrafficState::Jammed }, 10), Some(4));
    }

    #[test]
    fn pond_is_impassable() {
        let p = policy(10);
        assert_eq!(p.weighted_cost(CellKind::Pond, 10), None);
        assert!(!p.allows(CellKind::Pond, 10));
    }

    #[test]
    fn forbid_when_empty_blocks_when_low() {
        let p = CostPolicy::strict(10);
        assert_eq!(p.weighted_cost(CellKind::Plain, 0), None);
        assert!(!p.allows(CellKind::Plain, 0));
        assert!(p.allows(CellKind::Plain, 2));
    }

    #[test]
    fn fuel_scaling_grows_weight_as_fuel_drops() {
        let p = CostPolicy::conservative(100);
        let full = p.weighted_cost(CellKind::Plain, 100).unwrap();
        let half = p.weighted_cost(CellKind::Plain, 50).unwrap();
        let empty = p.weighted_cost(CellKind::Plain, 0).unwrap();
        assert!(full <= half, "full ({full}) should be <= half ({half})");
        assert!(half < empty, "half ({half}) should be < empty ({empty})");
    }

    #[test]
    fn weighted_cost_never_collapses_to_zero() {
        let p = policy(10);
        // Plain cell with maximum fuel scaling should still cost >= 1.
        let mut scaling = p;
        scaling.fuel_scaling = 1_000_000;
        scaling.max_fuel = 1;
        let cost = scaling.weighted_cost(CellKind::Plain, 1).unwrap();
        assert!(cost >= 1);
    }
}
