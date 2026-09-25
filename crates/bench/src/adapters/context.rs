//! Shared conversion between the live [`SolverContext`] and the local stub
//! types every `solver-greedy` solver understands.
//!
//! The greedy family (`greedy`, `fuelgreedy`, …) differs only in its
//! *assignment policy*. Everything else is identical:
//!
//! - the world view — one [`solver_greedy::Spot`] per spot cell, one
//!   [`solver_greedy::Agent`] per controlled agent, packed spot ids,
//! - the day contract — solve once at day start, turn the resulting intents
//!   into per-agent target lists, then walk with
//!   [`super::walk::plan_walk`].
//!
//! Keeping that shared half here means a new greedy-family candidate only
//! adds its own `SolveContext` construction and scoring rule, and cannot
//! silently drift from the others on spot-id packing or target ordering.

use engine::model::{HexCoord, MapGrid, SolverContext};
use solver_greedy::{
    Action as GreedyAction, Agent as GreedyAgent, AgentId as GreedyAgentId,
    HexCoord as GreedyCoord, Spot as GreedySpot, SpotId as GreedySpotId,
};

/// Spot ids are packed `row * 32 + col`, which is unique for every contest
/// map (`MapSize::MAX = 32`) and fits greedy's `SpotId(u16)`.
pub(crate) fn spot_id_for_coord(coord: HexCoord) -> u16 {
    let id = coord.row * 32 + coord.col;
    u16::try_from(id).unwrap_or(0)
}

/// Inverse of [`spot_id_for_coord`]: unpack and confirm the cell really
/// carries a spot.
pub(crate) fn coord_for_spot_id(spot_id: u16, map: &MapGrid) -> Option<HexCoord> {
    let id = i32::from(spot_id);
    let row = id / 32;
    let col = id % 32;
    let coord = HexCoord::new(col, row);
    map.get(coord).filter(|c| c.spot.is_some()).map(|_| coord)
}

/// Every spot cell on the map, in row-major order, with its live stock.
pub(crate) fn spots(map: &MapGrid) -> Vec<GreedySpot> {
    map.cells()
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
        .collect()
}

/// The controlled agents, in `ctx.agent_ids` order — the same order in which
/// solvers emit actions and [`super::walk::plan_walk`] indexes targets.
pub(crate) fn agents(ctx: &SolverContext<'_>) -> Vec<GreedyAgent> {
    ctx.agent_ids
        .iter()
        .filter_map(|id| {
            let a = ctx.state.agents.iter().find(|a| a.id == *id)?;
            Some(GreedyAgent {
                id: GreedyAgentId::new(*id),
                coord: GreedyCoord::new(a.pos.col, a.pos.row),
                fuel: a.fuel,
            })
        })
        .collect()
}

/// Turn a solver's intents into per-agent day target lists.
///
/// The result is indexed by position in `ctx.agent_ids`, so `out[i]` belongs
/// to `ctx.agent_ids[i]`; an agent with no matching intent gets an empty
/// list and waits all day. Unresolvable spot ids (a spot the map does not
/// have) also degrade to an empty list rather than panicking — the engine
/// remains the final authority on what happens.
pub(crate) fn day_targets(ctx: &SolverContext<'_>, intents: &[GreedyAction]) -> Vec<Vec<HexCoord>> {
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
    fn packed_ids_are_row_major_and_unique() {
        assert_eq!(spot_id_for_coord(HexCoord::new(0, 0)), 0);
        assert_eq!(spot_id_for_coord(HexCoord::new(3, 1)), 35);
        assert_eq!(spot_id_for_coord(HexCoord::new(31, 31)), 31 * 32 + 31);
    }

    /// A 32x32 map with a spot on every cell, so spot-id round-tripping can
    /// be checked at the contest's size bound.
    fn dummy_map() -> MapGrid {
        use engine::model::{MapSize, Spot, Terrain};
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
