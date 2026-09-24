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

use engine::model::{AgentActions, HexCoord, Solver, SolverContext};
use solver_greedy::{
    Action as GreedyAction, Agent as GreedyAgent, AgentId as GreedyAgentId, GreedySolver,
    HexCoord as GreedyCoord, SolveContext as GreedyContext, Solver as _, Spot as GreedySpot,
    SpotId as GreedySpotId,
};

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
        let map = &ctx.state.setting.map;

        // One target per controlled agent, in `ctx.agent_ids` order.
        let targets: Vec<Vec<HexCoord>> = ctx
            .agent_ids
            .iter()
            .map(|id| {
                let target = intents.iter().find_map(|a| match a {
                    GreedyAction::MoveToward { agent, spot } if agent.0 == *id => {
                        coord_for_spot_id(spot.0, map)
                    }
                    _ => None,
                });
                target.into_iter().collect()
            })
            .collect();

        plan_walk(ctx, &targets)
    }
}

/// Build greedy's view of the world from the live match state at day start.
fn greedy_context(ctx: &SolverContext<'_>) -> GreedyContext {
    let map = &ctx.state.setting.map;
    let spots: Vec<GreedySpot> = map
        .cells()
        .iter()
        .filter_map(|cell| {
            let s = cell.spot.as_ref()?;
            Some(GreedySpot {
                id: GreedySpotId::new(spot_id_for_coord(cell.coord)),
                coord: GreedyCoord::new(cell.coord.col, cell.coord.row),
                series_id: u16::try_from(s.series).unwrap_or(u16::MAX),
                stock: s.current_stock,
            })
        })
        .collect();
    let agents: Vec<GreedyAgent> = ctx
        .agent_ids
        .iter()
        .filter_map(|id| {
            let a = ctx.state.agents.iter().find(|a| a.id == *id)?;
            Some(GreedyAgent {
                id: GreedyAgentId::new(*id),
                coord: GreedyCoord::new(a.pos.col, a.pos.row),
                fuel: a.fuel,
            })
        })
        .collect();
    GreedyContext::for_test(0, agents, spots)
}

/// Spot ids are packed `row * 32 + col`, which is unique for every contest
/// map (`MapSize::MAX = 32`) and fits greedy's `SpotId(u16)`.
fn spot_id_for_coord(coord: HexCoord) -> u16 {
    let id = coord.row * 32 + coord.col;
    u16::try_from(id).unwrap_or(0)
}

fn coord_for_spot_id(spot_id: u16, map: &engine::model::MapGrid) -> Option<HexCoord> {
    let id = i32::from(spot_id);
    let row = id / 32;
    let col = id % 32;
    let coord = HexCoord::new(col, row);
    map.get(coord).filter(|c| c.spot.is_some()).map(|_| coord)
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

    #[test]
    fn spot_ids_round_trip_within_contest_bounds() {
        for (col, row) in [(0, 0), (5, 9), (31, 31), (7, 3)] {
            let c = HexCoord::new(col, row);
            assert_eq!(coord_for_spot_id(spot_id_for_coord(c), &dummy_map()), Some(c));
        }
    }

    /// A 32x32 map with a spot on every cell, so spot-id round-tripping can
    /// be checked at the contest's size bound.
    fn dummy_map() -> engine::model::MapGrid {
        use engine::model::{MapGrid, MapSize, Spot, Terrain};
        let size = MapSize::new(32, 32).unwrap();
        let cells = (0..32)
            .flat_map(|row| {
                (0..32).map(move |col| {
                    let coord = HexCoord::new(col, row);
                    engine::model::Cell {
                        coord,
                        terrain: Terrain::Plain,
                        spot: Some(Spot { series: 1, pos: coord, max_stock: 1, current_stock: 1 }),
                    }
                })
            })
            .collect();
        MapGrid::new(size, cells).unwrap()
    }
}
