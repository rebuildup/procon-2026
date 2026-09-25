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
//! The shared half of that contract (world view, spot-id packing, target
//! extraction) lives in [`super::context`]; this module owns only greedy's
//! context construction.

use engine::model::{AgentActions, Solver, SolverContext};
use solver_greedy::{GreedySolver, SolveContext as GreedyContext, Solver as _};

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
    fn new() -> Self {
        Self { inner: GreedySolver::new() }
    }
}

impl Solver for GreedyCandidate {
    fn name(&self) -> &'static str {
        "greedy"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let gctx = greedy_context(ctx);
        let intents = self.inner.decide(&gctx);
        plan_walk(ctx, &context::day_targets(ctx, &intents))
    }
}

/// Build greedy's view of the world from the live match state at day start.
fn greedy_context(ctx: &SolverContext<'_>) -> GreedyContext {
    let agents = context::agents(ctx);
    let spots = context::spots(&ctx.state.setting.map);
    GreedyContext::for_test(0, agents, spots)
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
    use engine::model::HexCoord;

    #[test]
    fn factory_accepts_bare_greedy_and_rejects_unknown_params() {
        let reg = CandidateRegistry::builtin();
        assert!(reg.build("greedy").is_ok());
        assert!(matches!(reg.build("greedy:beam=4"), Err(CandidateError::UnknownParameter { .. })));
    }

    #[test]
    fn greedy_decides_well_shaped_actions_for_every_controlled_agent() {
        use domain::{MatchId, PlayerId};
        use engine::model::{Agent, MapGrid, MapSize, MatchSetting, MatchState, Spot, Terrain};

        let size = MapSize::new(8, 8).unwrap();
        let cells = (0..8)
            .flat_map(|row| {
                (0..8).map(move |col| {
                    let coord = HexCoord::new(col, row);
                    let spot = if (col, row) == (2, 0) {
                        Some(Spot { series: 1, pos: coord, max_stock: 3, current_stock: 3 })
                    } else {
                        None
                    };
                    engine::model::Cell { coord, terrain: Terrain::Plain, spot }
                })
            })
            .collect();
        let map = MapGrid::new(size, cells).unwrap();
        let mut agents = vec![Agent::patrol(0, HexCoord::new(0, 0), 4, 3)];
        while agents.len() < 3 {
            let id = u8::try_from(agents.len()).unwrap_or(u8::MAX);
            agents.push(Agent::supply(id, HexCoord::new(7, 7)));
        }
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("greedy-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents,
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        let actions = GreedyCandidate::new().decide(&ctx);
        assert!(
            actions.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
            "greedy must produce a well-shaped day plan"
        );
    }
}
