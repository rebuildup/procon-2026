//! Dynamic replanning: detect inventory / traffic changes between turns and
//! trigger re-clustering / re-assignment when the change exceeds a
//! configurable threshold.
//!
//! See `docs/contest/strategy.md` §5.3 — the static cluster map computed at
//! the start of a day is only useful as long as the underlying world state
//! (remaining spot stock + road congestion) has not shifted meaningfully.
//! When opponents claim balls, or when shared roads tip from 順調 (1) to
//! 混雑 (2) or 渋滞 (4), the greedy assignment from the previous turn is
//! stale. [`DynamicReplanner`] wraps an inner [`Solver`] and decides, on each
//! turn, whether the cached plan should be reused or whether the inner solver
//! should be re-invoked to recompute clusters + assignments.
//!
//! EXPECTED INTEGRATION: domain
//! -----------------------------
//! [`InventorySnapshot`] and [`TrafficSnapshot`] are local stub types — once
//! the `domain` crate (Issue #11) lands they are expected to be replaced by
//! the canonical domain snapshots. The diff metric and threshold contract
//! should stay stable; only the concrete field types move.
//!
//! [`Solver`]: crate::solver::Solver

#![allow(clippy::cast_precision_loss)]

use crate::solver::{SeriesClusteredSolver, Solver};
use crate::types::{HexCoord, SolveRequest, SolveResponse, SpotId};
use std::collections::BTreeMap;

/// Per-spot stock snapshot at one turn. `SpotId -> remaining stock`.
///
/// Spot ids present in `stocks` have a known remaining count; spots absent
/// from the map are treated as having zero stock when [`Self::diff`] compares
/// against another snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InventorySnapshot {
    pub stocks: BTreeMap<SpotId, u32>,
}

impl InventorySnapshot {
    /// Build a snapshot from an iterator of `(spot_id, stock)` pairs.
    #[must_use]
    pub fn from_pairs(pairs: impl IntoIterator<Item = (SpotId, u32)>) -> Self {
        Self { stocks: pairs.into_iter().collect() }
    }

    /// Read the stock of `spot` (zero if absent).
    #[must_use]
    pub fn stock_of(&self, spot: &SpotId) -> u32 {
        self.stocks.get(spot).copied().unwrap_or(0)
    }

    /// Symmetric mean absolute change vs `other`, normalised by `other`'s
    /// total stock. Range: `[0.0, 1.0+]` where `1.0` means "all stock moved
    /// or vanished" and `0.0` means "identical".
    ///
    /// Edge cases:
    /// - Both snapshots empty → `0.0`.
    /// - `other` empty, `self` non-empty → `1.0` (everything is new).
    /// - `self` empty, `other` non-empty → `1.0` (everything vanished).
    #[must_use]
    pub fn diff(&self, other: &InventorySnapshot) -> f64 {
        let mut total_diff: u64 = 0;
        let mut total_old: u64 = 0;
        // Walk the union in deterministic order so diff is stable.
        let mut all_keys: Vec<&SpotId> = self.stocks.keys().collect();
        for k in other.stocks.keys() {
            if !self.stocks.contains_key(k) {
                all_keys.push(k);
            }
        }
        all_keys.sort();
        all_keys.dedup();
        for k in all_keys {
            let a = self.stock_of(k);
            let b = other.stock_of(k);
            total_diff += (i64::from(a) - i64::from(b)).unsigned_abs();
            total_old += u64::from(b);
        }
        match total_old {
            0 => {
                if total_diff == 0 {
                    0.0
                } else {
                    1.0
                }
            }
            _ => total_diff as f64 / total_old as f64,
        }
    }
}

/// Per-edge congestion snapshot at one turn. `(from, to) -> congestion level`.
///
/// Edges absent from the map are treated as congestion-free when [`Self::diff`]
/// compares against another snapshot. Congestion levels are non-negative
/// integers; the canonical encoding is the contest's 順調=1 / 混雑=2 /
/// 渋滞=4 rule, but the metric is unit-less so any encoding works.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrafficSnapshot {
    pub congestion: std::collections::HashMap<(HexCoord, HexCoord), u32>,
}

impl TrafficSnapshot {
    /// Build a snapshot from an iterator of `((from, to), level)` pairs.
    #[must_use]
    pub fn from_pairs(pairs: impl IntoIterator<Item = ((HexCoord, HexCoord), u32)>) -> Self {
        Self { congestion: pairs.into_iter().collect() }
    }

    /// Read the congestion of `edge` (zero if absent).
    #[must_use]
    pub fn congestion_of(&self, edge: &(HexCoord, HexCoord)) -> u32 {
        self.congestion.get(edge).copied().unwrap_or(0)
    }

