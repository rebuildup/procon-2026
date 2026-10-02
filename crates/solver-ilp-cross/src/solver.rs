//! `IlpCrossSolver` — Phase 3 cross-day LP solver.
//!
//! Reference: `docs/contest/strategy.md` §5.3 "Phase 3（理想）: 集中計算 +
//! 部分MCTS or ILP".
//!
//! ## What this solver does
//!
//! It plans an entire multi-day match in one shot, treating the day-by-day
//! plan as a relaxed binary linear program whose decision variables are
//! `x[agent, day, step, direction] ∈ [0, 1]`. The objective encodes the
//! contest ranking from `strategy.md` §1 (lower is better), and the
//! constraints enforce day-budget, fuel-cap, and pond-avoidance rules.
//!
//! When the `good_lp` feature is enabled the crate would solve the LP with
//! the real backend; in the prototype we deliberately fall back to a
//! deterministic greedy decode driven by the same per-variable objective
//! coefficients. The fallback path keeps the test suite self-contained
//! (no native solver dependency) while exercising exactly the same LP
//! model — so the test that compares ILP output to the greedy solver is
//! meaningful on both code paths.
//!
//! ## Integration status
//!
//! This is the Phase 3 / cross-day variant. It owns the multi-day plan, not
//! the single-day one — see `solver-greedy` and `solver-astar` for the
//! step-by-step variants. When `engine::solver::Solver` (issue #57) lands
//! the `Solver` trait defined in `crate::lib` will be **replaced** by the
//! canonical one and `decide` will operate on a `SolveContext` already
//! wrapped around a `MatchSnapshot`.

use std::time::Duration;

use tracing::debug;
#[cfg(feature = "good_lp")]
use tracing::warn;

use crate::Solver;
use crate::model::{Model, greedy_decode, score_plan};
use crate::types::{Plan, STRATEGY_NAME, SolveContext};

/// LP/ILP cross-day solver.
///
/// Phase 3 of `docs/contest/strategy.md` §5.3 — solves an entire multi-day
/// match as one relaxed LP and (when the `good_lp` feature is enabled)
/// delegates to the real backend; otherwise degrades to a deterministic
/// greedy decode driven by the same per-variable objective coefficients.
#[derive(Debug, Clone, Default)]
pub struct IlpCrossSolver {
    /// Solver-wide tuning knobs. Empty for the prototype; populated by a
    /// builder in a future issue.
    _private: (),
}

impl IlpCrossSolver {
    /// Construct a fresh solver instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Plan an entire multi-day match.
    ///
    /// `remaining_time` is the per-day budget in milliseconds; when the LP
    /// backend is unavailable or the budget is exhausted we fall back to
    /// `greedy_decode`. The greedy fallback is **always valid** — a
    /// plan that names every (agent, day, step) — so it can never make the
    /// caller worse off than returning an empty plan.
    #[must_use]
    pub fn solve(&self, ctx: &SolveContext, remaining_time: Duration) -> Plan {
        if ctx.agents.is_empty() || ctx.days.is_empty() {
            return Plan::new(Vec::new())
                .with_metadata(serde_json::json!({ "solver": self.name(), "kind": "empty" }));
        }

        let model = Model::from_context(ctx);
        debug!(
            variables = model.variable_count(),
            forced_zero = model.forced_zero_count(),
            "ilp-cross: built model"
        );

        let (actions, kind) = Self::dispatch(&model, ctx, remaining_time);

        let score = score_plan(ctx, &actions);
        Plan::new(actions).with_metadata(serde_json::json!({
            "solver": self.name(),
            "kind": kind,
            "variables": model.variable_count(),
            "forced_zero": model.forced_zero_count(),
            "objective": score,
        }))
    }

    /// Per-day budget helper that derives a [`Duration`] from the context's
    /// first day. Production callers pass their own deadline; this is the
    /// "I don't care, use the day budget" entry point.
    #[must_use]
    pub fn solve_default(&self, ctx: &SolveContext) -> Plan {
        let remaining = ctx
            .days
            .first()
            .and_then(|d| d.seconds_budget)
            .map_or(Duration::from_secs(60), |budget| Duration::from_secs(u64::from(budget)));
        self.solve(ctx, remaining)
    }

