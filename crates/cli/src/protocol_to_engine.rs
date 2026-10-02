//! Convert the official wire `MatchSetting` (returned by `GET /setting`) into
//! the engine's internal `engine::model::MatchSetting`.
//!
//! Pure function — no I/O, no clock. The result drives every per-day
//! [`engine::solver::SolverContext`] the coordinator hands to the solver.
//!
//! # Cell type mapping (matches `protocol::match_config::CellType`)
//!
//! | Wire `CellType` | Engine `Terrain`        |
//! |---------------:|-------------------------|
//! | `0`            | `Plain`                 |
//! | `1`            | `Mountain`              |
//! | `2`            | `Pond`                  |
//! | `3`            | `Road(Smooth)`          |
//!
//! Spot-bearing plains are modelled by `Cell::spot == Some(...)` rather than
//! by a separate terrain variant; the adapter therefore overlays `p.spots`
//! onto the plain cells after the base terrain pass.
//!
//! # Agent composition (MVP)
//!
//! The wire exposes only initial cell indices (`MatchSetting.agents`); the
//! engine needs `(id, kind, pos, fuel_cap, inventory)`. The MVP rule is the
//! simplest legal alternation that satisfies the team-composition validator:
//!
//! - even index -> `Agent::patrol(id, pos, day_steps, 3)`,
//! - odd index  -> `Agent::supply(id, pos)` (fuel uncapped).
//!
//! The 3x multiplier is the canonical patrol cap from `engine::model`; real
//! kinds come from `POST /agent` (the solver picks them, the coordinator
//! just stores them), but that loop closes in a later issue.
//!
//! # Day parameters
//!
//! The wire publishes per-day `daySteps` / `daySeconds`, while
//! `engine::model::MatchSetting` collapses them to a single `day_steps` plus
//! `num_days`. The MVP picks `day_steps[0]` for the engine; the coordinator
//! overrides it on a per-day basis when it builds the `SolverContext`, so the
//! `0`-th entry is only a placeholder used for agent construction.
use engine::model::{
    Agent, Cell, HexCoord, MapGrid, MapSize, MatchSetting as EngineSetting, Spot, Terrain,
    TrafficState,
};
use engine::solver::TeamId;
use protocol::match_config::{CellType, MatchSetting as WireSetting};

/// Convert a wire [`WireSetting`] into an [`EngineSetting`] ready for the
/// coordinator loop.
///
/// # Errors
///
/// Returns a human-readable error string when the wire payload is malformed
/// in a way the engine cannot recover from (out-of-bounds `CellType`,
/// row-major index that does not fit the map, mismatched cell count, …).
pub fn protocol_setting_to_engine(p: &WireSetting, team: TeamId) -> Result<EngineSetting, String> {
    let width = u8::try_from(p.map.width).map_err(|_| "map.width out of u8 range")?;
    let height = u8::try_from(p.map.height).map_err(|_| "map.height out of u8 range")?;
    let size = MapSize::new(width, height)?;

    if p.map.cells.len() != usize::from(height) {
        return Err(format!("map.cells has {} rows, expected {}", p.map.cells.len(), height));
    }

    // Build the base grid cell-by-cell, then overlay the spot list.
    let mut cells: Vec<Cell> = Vec::with_capacity(usize::from(width) * usize::from(height));
    for (row, row_cells) in p.map.cells.iter().enumerate() {
        if row_cells.len() != usize::from(width) {
            return Err(format!("row {row} has {} cells, expected {width}", row_cells.len()));
        }
        for (col, cell_type) in row_cells.iter().enumerate() {
            let coord =
                HexCoord::new(i32::try_from(col).unwrap_or(0), i32::try_from(row).unwrap_or(0));
            let terrain = cell_type_to_terrain(*cell_type)?;
            cells.push(Cell { coord, terrain, spot: None });
        }
    }

    let mut grid = MapGrid::new(size, cells)?;

    // Overlay spots: brand+1 -> series, pos row-major -> HexCoord.
    for wire_spot in &p.spots {
        let pos_idx = usize::try_from(wire_spot.pos)
            .map_err(|_| format!("spot.pos {} out of usize range", wire_spot.pos))?;
        let row = pos_idx / usize::from(width);
        let col = pos_idx % usize::from(width);
        let coord = HexCoord::new(
            i32::try_from(col).map_err(|_| "spot col out of i32 range")?,
            i32::try_from(row).map_err(|_| "spot row out of i32 range")?,
        );
        let series = u32::try_from(wire_spot.brand)
            .map_err(|_| format!("spot.brand {} out of u32 range", wire_spot.brand))?
            + 1;
        let max_stock = u32::try_from(wire_spot.stocks)
            .map_err(|_| format!("spot.stocks {} out of u32 range", wire_spot.stocks))?;
        let spot = Spot { series, pos: coord, max_stock, current_stock: max_stock };
        let cell = grid.get_mut(coord).ok_or_else(|| {
            format!("spot at pos {} maps to {coord:?} which is outside the map", wire_spot.pos)
        })?;
        cell.spot = Some(spot);
    }

    // Agents: even index -> patrol, odd index -> supply. Day-steps feed the
    // patrol fuel cap; supply has none.
    let day_steps0 = match p.day_steps.first() {
        Some(v) => u32::try_from(*v).map_err(|_| "day_steps[0] out of u32 range")?,
        None => return Err("day_steps is empty".to_owned()),
    };
    let mut agents = Vec::with_capacity(p.agents.len());
    for (idx, wire_pos) in p.agents.iter().enumerate() {
        let pos_idx = usize::try_from(*wire_pos)
            .map_err(|_| format!("agent start pos {wire_pos} out of usize range"))?;
        let row = pos_idx / usize::from(width);
        let col = pos_idx % usize::from(width);
        let coord = HexCoord::new(
            i32::try_from(col).map_err(|_| "agent col out of i32 range")?,
            i32::try_from(row).map_err(|_| "agent row out of i32 range")?,
        );
        let id = u8::try_from(idx).map_err(|_| format!("agent index {idx} out of u8 range"))?;
        let agent = if idx % 2 == 0 {
            Agent::patrol(id, coord, day_steps0, 3)
        } else {
            Agent::supply(id, coord)
        };
        agents.push(agent);
    }

    let num_days = u32::try_from(p.day_steps.len())
        .map_err(|_| format!("day_steps.len() {} out of u32 range", p.day_steps.len()))?;

    Ok(EngineSetting {
        match_id: domain::MatchId::new("match-from-setting")?,
        team,
        map: grid,
        agents,
        day_steps: day_steps0,
        num_days,
    })
}