    /// Symmetric mean absolute change in congestion vs `other`, normalised
    /// by the maximum of `self`'s and `other`'s total congestion. Range:
    /// `[0.0, 1.0]`. `0.0` means identical; `1.0` means complete turnover.
    ///
    /// Edge cases mirror [`InventorySnapshot::diff`].
    #[must_use]
    pub fn diff(&self, other: &TrafficSnapshot) -> f64 {
        let mut total_diff: u64 = 0;
        let mut total_max: u64 = 0;
        // Walk the union. `HashMap` ordering is non-deterministic, but the
        // L1/max metric only depends on per-edge values, not iteration order.
        let mut all_keys: std::collections::HashSet<(HexCoord, HexCoord)> =
            self.congestion.keys().copied().collect();
        for k in other.congestion.keys() {
            all_keys.insert(*k);
        }
        for k in all_keys {
            let a = u64::from(self.congestion_of(&k));
            let b = u64::from(other.congestion_of(&k));
            total_diff += a.abs_diff(b);
            total_max += a.max(b);
        }
        if total_max == 0 { 0.0 } else { total_diff as f64 / total_max as f64 }
    }
}

/// The previous solver output. Aliased to `SolveResponse` to match the
/// crate's existing public surface; once the domain swap lands the alias can
/// be redirected without breaking callers.
pub type Plan = SolveResponse;

/// State tracked across turns by [`DynamicReplanner`].
///
/// Owners are expected to construct one of these on turn 0 (no previous plan)
/// and then pass it back into [`DynamicReplanner::solve_with_state`] on each
/// subsequent turn so the planner can compare the current world against the
/// last snapshot.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)] // Fields named `last_*` per Issue #30 spec.
pub struct PlanState {
    pub last_plan: Plan,
    pub last_inventory: InventorySnapshot,
    pub last_traffic: TrafficSnapshot,
    pub last_replan_turn: u32,
}

impl PlanState {
    /// Build a fresh state. `last_replan_turn` is typically `0` when there
    /// is no prior plan.
    #[must_use]
    pub fn new(
        last_plan: Plan,
        last_inventory: InventorySnapshot,
        last_traffic: TrafficSnapshot,
        last_replan_turn: u32,
    ) -> Self {
        Self { last_plan, last_inventory, last_traffic, last_replan_turn }
    }

    /// Build a placeholder state for turn 0 (empty plan, empty snapshots,
    /// `last_replan_turn` = 0). After the first real solve the engine should
    /// overwrite all fields with concrete values.
    #[must_use]
    pub fn initial() -> Self {
        Self {
            last_plan: Plan { plans: Vec::new(), variety: 0 },
            last_inventory: InventorySnapshot::default(),
            last_traffic: TrafficSnapshot::default(),
            last_replan_turn: 0,
        }
    }
}

/// Dynamic replanner. Wraps an inner [`Solver`] and decides — between
/// consecutive turns — whether to reuse the cached plan or re-cluster /
/// re-assign.
///
/// The three thresholds are exposed as direct fields per the issue spec:
/// - `inventory_threshold`: minimum [`InventorySnapshot::diff`] to retrigger.
/// - `traffic_threshold`:   minimum [`TrafficSnapshot::diff`] to retrigger.
/// - `min_replan_interval_turns`: minimum number of turns between two
///   consecutive replans, regardless of how severe the drift looks. This
///   protects against replanning every turn under noisy signals.
pub struct DynamicReplanner<S: Solver = SeriesClusteredSolver> {
    inner: S,
    pub inventory_threshold: f64,
    pub traffic_threshold: f64,
    pub min_replan_interval_turns: u32,
}

impl<S: Solver> DynamicReplanner<S> {
    /// Build a replanner wrapping `inner` with the supplied thresholds.
    /// Thresholds must be non-negative; callers are expected to validate
    /// before construction if they need to reject bad configs.
    #[must_use]
    pub fn new(
        inner: S,
        inventory_threshold: f64,
        traffic_threshold: f64,
        min_replan_interval_turns: u32,
    ) -> Self {
        Self { inner, inventory_threshold, traffic_threshold, min_replan_interval_turns }
    }

    /// Return `true` iff:
    /// 1. at least `min_replan_interval_turns` have elapsed since
    ///    `state.last_replan_turn`, AND
    /// 2. either the inventory diff or the traffic diff exceeds its
    ///    corresponding threshold.
    ///
    /// The function is pure with respect to `&self`, `state`, and the
    /// snapshots; no field on `state` is mutated by the check itself. That
    /// lets callers use it as a guard before deciding to replan.
    #[must_use]
    pub fn should_replan(
        &self,
        state: &PlanState,
        current_turn: u32,
        inventory: &InventorySnapshot,
        traffic: &TrafficSnapshot,
    ) -> bool {
        let interval = current_turn.saturating_sub(state.last_replan_turn);
        if interval < self.min_replan_interval_turns {
            return false;
        }
        let inv_diff = inventory.diff(&state.last_inventory);
        let traffic_diff = traffic.diff(&state.last_traffic);
        inv_diff > self.inventory_threshold || traffic_diff > self.traffic_threshold
    }