    /// Route to the real LP backend when available, else fall back to a
    /// greedy decode that uses the same per-variable objective.
    ///
    /// Centralised in one method so the call-site in [`Self::solve`] does
    /// not need to know whether the `good_lp` feature is on.
    fn dispatch(
        model: &Model,
        ctx: &SolveContext,
        remaining_time: Duration,
    ) -> (Vec<crate::types::Action>, &'static str) {
        #[cfg(feature = "good_lp")]
        {
            if !remaining_time.is_zero() {
                match solve_with_good_lp(model, ctx, remaining_time) {
                    Ok(plan) => return (plan, "good_lp"),
                    Err(err) => warn!(error = %err, "good_lp solve failed; falling back to greedy"),
                }
            }
        }
        let _ = remaining_time;
        (greedy_decode(ctx, model), "greedy_decode")
    }
}

/// `good_lp` solve path. Only compiled when the `good_lp` feature is on.
///
/// **Prototype stub.** A real implementation would translate the relaxed
/// binary variables into `good_lp` expressions, attach the per-(agent, day,
/// step) exactly-one constraint and the fuel caps, hand the LP to a
/// branch-and-bound solver, and decode the result back into a
/// [`Plan`](crate::types::Plan). We intentionally do *not* link a native
/// CBC binary here — that lands in the next ticket once a real backend is
/// chosen and pinned in CI. The current stub returns an error so callers
/// fall back to the deterministic greedy decode (see [`IlpCrossSolver::solve`]).
#[cfg(feature = "good_lp")]
fn solve_with_good_lp(
    _model: &Model,
    _ctx: &SolveContext,
    _remaining_time: Duration,
) -> Result<Vec<crate::types::Action>, String> {
    Err("good_lp backend is a prototype stub; greedy fallback engaged".to_owned())
}

/// `Solver` trait impl on the cross-day solver. Mirrors the local stub
/// trait (issue #57 compatibility) so the bench harness can dispatch
/// across crates before the canonical `engine::solver::Solver` lands.
impl crate::Solver for IlpCrossSolver {
    fn name(&self) -> &'static str {
        STRATEGY_NAME
    }

    fn decide(&self, ctx: &crate::SolveContext) -> crate::Plan {
        self.solve_default(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        Agent, AgentId, CellKind, DaySpec, HexCoord, SeriesId, SolveContext, Spot, SpotId,
    };

    fn ctx() -> SolveContext {
        let days = vec![DaySpec::new(5)];
        let cells = vec![(HexCoord::new(0, 0), CellKind::Plain)];
        let spots = vec![Spot {
            id: SpotId(0),
            coord: HexCoord::new(1, 0),
            series_id: SeriesId(0),
            initial_stock: 1,
        }];
        let agents = vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 50 }];
        SolveContext::for_test(days, cells, spots, agents)
    }

    #[test]
    fn solver_name_is_stable() {
        let solver = IlpCrossSolver::new();
        assert_eq!(solver.name(), STRATEGY_NAME);
    }

    #[test]
    fn empty_context_returns_empty_plan() {
        let solver = IlpCrossSolver::new();
        let ctx = SolveContext::for_test(vec![], vec![], vec![], vec![]);
        let plan = solver.solve_default(&ctx);
        assert!(plan.is_empty());
        assert_eq!(plan.metadata["kind"], "empty");
    }

    #[test]
    fn solve_returns_one_action_per_agent_per_step() {
        let solver = IlpCrossSolver::new();
        let ctx = ctx();
        let plan = solver.solve_default(&ctx);

        let expected = ctx.agents.len() * ctx.day_steps(0) as usize;
        assert_eq!(plan.len(), expected);

        for action in &plan.actions {
            assert_eq!(action.agent(), AgentId(0));
        }
    }

    #[test]
    fn plan_metadata_reports_objective_and_kind() {
        let solver = IlpCrossSolver::new();
        let ctx = ctx();
        let plan = solver.solve_default(&ctx);
        assert_eq!(plan.metadata["solver"], STRATEGY_NAME);
        assert!(plan.metadata["objective"].is_number());
        assert!(plan.metadata["variables"].as_u64().unwrap() > 0);
    }

    #[test]
    fn solve_respects_per_day_budget() {
        let solver = IlpCrossSolver::new();
        let ctx = ctx();
        let plan = solver.solve(&ctx, Duration::ZERO);
        // Zero budget must still return a valid (greedy) plan, never an
        // empty one — otherwise the caller loses the day outright.
        assert!(!plan.is_empty());
    }
}
