//! Wire `MatchSetting` → engine `MatchSetting` conversion.
//!
//! The conversion itself lives in [`engine::from_wire`] so that the offline
//! canonical evaluator (`crates/bench`) and this CLI share **one** code path
//! from the official wire shape to [`engine::model::MatchSetting`]. Keeping
//! it in the engine is what makes a bundled bench fixture and a live
//! `GET /setting` payload reach [`engine::MatchSim`] identically.
//!
//! This module is a thin re-export for the CLI's existing call sites and
//! tests. Prefer `engine::from_wire::protocol_setting_to_engine` in new code.

pub use engine::from_wire::protocol_setting_to_engine;

#[cfg(test)]
mod tests {
    use super::*;
    use engine::model::{AgentKind, HexCoord, Terrain, TrafficState};
    use engine::solver::TeamId;
    use protocol::match_config::{CellType, Map, MatchSetting as WireSetting, Spot as WireSpot};

    fn wire_setting() -> WireSetting {
        let mut cells = vec![vec![CellType::TYPE_0; 8]; 8];
        cells[3][4] = CellType::TYPE_3; // pond (official: 3 = 池)
        cells[0][7] = CellType::TYPE_2; // mountain (official: 2 = 山地)
        cells[7][0] = CellType::TYPE_1; // road (official: 1 = 道路)
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

    /// The CLI keeps a smoke test on the re-exported path so a future move
    /// of the real implementation cannot silently break the coordinator.
    #[test]
    fn reexport_still_converts_wire_settings() {
        let team = TeamId::new("player-a").unwrap();
        let setting = protocol_setting_to_engine(&wire_setting(), team).unwrap();
        assert_eq!(setting.agents.len(), 2);
        assert_eq!(setting.agents[0].kind, AgentKind::Patrol);
        assert_eq!(setting.agents[1].kind, AgentKind::Supply);
        let spot_cell = setting.map.get(HexCoord::new(3, 4)).unwrap();
        assert_eq!(spot_cell.spot.as_ref().expect("spot").series, 1);
        assert_eq!(setting.map.get(HexCoord::new(4, 3)).unwrap().terrain, Terrain::Pond);
        assert_eq!(
            setting.map.get(HexCoord::new(0, 7)).unwrap().terrain,
            Terrain::Road(TrafficState::Smooth)
        );
    }
}