    /// Run the planner for one turn.
    ///
    /// If [`Self::should_replan`] returns `true`, the inner solver is
    /// re-invoked to re-cluster and re-assign, and `state` is updated in
    /// place with the new plan and the current snapshots. Otherwise the
    /// cached `state.last_plan` is returned untouched.
    pub fn solve_with_state(
        &self,
        state: &mut PlanState,
        request: SolveRequest,
        current_turn: u32,
        inventory: &InventorySnapshot,
        traffic: &TrafficSnapshot,
    ) -> SolveResponse {
        if self.should_replan(state, current_turn, inventory, traffic) {
            let resp = self.inner.solve(request);
            state.last_plan = resp.clone();
            state.last_inventory.clone_from(inventory);
            state.last_traffic.clone_from(traffic);
            state.last_replan_turn = current_turn;
            resp
        } else {
            state.last_plan.clone()
        }
    }

    /// Reference to the inner solver. Exposed so callers can introspect or
    /// unit-test the wrapped type without re-wrapping.
    #[must_use]
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl DynamicReplanner<SeriesClusteredSolver> {
    /// Build a replanner with sensible defaults:
    /// `inventory_threshold = 0.1` (10% of stock shifted), `traffic_threshold
    /// = 0.2` (20% of congestion surface moved), `min_replan_interval_turns =
    /// 1` (allow replanning every turn once drift is detected).
    #[must_use]
    pub fn with_default_config() -> Self {
        Self::new(SeriesClusteredSolver::with_default_config(), 0.1, 0.2, 1)
    }
}

/// `Solver` impl that delegates straight to the inner solver. The replan
/// logic is opt-in via [`DynamicReplanner::solve_with_state`]; this impl
/// exists so the type can be passed wherever `&dyn Solver` is expected and
/// keeps the engine seam (`Solver::solve`) untouched.
impl<S: Solver> Solver for DynamicReplanner<S> {
    fn solve(&self, request: SolveRequest) -> SolveResponse {
        self.inner.solve(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < EPS, "expected {expected}, got {actual}");
    }

    #[test]
    fn inventory_diff_is_zero_on_identical_snapshots() {
        let a = InventorySnapshot::from_pairs([(SpotId::new("s1"), 3), (SpotId::new("s2"), 0)]);
        let b = a.clone();
        assert_close(a.diff(&b), 0.0);
    }

    #[test]
    fn inventory_diff_handles_missing_keys_in_both_directions() {
        let a = InventorySnapshot::from_pairs([(SpotId::new("s1"), 5)]);
        let b = InventorySnapshot::from_pairs([(SpotId::new("s2"), 5)]);
        // L1 diff = 10 (both keys swap to/from zero), old total = 5 → 2.0.
        assert_close(a.diff(&b), 2.0);
    }

    #[test]
    fn inventory_diff_is_one_when_other_is_empty_and_self_is_nonempty() {
        let a = InventorySnapshot::from_pairs([(SpotId::new("s1"), 5)]);
        let b = InventorySnapshot::default();
        assert_close(a.diff(&b), 1.0);
    }

    #[test]
    fn traffic_diff_is_zero_when_both_snapshots_are_empty() {
        let a = TrafficSnapshot::default();
        let b = TrafficSnapshot::default();
        assert_close(a.diff(&b), 0.0);
    }

    #[test]
    fn traffic_diff_is_one_when_only_old_snapshot_has_data() {
        let a = TrafficSnapshot::default();
        let b = TrafficSnapshot::from_pairs([((HexCoord::new(0, 0), HexCoord::new(1, 0)), 4)]);
        // diff = 4, max = 4 → 1.0.
        assert_close(a.diff(&b), 1.0);
    }

    #[test]
    fn traffic_diff_is_partial_for_localised_change() {
        let a = TrafficSnapshot::from_pairs([
            ((HexCoord::new(0, 0), HexCoord::new(1, 0)), 4),
            ((HexCoord::new(1, 0), HexCoord::new(2, 0)), 4),
            ((HexCoord::new(2, 0), HexCoord::new(3, 0)), 4),
            ((HexCoord::new(3, 0), HexCoord::new(4, 0)), 4),
        ]);
        // One of the four edges degrades from 4 → 0.
        let b = TrafficSnapshot::from_pairs([
            ((HexCoord::new(0, 0), HexCoord::new(1, 0)), 4),
            ((HexCoord::new(1, 0), HexCoord::new(2, 0)), 0),
            ((HexCoord::new(2, 0), HexCoord::new(3, 0)), 4),
            ((HexCoord::new(3, 0), HexCoord::new(4, 0)), 4),
        ]);
        // L1 diff = 4; max total = max(4, 4) summed = 16 → 0.25.
        assert_close(a.diff(&b), 0.25);
    }
}
