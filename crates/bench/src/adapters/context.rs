//! Shared conversion from the engine's live [`SolverContext`] to
//! `solver_greedy`'s [`SolveContext`].
//!
//! Every adapter here that drives a solver from `crates/solver-greedy` asks
//! the engine the same question — *where are my agents, and which spots still
//! have stock today?* — and turns the answers into the same view of the day.
//! Keeping that conversion in one place is what makes two greedy-family
//! candidates comparable: `greedy` and `nearest` see identical spots, in the
//! identical order, with identical ids, and differ **only** in their
//! assignment policy. An evaluation gap therefore cannot come from a subtly
//! different input conversion.

use engine::model::{HexCoord, MapGrid, SolverContext};
use solver_greedy::{
    Action as GreedyAction, Agent as GreedyAgent, AgentId as GreedyAgentId,
    HexCoord as GreedyCoord, SolveContext as GreedyContext, Spot as GreedySpot,
    SpotId as GreedySpotId,
};

/// Build the greedy-family solver's view of the world from the live match
/// state at day start.
///
/// Spot order follows `map.cells()` iteration and agent order follows
/// `ctx.agent_ids`, so the resulting decision is deterministic for a given
/// state — which is what lets the evaluator claim run-to-run reproducibility.
pub(crate) fn solve_context(ctx: &SolverContext<'_>) -> GreedyContext {
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

/// Turn one day's worth of *intents* into [`super::walk::plan_walk`]'s
/// target list: one target per controlled agent, in `ctx.agent_ids` order.
///
/// An agent whose intent is missing or is a `Stay` gets an empty target list,
/// which [`super::walk::plan_walk`] turns into waiting all day.
pub(crate) fn targets_from_intents(
    ctx: &SolverContext<'_>,
    intents: &[GreedyAction],
) -> Vec<Vec<HexCoord>> {
    let map = &ctx.state.setting.map;
    ctx.agent_ids
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
        .collect()
}

/// Spot ids are packed `row * 32 + col`, which is unique for every contest
/// map (`MapSize::MAX = 32`) and fits greedy's `SpotId(u16)`.
pub(crate) fn spot_id_for_coord(coord: HexCoord) -> u16 {
    let id = coord.row * 32 + coord.col;
    u16::try_from(id).unwrap_or(0)
}

/// Inverse of [`spot_id_for_coord`]; `None` when the id does not name a cell
/// that actually carries a spot on `map`.
pub(crate) fn coord_for_spot_id(spot_id: u16, map: &MapGrid) -> Option<HexCoord> {
    let id = i32::from(spot_id);
    let row = id / 32;
    let col = id % 32;
    let coord = HexCoord::new(col, row);
    map.get(coord).filter(|c| c.spot.is_some()).map(|_| coord)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_ids_round_trip_within_contest_bounds() {
        for (col, row) in [(0, 0), (5, 9), (31, 31), (7, 3)] {
            let c = HexCoord::new(col, row);
            assert_eq!(coord_for_spot_id(spot_id_for_coord(c), &dummy_map()), Some(c));
        }
    }

    #[test]
    fn spot_ids_are_unique_across_the_whole_map() {
        let mut seen = std::collections::HashSet::new();
        for row in 0..32 {
            for col in 0..32 {
                assert!(seen.insert(spot_id_for_coord(HexCoord::new(col, row))));
            }
        }
    }

    /// A 32x32 map with a spot on every cell, so spot-id round-tripping can
    /// be checked at the contest's size bound.
    fn dummy_map() -> MapGrid {
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
