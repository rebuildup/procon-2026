//! # `solver-greedy` — Phase 1 貪欲法ソルバ (greedy solver skeleton)
//!
//! Reference strategy: `docs/contest/strategy.md` §5.3
//! ("Phase 1（短期・予選）: 貪欲法").
//!
//! Per the strategy note, Phase 1 of our solver ladder is the simplest
//! possible greedy:
//!
//! > Each step, head to the closest unclaimed spot. Tie-break by variety
//! > first (so we maximise `distinct series` across the day — the top
//! > scoring criterion). No look-ahead, no pathfinding, no day-rollover
//! > optimisation. Must run in real time (under one step-budget worth of
//! > wall-clock per step).
//!
//! This crate ships the **skeleton** of that strategy plus a deterministic
//! `Solver` trait so that the bench harness (issue #20) can already drive
//! it. The actual decision logic is intentionally tiny — the real value of
//! this crate today is the integration surface and the smoke test. Heavier
//! heuristics land in subsequent issues (#23, #24).
//!
//! ## Integration status
//!
//! EXPECTED INTEGRATION INTERFACE
//! =============================
//! The `Solver` trait and the `Action` / `SolveContext` types will move out
//! of this crate as soon as `crates/bench` (issue #20) and `crates/domain`
//! (#11/#12/#13/#53) merge to `main`. The signatures here mirror the
//! contract described in the parent issue:
//!
//! ```text
//! pub trait Solver {
//!     fn name(&self) -> &'static str;
//!     fn decide(&self, ctx: &SolveContext) -> Vec<Action>;
//! }
//! ```
//!
//! When that lands, this crate will become a leaf that depends on
//! `domain` + `bench` and the local `types` module will be deleted. Until
//! then, this stub trait keeps the crate self-contained.

#![doc(test(attr(deny(warnings))))]
#![warn(missing_docs)]

mod greedy;
mod types;

pub use greedy::GreedySolver;
pub use types::{Action, Agent, AgentId, HexCoord, SolveContext, Spot, SpotId};

/// Strategy descriptor, used by the bench harness (issue #20) to label
/// reports. Stable for telemetry.
pub const STRATEGY_NAME: &str = "greedy-phase1-variety-first";

/// Solver trait stub.
///
/// EXPECTED INTEGRATION INTERFACE — see module docs. Will be replaced by the
/// canonical version in `crates/bench` once issue #20 merges.
pub trait Solver {
    /// Stable, telemetry-friendly strategy name.
    fn name(&self) -> &'static str;

    /// Decide the next action for every agent we control.
    ///
    /// The returned vector must contain exactly one [`Action`] per agent in
    /// `ctx.agents`, in the same order.
    fn decide(&self, ctx: &SolveContext) -> Vec<Action>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solver_name_is_stable() {
        let s = GreedySolver::new();
        assert_eq!(s.name(), STRATEGY_NAME);
    }

    #[test]
    fn decide_emits_one_action_per_agent() {
        let ctx = SolveContext::for_test(
            0,
            vec![
                Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 10 },
                Agent { id: AgentId(1), coord: HexCoord::new(1, 0), fuel: 10 },
            ],
            vec![Spot { id: SpotId(0), coord: HexCoord::new(0, 1), series_id: 0, stock: 1 }],
        );
        let actions = GreedySolver::new().decide(&ctx);
        assert_eq!(actions.len(), ctx.agents.len());
        assert_eq!(actions[0].agent(), AgentId(0));
        assert_eq!(actions[1].agent(), AgentId(1));
    }
}