/// Translate a wire [`CellType`] to an engine [`Terrain`].
///
/// See module-level table for the exact mapping.
fn cell_type_to_terrain(c: CellType) -> Result<Terrain, String> {
    match c.value() {
        0 => Ok(Terrain::Plain),
        1 => Ok(Terrain::Mountain),
        2 => Ok(Terrain::Pond),
        3 => Ok(Terrain::Road(TrafficState::Smooth)),
        other => Err(format!("unknown CellType value {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::match_config::{Map, Spot as WireSpot};

    fn wire_setting() -> WireSetting {
        // 8x8 plain grid; one plain cell carries a brand-0 spot; agents at
        // indices 0 and 1 (so even -> patrol, odd -> supply).
        let mut cells = vec![vec![CellType::TYPE_0; 8]; 8];
        cells[3][4] = CellType::TYPE_2; // pond
        cells[0][7] = CellType::TYPE_1; // mountain
        cells[7][0] = CellType::TYPE_3; // road
        WireSetting {
            starts_at: 1_700_000_000,
            day_seconds: vec![60],
            day_steps: vec![10],
            map: Map { height: 8, width: 8, cells },
            spots: vec![WireSpot { brand: 0, pos: 4 * 8 + 3, stocks: 2 }],
            agents: vec![0, 1],
            fuel_limits: 30,
            players: 2,
            busy_threshold: 3,
            jammed_threshold: 5,
        }
    }

    #[test]
    fn cell_type_mapping_is_stable() {
        assert_eq!(cell_type_to_terrain(CellType::TYPE_0).unwrap(), Terrain::Plain);
        assert_eq!(cell_type_to_terrain(CellType::TYPE_1).unwrap(), Terrain::Mountain);
        assert_eq!(cell_type_to_terrain(CellType::TYPE_2).unwrap(), Terrain::Pond);
        assert_eq!(
            cell_type_to_terrain(CellType::TYPE_3).unwrap(),
            Terrain::Road(TrafficState::Smooth)
        );
    }

    #[test]
    fn round_trip_setting() {
        let team = engine::solver::TeamId::new("player-a").unwrap();
        let setting = protocol_setting_to_engine(&wire_setting(), team).unwrap();

        assert_eq!(setting.agents.len(), 2);
        // Agent 0 (even) -> patrol.
        assert_eq!(setting.agents[0].kind, engine::model::AgentKind::Patrol);
        assert_eq!(setting.agents[0].id, 0);
        // Agent 1 (odd) -> supply.
        assert_eq!(setting.agents[1].kind, engine::model::AgentKind::Supply);
        assert_eq!(setting.agents[1].id, 1);

        // Spot overlay lands on a plain cell (pos=35 = row 4, col 3).
        let spot_cell = setting.map.get(HexCoord::new(3, 4)).unwrap();
        let spot = spot_cell.spot.as_ref().expect("spot attached to plain cell");
        assert_eq!(spot.series, 1); // brand=0 -> series=1
        assert_eq!(spot.max_stock, 2);
        assert_eq!(spot.current_stock, 2);

        // Terrain overrides stick.
        assert_eq!(setting.map.get(HexCoord::new(4, 3)).unwrap().terrain, Terrain::Pond);
        assert_eq!(setting.map.get(HexCoord::new(7, 0)).unwrap().terrain, Terrain::Mountain);
        assert_eq!(
            setting.map.get(HexCoord::new(0, 7)).unwrap().terrain,
            Terrain::Road(TrafficState::Smooth)
        );
    }

    #[test]
    fn mismatched_row_count_is_rejected() {
        let mut wire = wire_setting();
        wire.map.cells = vec![vec![CellType::TYPE_0; 8]; 7];
        let team = engine::solver::TeamId::new("player-a").unwrap();
        assert!(protocol_setting_to_engine(&wire, team).is_err());
    }

    #[test]
    fn empty_day_steps_is_rejected() {
        let mut wire = wire_setting();
        wire.day_steps.clear();
        let team = engine::solver::TeamId::new("player-a").unwrap();
        assert!(protocol_setting_to_engine(&wire, team).is_err());
    }
}
