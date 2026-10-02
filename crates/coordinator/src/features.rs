//! Map feature extraction.
//!
//! EXPECTED INTEGRATION: domain/engine — feature extraction will move next to
//! the real `MatchSetting` once it lands. For now we compute lightweight
//! characteristics from the local stub in [`crate::types`].

use crate::types::MatchSetting;

/// Normalised characteristics describing a single match setting.
///
/// The picker rule layer reasons over these values, not raw match data. Every
/// field is on a roughly comparable scale so the rule predicates can compare
/// magnitudes directly (per `docs/contest/strategy.md` §5.1).
///
/// * `series_count`     — number of distinct udon series on the board.
/// * `spot_count`       — number of spot locations on the board.
/// * `agent_count`      — number of agents in the team (3..=8).
/// * `map_avg_cost`     — mean terrain traversal cost, in \[1.0, 4.0\].
/// * `road_density`     — fraction of cells that are road, in \[0.0, 1.0\].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Characteristics {
    pub series_count: u32,
    pub spot_count: u32,
    pub agent_count: u32,
    pub map_avg_cost: f32,
    pub road_density: f32,
}

/// Per-cell traversal cost. Mirrors the ratio from `strategy.md` §3.2 / §4.
#[inline]
const fn cell_cost(t: crate::types::Terrain) -> f32 {
    match t {
        crate::types::Terrain::Plain | crate::types::Terrain::Road => 1.0,
        crate::types::Terrain::Mountain => 4.0,
    }
}

/// Compute characteristics from a [`MatchSetting`].
///
/// All numeric outputs are deterministic and pure — same input yields the
/// same output. There is no randomness, no clock, no I/O.
#[must_use]
pub fn extract(setting: &MatchSetting) -> Characteristics {
    // Convert counts through `u16` so we can use `f32::from` losslessly:
    // every cell count the contest will ever ship (≤ 32 * 32 = 1024) fits.
    let cell_count_u = u16::try_from(setting.cell_count()).unwrap_or(1).max(1);
    let road_count_u = u16::try_from(setting.road_cell_count()).unwrap_or(0);

    let road_density = f32::from(road_count_u) / f32::from(cell_count_u);

    let mut sum: f32 = 0.0;
    for t in &setting.terrain {
        sum += cell_cost(*t);
    }
    let avg_cost = sum / f32::from(cell_count_u);

    Characteristics {
        series_count: u32::from(setting.series_count),
        spot_count: u32::from(setting.spot_count),
        agent_count: u32::from(setting.agent_count),
        map_avg_cost: avg_cost,
        road_density,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Terrain;

    fn empty(width: u8, height: u8) -> MatchSetting {
        MatchSetting {
            width,
            height,
            terrain: vec![Terrain::Plain; usize::from(width) * usize::from(height)],
            series_count: 0,
            spot_count: 0,
            agent_count: 0,
        }
    }

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "{a} vs {b}");
    }

    #[test]
    fn empty_board_has_zero_road_density_and_unit_avg_cost() {
        let s = empty(4, 4);
        let c = extract(&s);
        approx(c.road_density, 0.0);
        approx(c.map_avg_cost, 1.0);
    }

    #[test]
    fn all_road_board_has_full_density_and_unit_avg_cost() {
        let mut s = empty(4, 4);
        s.terrain = vec![Terrain::Road; 16];
        let c = extract(&s);
        approx(c.road_density, 1.0);
        approx(c.map_avg_cost, 1.0);
    }

    #[test]
    fn all_mountain_board_has_four_x_avg_cost() {
        let mut s = empty(2, 2);
        s.terrain = vec![Terrain::Mountain; 4];
        let c = extract(&s);
        approx(c.road_density, 0.0);
        approx(c.map_avg_cost, 4.0);
    }

    #[test]
    fn mixed_board_proportions() {
        let mut s = empty(2, 2);
        // 2 plain (cost 1) + 1 road (cost 1) + 1 mountain (cost 4)
        s.terrain = vec![Terrain::Plain, Terrain::Plain, Terrain::Road, Terrain::Mountain];
        let c = extract(&s);
        // 1/4 = 0.25 road
        approx(c.road_density, 0.25);
        // (1+1+1+4)/4 = 1.75
        approx(c.map_avg_cost, 1.75);
    }
}
