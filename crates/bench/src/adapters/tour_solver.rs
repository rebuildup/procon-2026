//! Day-level tour planner for large maps (Issue #333).
//!
//! Assigns each series to the nearest patrol and walks a nearest-neighbor
//! tour. Designed for 32x32 maps with 27+ series where step-greedy misses
//! far series.
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use crate::adapters::walk::{BudgetMode, WalkMode, plan_walk_with};
use engine::model::{
    AgentActions, AgentKind, HexCoord, MapGrid, Solver, SolverContext,
};

pub struct TourFactory;
const PARAMS: &[ParamSpec] =
    &[ParamSpec { key: "budget", required: true, doc: "official (required)" }];
impl CandidateFactory for TourFactory {
    fn algorithm(&self) -> &'static str {
        "tour"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "Day-level series-to-patrol tour for large maps"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("tour requires budget=official".into()));
        }
        Ok(Box::new(TourCandidate))
    }
}

pub struct TourCandidate;

/// Hex distance (even-row right-offset, matching `walk::hex_distance`).
fn hex_dist(a: HexCoord, b: HexCoord) -> u32 {
    crate::adapters::walk::hex_distance(a, b)
}

impl Solver for TourCandidate {
    fn name(&self) -> &'static str {
        "tour"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let state = ctx.state;
        let map = &state.setting.map;
        let n = state.agents.len();
        let patrols: Vec<usize> = (0..n)
            .filter(|&i| state.agents[i].kind == AgentKind::Patrol)
            .collect();

        // Group spots by series.
        let mut series_spots: std::collections::BTreeMap<u32, Vec<HexCoord>> =
            std::collections::BTreeMap::new();
        for c in map.cells() {
            if let Some(spot) = &c.spot {
                series_spots.entry(spot.series).or_default().push(c.coord);
            }
        }

        // Assign each series to the nearest patrol (min hex distance to any spot).
        let mut patrol_targets: Vec<Vec<HexCoord>> = vec![Vec::new(); n];
        for (_, spots) in &series_spots {
            let mut best: Option<(u32, usize, HexCoord)> = None;
            for &p in &patrols {
                for &pos in spots {
                    let d = hex_dist(state.agents[p].pos, pos);
                    if best.is_none_or(|(b, _, _)| d < b) {
                        best = Some((d, p, pos));
                    }
                }
            }
            if let Some((_, p, pos)) = best {
                patrol_targets[p].push(pos);
            }
        }

        // Nearest-neighbor tour order from each patrol's start.
        let mut targets: Vec<Vec<HexCoord>> = vec![Vec::new(); n];
        for &p in &patrols {
            let mut remaining = patrol_targets[p].clone();
            let mut sorted = Vec::new();
            let mut cur = state.agents[p].pos;
            while !remaining.is_empty() {
                let (idx, _) = remaining
                    .iter()
                    .enumerate()
                    .min_by_key(|&(_, &pos)| hex_dist(cur, pos))
                    .unwrap();
                cur = remaining.remove(idx);
                sorted.push(cur);
            }
            targets[p] = sorted;
        }

        // Supplies follow their paired patrol.
        for i in 0..n {
            if state.agents[i].kind == AgentKind::Supply {
                // Find nearest patrol.
                if let Some(&p) = patrols.iter().min_by_key(|&&p| {
                    hex_dist(state.agents[i].pos, state.agents[p].pos)
                }) {
                    targets[i] = targets[p].clone();
                }
            }
        }

        plan_walk_with(ctx, &targets, WalkMode::Astar, BudgetMode::Official)
    }
}
