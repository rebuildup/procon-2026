//! Exact-ish day coverage bound: try several assignment orders (round-robin
//! nearest, farthest-first, per-agent nearest) and report the best cover count.
use bench::{CandidateRegistry, CaseSpec};
use engine::{
    MatchSim,
    model::{AgentActions, HexCoord, MapGrid, MatchState, SolverContext, Terrain},
};
use std::collections::BTreeMap;

fn step_cost(terrain: Terrain) -> u32 {
    engine::wire_plan::terrain_step_cost(terrain).unwrap_or(u32::MAX)
}
fn index(map: &MapGrid, p: HexCoord) -> usize {
    usize::try_from(p.row).expect("row") * usize::from(map.size.width)
        + usize::try_from(p.col).expect("col")
}
fn dist_matrix(map: &MapGrid, from: HexCoord) -> Vec<u32> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = map.cells().len();
    let mut dist = vec![u32::MAX; n];
    let start = index(map, from);
    dist[start] = 0;
    let mut q = BinaryHeap::from([Reverse((0u32, start))]);
    while let Some(Reverse((d, i))) = q.pop() {
        if d != dist[i] {
            continue;
        }
        let origin = &map.cells()[i];
        for dir in engine::model::Direction::ALL {
            let to = dir.step(origin.coord);
            let Some(cell) = map.get(to) else { continue };
            if cell.terrain == Terrain::Pond {
                continue;
            }
            let nd = d.saturating_add(step_cost(origin.terrain));
            let j = index(map, to);
            if nd < dist[j] {
                dist[j] = nd;
                q.push(Reverse((nd, j)));
            }
        }
    }
    dist
}

/// Greedy tour from fixed starts: repeatedly give the cheapest feasible
/// (series, agent) pair to that agent. Returns covered count.
fn greedy_cover(
    map: &MapGrid,
    starts: &[HexCoord],
    budget: u32,
    series_spots: &BTreeMap<u32, Vec<HexCoord>>,
    prefer_far: bool,
) -> u32 {
    let mut pos = starts.to_vec();
    let mut left = vec![budget; starts.len()];
    let mut remaining: Vec<u32> = series_spots.keys().copied().collect();
    let mut covered = 0u32;
    while !remaining.is_empty() {
        let mut best: Option<(u32, usize, u32, HexCoord)> = None;
        for &s in &remaining {
            for (ai, &p) in pos.iter().enumerate() {
                let dists = dist_matrix(map, p);
                for &spot in &series_spots[&s] {
                    let d = dists[index(map, spot)];
                    if d == u32::MAX {
                        continue;
                    }
                    let cost = d + 1;
                    if cost > left[ai] {
                        continue;
                    }
                    let better = match &best {
                        None => true,
                        Some((b, _, _, _)) => {
                            if prefer_far {
                                cost > *b
                            } else {
                                cost < *b
                            }
                        }
                    };
                    if better {
                        best = Some((cost, ai, s, spot));
                    }
                }
            }
        }
        let Some((cost, ai, s, spot)) = best else { break };
        left[ai] -= cost;
        pos[ai] = spot;
        covered += 1;
        remaining.retain(|&x| x != s);
    }
    covered
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("expected CASE_JSON KINDS".into());
    }
    let case: CaseSpec = if let Some(name) = args[1].strip_prefix("fixture:") {
        CaseSpec::from_bundled_fixture(name)?
    } else {
        serde_json::from_str(&std::fs::read_to_string(&args[1])?)?
    };
    let pattern = bench::kinds::parse_kinds(&args[2])?;
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a")?;
    let mut state = MatchState::new(case.to_engine_setting_with_kinds(&team, Some(&kinds))?)?;
    let candidate = CandidateRegistry::builtin().build("coverage:budget=official")?;
    let solver = candidate.build_solver()?;
    let sim = MatchSim::new(case.sim_config()?);
    let mut series_spots: BTreeMap<u32, Vec<HexCoord>> = BTreeMap::new();
    for c in state.setting.map.cells() {
        if let Some(spot) = &c.spot {
            series_spots.entry(spot.series).or_default().push(c.coord);
        }
    }
    while !state.is_finished() {
        let day = state.day;
        let budget = state.setting.day_steps_for(day);
        let patrols: Vec<usize> = (0..state.agents.len())
            .filter(|&i| state.agents[i].kind == engine::model::AgentKind::Patrol)
            .collect();
        let starts: Vec<HexCoord> = patrols.iter().map(|&p| state.agents[p].pos).collect();
        let near = greedy_cover(&state.setting.map, &starts, budget, &series_spots, false);
        let far = greedy_cover(&state.setting.map, &starts, budget, &series_spots, true);
        println!(
            "day={day} budget={budget} starts={:?} greedy_near={near}/{} greedy_far={far}/{}",
            starts.iter().map(|p| (p.col, p.row)).collect::<Vec<_>>(),
            series_spots.len(),
            series_spots.len(),
        );
        let ctx = SolverContext::from_state(&state);
        let plan = solver.decide(&ctx);
        let n = ctx.agent_ids.len();
        for chunk in plan.actions.chunks(n) {
            sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
        }
    }
    Ok(())
}
