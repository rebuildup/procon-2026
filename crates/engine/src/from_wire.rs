//! Convert the official wire `MatchSetting` (from `GET /setting`, or a
//! bundled bench fixture) into the engine's internal
//! [`engine::model::MatchSetting`](crate::model::MatchSetting).
//!
//! Pure function — no I/O, no clock. This is the *single* conversion used by
//! every offline evaluation path: the CLI coordinator and the `bench`
//! canonical evaluator both call it, so a fixture and a live server setting
//! reach [`crate::MatchSim`] through exactly the same code.
//!
//! # Cell type mapping (Issue #226 — official table)
//!
//! | Wire `CellType` | Engine `Terrain`        |
//! |---------------:|-------------------------|
//! | `0`            | `Plain`                 |
//! | `1`            | `Road(Smooth)`          |
//! | `2`            | `Mountain`              |
//! | `3`            | `Pond`                  |
//!
//! Source of truth: the official `試合設定JSONについて.md` §「cells の地形タイプ」
//! shipped inside `docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip`
//! (inner path `試合設定用JSONファイル/試合設定JSONについて.md`): `0`=平地,
//! `1`=道路, `2`=山地, `3`=池. The vendored OpenAPI `CellType` only declares
//! `enum: [0, 1, 2, 3]` with no per-value meaning, so that md is the only
//! authoritative value table. Before Issue #226 this function used a
//! permuted table (1=Mountain, 2=Pond, 3=Road), which made the engine walk
//! into official ponds and every moving plan get `revision=-1` from the
//! official server.
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
//! The 3x multiplier is the canonical patrol cap from [`crate::model`]; real
//! kinds come from `POST /agent` on the live path (the solver picks them, the
//! coordinator just stores them).
//!
//! This alternation is **deterministic given the wire setting**, so two runs
//! of the same fixture always field the same team composition. That is what
//! makes cross-candidate comparison on a fixed corpus meaningful.
//!
//! # Day parameters
//!
//! The wire publishes per-day `daySteps`; it is carried through verbatim as
//! [`crate::model::MatchSetting::day_steps_by_day`], so day N really does get
//! `daySteps[N]` steps rather than a copy of day 0's value.
//!
//! `daySeconds` is **not** carried into the engine setting: the simulator has
//! no clock by design (that is what makes it reproducible), so there is
//! nothing to measure it against. It is validated for coherence (same length
//! as `daySteps`, every entry positive) and then judged *by the evaluator*,
//! which compares each day's measured answer time against that day's budget.
//! That is the official per-day answer window; it is a different thing from
//! the harness deadline that kills a runaway worker.
//!
//! # Fuel
//!
//! `fuelLimits` is the patrol fuel cap, taken verbatim. Earlier this function
//! hard-coded `daySteps[0] * 3`, which simulated a *different* match than the
//! one described (e.g. `small-8x8` is `daySteps=16, fuelLimits=24`, so agents
//! were evaluated with 48 fuel instead of 24).
//!
//! Agent *kinds* still alternate patrol/supply, because the wire exposes only
//! starting cells — kind selection is a separate question from fuel.
//!
//! # Traffic thresholds
//!
//! `busyThreshold` / `jammedThreshold` are case properties, not defaults.
//! Use [`sim_config_from_wire`] to build the [`crate::SimConfig`] for a case;
//! [`protocol_setting_to_engine`] only builds the match setting.

use crate::model::MatchSetting as EngineSetting;
use crate::model::{Agent, Cell, HexCoord, MapGrid, MapSize, Spot, Terrain, TrafficState};
use crate::rules::TrafficThresholds;
use crate::solver::TeamId;
use crate::{MatchSim, SimConfig};
use protocol::match_config::{CellType, MatchSetting as WireSetting};

/// Build the [`SimConfig`] a case actually specifies.
///
/// Traffic thresholds are part of the match definition: the bundled fixtures
/// declare `busyThreshold=1, jammedThreshold=2`, while
/// [`TrafficThresholds::default`] is `2/5`. Running a case with the default
/// plays a *different* game — road cells promote and jam on other counts, so
/// movement costs diverge and scores are not comparable.
///
/// # Errors
/// Returns the threshold validation error from Q&A Q13's ranges.
pub fn sim_config_from_wire(p: &WireSetting) -> Result<SimConfig, String> {
    let busy =
        u32::try_from(p.busy_threshold).map_err(|_| "busyThreshold out of range".to_owned())?;
    let jammed =
        u32::try_from(p.jammed_threshold).map_err(|_| "jammedThreshold out of range".to_owned())?;
    let traffic_thresholds = TrafficThresholds::new(busy, jammed)?;
    Ok(SimConfig { traffic_thresholds })
}

/// Convenience: the [`MatchSim`] configured for this wire setting.
///
/// # Errors
/// [`sim_config_from_wire`]'s validation error.
pub fn match_sim_for_wire(p: &WireSetting) -> Result<MatchSim, String> {
    Ok(MatchSim::new(sim_config_from_wire(p)?))
}

