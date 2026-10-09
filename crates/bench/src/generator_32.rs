//! 32x32 large-map case generator for Issue #331 evaluation.
//!
//! Generates reproducible 32x32 fixtures with 27 series across multiple
//! distribution patterns and terrain mixes. Each case records its generation
//! parameters so the exact board can be rebuilt from the seed.
use crate::case::{CaseSpec, CaseSource};
use protocol::match_config::Spot as WireSpot;

/// Terrain distribution knobs.
#[derive(Debug, Clone, Copy)]
pub struct TerrainMix {
    pub plain_pct: u32,
    pub road_pct: u32,
    pub mountain_pct: u32,
    // pond_pct = 100 - plain - road - mountain
}

impl TerrainMix {
    pub const BALANCED: Self = Self { plain_pct: 40, road_pct: 30, mountain_pct: 20 };
    pub const ROAD_HEAVY: Self = Self { plain_pct: 30, road_pct: 50, mountain_pct: 15 };
    pub const MOUNTAIN_HEAVY: Self = Self { plain_pct: 25, road_pct: 20, mountain_pct: 45 };
    pub const FUEL_TIGHT: Self = Self { plain_pct: 50, road_pct: 25, mountain_pct: 15 };
}

/// Series distribution pattern.
#[derive(Debug, Clone, Copy)]
pub enum SeriesPattern {
    /// Series spread uniformly across the map.
    Uniform,
    /// Series clustered in the center.
    Central,
    /// Series clustered at the edges.
    Peripheral,
}

/// Generation knobs for one 32x32 case.
#[derive(Debug, Clone, Copy)]
pub struct LargeCaseParams {
    pub seed: u64,
    pub n_series: usize,
    pub n_agents: usize,
    pub n_days: usize,
    pub day_steps_base: u32,
    pub fuel_cap: u32,
    pub terrain: TerrainMix,
    pub pattern: SeriesPattern,
}

impl LargeCaseParams {
    /// Canonical 32x32 / 27-series / 7-agent / 7-day case (matches live #1762 shape).
    pub const CANONICAL: Self = Self {
        seed: 42,
        n_series: 27,
        n_agents: 7,
        n_days: 7,
        day_steps_base: 80,
        fuel_cap: 121,
        terrain: TerrainMix::BALANCED,
        pattern: SeriesPattern::Uniform,
    };
}

