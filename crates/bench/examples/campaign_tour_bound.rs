//! Greedy day-level tour: how many series can 3 patrols cover on each day if
//! we plan the whole day as a tour instead of step-greedy?
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
fn all_pairs(map: &MapGrid, from: HexCoord) -> Vec<u32> {
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
        // Greedy team tour: repeatedly assign the (series, patrol) pair with
        // the smallest marginal cost, where marginal = dist(patrol_current, nearest spot).
        // Each agent has `budget` units; a visit costs dist + 1 (the pick wait).
        let mut agent_pos: Vec<HexCoord> = patrols.iter().map(|&p| state.agents[p].pos).collect();
        let mut agent_left: Vec<u32> = vec![budget; patrols.len()];
        let mut remaining: Vec<u32> = series_spots.keys().copied().collect();
        let mut covered = 0u32;
        // Precompute distances from each current agent pos to each spot.
        loop {
            // find best (series, agent) by marginal cost
            let mut best: Option<(u32, usize, u32, HexCoord)> = None; // (cost, agent_idx, series, spot)
            for &s in &remaining {
                for (ai, &pos) in agent_pos.iter().enumerate() {
                    let dists = all_pairs(&state.setting.map, pos);
                    for &spot in &series_spots[&s] {
                        let d = dists[index(&state.setting.map, spot)];
                        if d == u32::MAX {
                            continue;
                        }
                        let cost = d + 1;
                        if cost > agent_left[ai] {
                            continue;
                        }
                        if best.is_none_or(|(b, _, _, _)| cost < b) {
                            best = Some((cost, ai, s, spot));
                        }
                    }
                }
            }
            let Some((cost, ai, s, spot)) = best else { break };
            agent_left[ai] -= cost;
            agent_pos[ai] = spot;
            covered += 1;
            remaining.retain(|&x| x != s);
            if remaining.is_empty() {
                break;
            }
        }
        let missing: Vec<_> = remaining;
        println!(
            "day={day} budget={budget} greedy_tour_covered={covered}/{} missing={missing:?}",
            series_spots.len(),
        );
        // Then play the real coverage solver so positions advance normally.
        let ctx = SolverContext::from_state(&state);
        let plan = solver.decide(&ctx);
        let n = ctx.agent_ids.len();
        for chunk in plan.actions.chunks(n) {
            sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
        }
    }
    Ok(())
}