/// Convert a wire [`WireSetting`] into an [`EngineSetting`] ready for
/// [`crate::MatchSim`].
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

    // Agents: even index -> patrol, odd index -> supply.
    //
    // Fuel comes straight from the wire's `fuelLimits` (the maximum load a
    // patrol carries). It is NOT derived from day steps — that produced a
    // different fuel budget than the setting declares.
    // Per-day steps, carried through verbatim: day N gets `daySteps[N]`.
    let day_steps_by_day: Vec<u32> = p
        .day_steps
        .iter()
        .enumerate()
        .map(|(i, v)| u32::try_from(*v).map_err(|_| format!("day_steps[{i}] out of u32 range")))
        .collect::<Result<_, _>>()?;
    // `daySeconds` is one budget per day, exactly like `daySteps`. A vector
    // of the wrong length or carrying a non-positive value describes a match
    // the simulator cannot model, so reject it here rather than let the
    // evaluator silently judge a day against no budget.
    if p.day_seconds.len() != p.day_steps.len() {
        return Err(format!(
            "daySeconds has {} entries but daySteps has {}",
            p.day_seconds.len(),
            p.day_steps.len()
        ));
    }
    for (i, secs) in p.day_seconds.iter().enumerate() {
        if *secs <= 0 {
            return Err(format!("daySeconds[{i}] must be > 0, got {secs}"));
        }
    }

    let day_steps0 = day_steps_by_day.first().copied().ok_or("day_steps is empty")?;
    if day_steps_by_day.contains(&0) {
        return Err("every day must allow at least one step".to_owned());
    }
    let fuel_cap = u32::try_from(p.fuel_limits)
        .map_err(|_| format!("fuelLimits {} out of u32 range", p.fuel_limits))?;
    if fuel_cap == 0 {
        return Err("fuelLimits must be >= 1".to_owned());
    }
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
            Agent::patrol_with_fuel(id, coord, fuel_cap)
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
        day_steps_by_day,
    })
}

/// Translate a wire [`CellType`] to an engine [`Terrain`].
///
/// See module-level table for the exact mapping.
fn cell_type_to_terrain(c: CellType) -> Result<Terrain, String> {
    match c.value() {
        0 => Ok(Terrain::Plain),
        1 => Ok(Terrain::Road(TrafficState::Smooth)),
        2 => Ok(Terrain::Mountain),
        3 => Ok(Terrain::Pond),
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

    /// Issue #226: the wire table is the official
    /// `試合設定JSONについて.md` §「cells の地形タイプ」 (bundled inside
    /// `docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip`, inner
    /// path `試合設定用JSONファイル/試合設定JSONについて.md`):
    /// `0` = 平地, `1` = 道路, `2` = 山地, `3` = 池.
    ///
    /// Table-driven so a future re-permutation fails here with the raw wire
    /// value in the message, not silently in a live match.
    #[test]
    fn cell_type_to_terrain_matches_official_md_table() {
        let official: [(i32, CellType, Terrain); 4] = [
            (0, CellType::TYPE_0, Terrain::Plain),
            (1, CellType::TYPE_1, Terrain::Road(TrafficState::Smooth)),
            (2, CellType::TYPE_2, Terrain::Mountain),
            (3, CellType::TYPE_3, Terrain::Pond),
        ];
        for (raw, code, want) in official {
            assert_eq!(code.value(), raw, "CellType constant must carry wire value {raw}");
            assert_eq!(
                cell_type_to_terrain(code).unwrap(),
                want,
                "wire cell type {raw} must decode per the official md table"
            );
        }
    }

    /// Issue #226 equivalence proof (machine-checked): the corrected table is
    /// exactly the pre-#226 table read through the inverse of the identity
    /// permutation that was applied to every authored cell source — so the
    /// permutation preserved terrain meaning.
    ///
    /// Permutation P (authored content): `0→0, 1→2, 2→3, 3→1`.
    /// Its inverse: `0→0, 1→3, 2→1, 3→2`.
    /// Property: `new_table(v) == old_table(P⁻¹(v))` for all `v` in `0..=3`.
    #[test]
    fn official_table_equals_old_table_through_inverse_permutation() {
        // The pre-#226 (wrong) table this issue fixed, spelled out so the
        // comparison is against the historical world, not a re-derivation.
        const OLD_TABLE: [Terrain; 4] = [
            Terrain::Plain,
            Terrain::Mountain,
            Terrain::Pond,
            Terrain::Road(TrafficState::Smooth),
        ];
        // Inverse of `0→0, 1→2, 2→3, 3→1`.
        const INVERSE_PERMUTE: [usize; 4] = [0, 3, 1, 2];
        const CODES: [CellType; 4] =
            [CellType::TYPE_0, CellType::TYPE_1, CellType::TYPE_2, CellType::TYPE_3];

        for (v, code) in CODES.into_iter().enumerate() {
            let new = cell_type_to_terrain(code).expect("0..=3 are all known");
            let old = OLD_TABLE[INVERSE_PERMUTE[v]];
            assert_eq!(
                new, old,
                "wire {v}: corrected table must equal old_table(inverse_permute({v}))"
            );
        }
    }

    #[test]
    fn round_trip_setting() {
        let team = TeamId::new("player-a").unwrap();
        let setting = protocol_setting_to_engine(&wire_setting(), team).unwrap();

        assert_eq!(setting.agents.len(), 2);
        // Agent 0 (even) -> patrol.
        assert_eq!(setting.agents[0].kind, crate::model::AgentKind::Patrol);
        assert_eq!(setting.agents[0].id, 0);
        // Agent 1 (odd) -> supply.
        assert_eq!(setting.agents[1].kind, crate::model::AgentKind::Supply);
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
        let team = TeamId::new("player-a").unwrap();
        assert!(protocol_setting_to_engine(&wire, team).is_err());
    }

    #[test]
    fn empty_day_steps_is_rejected() {
        let mut wire = wire_setting();
        wire.day_steps.clear();
        let team = TeamId::new("player-a").unwrap();
        assert!(protocol_setting_to_engine(&wire, team).is_err());
    }

    #[test]
    fn conversion_is_deterministic() {
        // Same wire setting twice -> byte-identical engine setting. This is
        // what makes fixed-corpus evaluation reproducible.
        let team = TeamId::new("player-a").unwrap();
        let a = protocol_setting_to_engine(&wire_setting(), team.clone()).unwrap();
        let b = protocol_setting_to_engine(&wire_setting(), team).unwrap();
        assert_eq!(a, b);
    }
}
