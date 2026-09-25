//! `greedy` candidate — the Phase-1 closest-spot / variety-first greedy from
//! `crates/solver-greedy`.
//!
//! A deliberately different *algorithm* from `loop` (greedy assignment vs a
//! refined tour), so the corpus demonstrates cross-algorithm comparison and
//! not just a parameter sweep of one solver.
//!
//! # How the adapter works
//!
//! `solver_greedy::GreedySolver::decide` answers one question per step:
//! *"which spot should each agent head for right now?"* — returning intents
//! (`MoveToward { agent, spot }` / `Stay`), not concrete hex directions.
//!
//! The contest asks for one day's worth of concrete actions per answer
//! (Q&A その2 A23/A24), so this adapter asks greedy once at the start of the
//! day and then walks each agent toward its assigned spot with
//! [`super::walk::plan_walk`] for the rest of the day.
//!
//! That is a deliberate, documented simplification: greedy's per-step
//! re-evaluation would need a second simulation of how stock and positions
//! evolve inside the day, which is exactly the engine's job. Re-deriving it
//! here would duplicate [`engine::MatchSim`] and make the two drift. Greedy
//! therefore contributes its *assignment policy*; the engine contributes the
//! *outcome*.
//!
//! The `SolverContext` → `solver_greedy` conversion is shared with the other
//! greedy-family candidate via [`super::context`], so `greedy` and `nearest`
//! differ only in assignment policy.

use engine::model::{AgentActions, HexCoord, Solver, SolverContext};
use solver_greedy::{GreedySolver, Solver as _};

use super::context;
use super::walk::plan_walk;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};

/// Parameter schema. Empty today — `GreedySolver` exposes no knobs — but the
/// id grammar already carries them, so adding one later does not change the
/// candidate-id format.
pub const PARAMS: &[ParamSpec] = &[];

/// Greedy candidate: assigns each agent a target spot for the day, then walks.
pub struct GreedyCandidate {
    inner: GreedySolver,
}

impl GreedyCandidate {
    pub(crate) fn new() -> Self {
        Self { inner: GreedySolver::new() }
    }

    /// Greedy's target spot per controlled agent, one entry per position in
    /// `ctx.agent_ids` (empty when the agent has no target).
    ///
    /// Extracted from [`Solver::decide`] so the `astar` candidate can reuse
    /// the exact same variety-aware nearest assignment and differ only in
    /// *routing* — the comparison then isolates path quality. Built on the
    /// shared greedy-family conversion in [`super::context`], so `greedy`,
    /// `nearest` and `astar` all see identical inputs.
    pub(crate) fn targets(&self, ctx: &SolverContext<'_>) -> Vec<Vec<HexCoord>> {
        let gctx = context::solve_context(ctx);
        let intents = self.inner.decide(&gctx);
        context::targets_from_intents(ctx, &intents)
    }
}

impl Solver for GreedyCandidate {
    fn name(&self) -> &'static str {
        "greedy"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        plan_walk(ctx, &self.targets(ctx))
    }
}

/// Factory for [`GreedyCandidate`].
pub struct GreedyFactory;

impl CandidateFactory for GreedyFactory {
    fn algorithm(&self) -> &'static str {
        "greedy"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn describe(&self) -> &'static str {
        "Phase-1 greedy: each agent heads for the closest unclaimed spot, \
         tie-broken by variety first. A distinct algorithm from `loop`."
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        Ok(Box::new(GreedyCandidate::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateRegistry;

    #[test]
    fn factory_accepts_bare_greedy_and_rejects_unknown_params() {
        let reg = CandidateRegistry::builtin();
        assert!(reg.build("greedy").is_ok());
        assert!(matches!(reg.build("greedy:beam=4"), Err(CandidateError::UnknownParameter { .. })));
    }
}
