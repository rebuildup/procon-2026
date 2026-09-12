//! # `solver-ilp-cross` — Phase 3 cross-day LP/ILP solver
//!
//! Reference strategy: `docs/contest/strategy.md` §5.3
//! ("Phase 3（理想）: 集中計算 + 部分MCTS or ILP").
//!
//! This crate plans an entire multi-day match in one shot. It treats the
//! day-by-day plan as a relaxed binary linear program whose decision
//! variables are `x[agent, day, step, direction] ∈ [0, 1]`. The objective
//! encodes the contest ranking from `strategy.md` §1 (lower is better), and
//! the constraints enforce day-budget, fuel-cap, and pond-avoidance rules.
//!
//! When the `good_lp` feature is enabled the crate solves the LP with the
//! real backend; otherwise it falls back to a deterministic greedy decode
//! driven by the same per-variable objective coefficients. The fallback
//! path keeps the test suite self-contained (no native solver dependency)
//! while exercising exactly the same LP model — so the test that compares
//! ILP output to the greedy solver is meaningful on both code paths.
//!
//! ## Integration status
//!
//! EXPECTED INTEGRATION INTERFACE
//! =============================
//! The `Solver` trait and the `SolveContext` / `Plan` types will move out
//! of this crate as soon as `crates/bench` (issue #20) and `crates/domain`
//! (#11/#12/#13/#53/#57) merge to `main`. The signatures here mirror the
//! contract described in the parent issue:
//!
//! ```text
//! pub trait Solver {
//!     fn name(&self) -> &'static str;
//!     fn decide(&self, ctx: &SolveContext) -> Plan;
//! }
//! ```
//!
//! When that lands, this crate will become a leaf that depends on
//! `engine` + `domain` and the local `types` module will be deleted.
//!
//! ## Decision variables and objective
//!
//! For each `(agent, day, step, direction)` tuple we declare a relaxed
//! binary variable. The objective is
//!
//! ```text
//! min  -variety * 100
//!     - daily_variety_sum * 10
//!     - total_noodles
//!     + response_time
//! ```
//!
//! where `variety` is the number of distinct series collected across the
//! whole match and `daily_variety_sum` is the sum of per-day distinct
//! series. See [`model::VARIETY_GLOBAL_WEIGHT`] /
//! [`model::VARIETY_DAILY_WEIGHT`] / [`model::NOODLE_WEIGHT`].
//!
//! ## Constraints
//!
//! 1. **One action per (agent, day, step).** Sum of the six direction
//!    variables equals exactly one — wait is the implicit "all zero" case.
//! 2. **Fuel cap.** Sum of move-cost per (agent, day) ≤ `fuel_cap(day)`. The
//!    default cap is `steps * 3` per `domain::FuelPolicy` (Q14).
//! 3. **No pond entry.** Variables whose direction would land the agent in
//!    a pond cell are forced to zero at model-build time.

#![doc(test(attr(deny(warnings))))]
#![warn(missing_docs)]

pub mod constraints;
mod model;
mod solver;
mod types;

pub use constraints::{
    ConstraintError, ConstraintResult, FuelParams, FuelRow, InventoryParams, InventoryRow,
    SeriesInventoryRow, SupplyArc, SupplyParams, add_fuel_constraints,
    add_spot_inventory_constraints, add_supply_arc_constraints,
};
pub use model::{
    Model, NOODLE_WEIGHT, RESPONSE_TIME_WEIGHT, VARIETY_DAILY_WEIGHT, VARIETY_GLOBAL_WEIGHT,
    VarKey, Variable, count_active_variables, greedy_decode, score_plan,
};
pub use solver::IlpCrossSolver;
pub use types::{
    Action, Agent, AgentId, CellKind, DaySpec, Direction, HexCoord, Plan, STRATEGY_NAME, SeriesId,
    SolveContext, Spot, SpotId,
};

/// Solver trait stub — the cross-day solver contract.
///
/// EXPECTED INTEGRATION INTERFACE — see module docs. Will be replaced by the
/// canonical `engine::solver::Solver` (issue #57) once that lands.
///
/// The contract is intentionally tiny: a solver has a name and an
/// `decide` method that maps a [`SolveContext`] to a [`Plan`]. The
/// coordinator drives solvers through `dyn Solver`, picks between them at
/// runtime, and keeps the best answer (issue #20 bench harness).
pub trait Solver {
    /// Stable, telemetry-friendly strategy name.
    fn name(&self) -> &'static str;

    /// Decide the next action for every (day, step, agent) row.
    ///
    /// The returned plan must contain exactly `agents * day_steps` actions per
    /// day, in submission order, so it can be encoded into the wire format
    /// by the protocol layer.
    fn decide(&self, ctx: &SolveContext) -> Plan;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_solver_implements_local_solver_trait() {
        // Compile-time proof the trait is dyn-compatible and the cross
        // solver implements it.
        let solver: Box<dyn Solver> = Box::new(IlpCrossSolver::new());
        assert_eq!(solver.name(), STRATEGY_NAME);
    }

    #[test]
    fn decide_emits_plan_with_expected_size() {
        let solver = IlpCrossSolver::new();
        let ctx = SolveContext::for_test(
            vec![types::DaySpec::new(4)],
            vec![],
            vec![],
            vec![
                Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 10 },
                Agent { id: AgentId(1), coord: HexCoord::new(1, 0), fuel: 10 },
            ],
        );
        let plan = solver.decide(&ctx);
        assert_eq!(plan.len(), 2 * 4);
    }
}