/// Deterministic PRNG (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Build a 32x32 `CaseSpec` from parameters.
pub fn generate_large_case(params: &LargeCaseParams) -> Result<CaseSpec, String> {
    let edge = 32i32;
    let n_cells = (edge * edge) as usize;
    let mut rng = Rng(params.seed | 1);

    // Terrain assignment
    let pond_pct = 100 - params.terrain.plain_pct - params.terrain.road_pct - params.terrain.mountain_pct;
    let mut cells: Vec<i32> = Vec::with_capacity(n_cells);
    for _ in 0..n_cells {
        let r = rng.below(100) as u32;
        let t = if r < params.terrain.plain_pct {
            0i32 // Plain
        } else if r < params.terrain.plain_pct + params.terrain.road_pct {
            1i32 // Road
        } else if r < params.terrain.plain_pct + params.terrain.road_pct + params.terrain.mountain_pct {
            2i32 // Mountain
        } else {
            3i32 // Pond
        };
        cells.push(t);
    }

    // Ensure borders are traversable (Road)
    for row in 0..edge {
        for col in 0..edge {
            if row == 0 || row == edge - 1 || col == 0 || col == edge - 1 {
                cells[(row * edge + col) as usize] = 1;
            }
        }
    }

    // Spot placement with series distribution pattern (Issue #333).
    let n_spots = params.n_series;
    let mut spots: Vec<(u32, i32, u32)> = Vec::with_capacity(n_spots);
    let mut used_positions = std::collections::HashSet::new();
    for series in 0..params.n_series as u32 {
        // Choose target region based on pattern.
        let pos = loop {
            let r = rng.below(n_cells as u64) as i32;
            let row = r / edge;
            let col = r % edge;
            if cells[r as usize] != 0 || used_positions.contains(&r) {
                continue;
            }
            // Pattern filter: bias placement toward the target region.
            let in_region = match params.pattern {
                SeriesPattern::Uniform => true,
                SeriesPattern::Central => {
                    let dr = (row - edge / 2).abs();
                    let dc = (col - edge / 2).abs();
                    dr + dc < edge / 3
                }
                SeriesPattern::Peripheral => {
                    let dr = (row - edge / 2).abs();
                    let dc = (col - edge / 2).abs();
                    dr + dc >= edge / 3
                }
            };
            if in_region {
                break r;
            }
        };
        used_positions.insert(pos);
        let stocks = 1 + rng.below(4) as u32;
        spots.push((series + 1, pos, stocks));
    }

    // Agent start positions (on Plain, not on spots)
    let mut agent_pos: Vec<i32> = Vec::with_capacity(params.n_agents);
    let mut used_starts = std::collections::HashSet::new();
    for _ in 0..params.n_agents {
        let pos = loop {
            let r = rng.below(n_cells as u64) as i32;
            if cells[r as usize] == 0 && !used_positions.contains(&r) && !used_starts.contains(&r) {
                break r;
            }
        };
        used_starts.insert(pos);
        agent_pos.push(pos);
    }

    // Day steps: vary around base
    let day_steps: Vec<u32> = (0..params.n_days)
        .map(|_| params.day_steps_base + (rng.below(20) as u32) - 10)
        .collect();

    // Build wire setting
    let map_cells: Vec<Vec<protocol::match_config::CellType>> = (0..edge)
        .map(|row| {
            (0..edge)
                .map(|col| {
                    use protocol::match_config::CellType;
                    match cells[(row * edge + col) as usize] {
                        0 => CellType::TYPE_0,
                        1 => CellType::TYPE_1,
                        2 => CellType::TYPE_2,
                        _ => CellType::TYPE_3,
                    }
                })
                .collect()
        })
        .collect();

    let wire_spots: Vec<WireSpot> = spots
        .iter()
        .map(|&(series, pos, stocks)| WireSpot {
            brand: (series - 1) as i32,
            pos,
            stocks: stocks as i32,
        })
        .collect();

    let wire_setting = protocol::match_config::MatchSetting {
        starts_at: 0,
        day_seconds: vec![60; params.n_days as usize],
        day_steps: day_steps.iter().map(|&s| s as i32).collect(),
        map: protocol::match_config::Map {
            height: edge,
            width: edge,
            cells: map_cells,
        },
        spots: wire_spots,
        agents: agent_pos.clone(),
        fuel_limits: params.fuel_cap as i32,
        players: 10,
        busy_threshold: 1,
        jammed_threshold: 6,
    };

    let case_id = format!(
        "generated:large32:seed={}:series={}:agents={}:days={}:terrain={:?}:pattern={:?}",
        params.seed, params.n_series, params.n_agents, params.n_days, params.terrain, params.pattern
    );

    Ok(CaseSpec {
        case_id,
        seed: Some(params.seed),
        generation_parameters: Some(serde_json::json!({
            "archetype": "large32",
            "seed": params.seed,
            "n_series": params.n_series,
            "n_agents": params.n_agents,
            "n_days": params.n_days,
            "day_steps_base": params.day_steps_base,
            "fuel_cap": params.fuel_cap,
            "terrain_mix": {
                "plain": params.terrain.plain_pct,
                "road": params.terrain.road_pct,
                "mountain": params.terrain.mountain_pct,
                "pond": pond_pct,
            },
            "pattern": format!("{:?}", params.pattern),
        })),
        source: CaseSource::Generated,
        setting: wire_setting,
    })
}

/// Generate the standard evaluation suite for 32x32 testing.
pub fn large32_suite() -> Vec<LargeCaseParams> {
    vec![
        LargeCaseParams::CANONICAL,
        LargeCaseParams {
            pattern: SeriesPattern::Central,
            ..LargeCaseParams::CANONICAL
        },
        LargeCaseParams {
            pattern: SeriesPattern::Peripheral,
            ..LargeCaseParams::CANONICAL
        },
        LargeCaseParams {
            terrain: TerrainMix::ROAD_HEAVY,
            ..LargeCaseParams::CANONICAL
        },
        LargeCaseParams {
            terrain: TerrainMix::MOUNTAIN_HEAVY,
            ..LargeCaseParams::CANONICAL
        },
        LargeCaseParams {
            terrain: TerrainMix::FUEL_TIGHT,
            fuel_cap: 60,
            ..LargeCaseParams::CANONICAL
        },
        // Short-day case (harder coverage)
        LargeCaseParams {
            day_steps_base: 50,
            ..LargeCaseParams::CANONICAL
        },
        // More agents
        LargeCaseParams {
            n_agents: 5,
            ..LargeCaseParams::CANONICAL
        },
        // More days
        LargeCaseParams {
            n_days: 10,
            ..LargeCaseParams::CANONICAL
        },
    ]
}
