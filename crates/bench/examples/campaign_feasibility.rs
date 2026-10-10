//! At each day start, report min route cost from each patrol to each series
//! spot, and whether it fits the day budget.
//! Usage: `campaign_feasibility CASE_JSON CANDIDATE KINDS`
use bench::{CandidateRegistry, CaseSpec};
use engine::{
    MatchSim,
    model::{Agent, AgentActions, HexCoord, MapGrid, MatchState, SolverContext, Terrain},
};
use std::collections::BTreeMap;

fn step_cost(terrain: Terrain) -> u32 {
    engine::wire_plan::terrain_step_cost(terrain).unwrap_or(u32::MAX)
}
fn index(map: &MapGrid, p: HexCoord) -> usize {
    usize::try_from(p.row).expect("row") * usize::from(map.size.width)
        + usize::try_from(p.col).expect("col")
}
fn min_route_cost(map: &MapGrid, agent: &Agent, goal: HexCoord) -> Option<u32> {
    // Dijkstra from agent.pos to goal with step_cost; returns None if unreachable.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = map.cells().len();
    let mut dist = vec![u32::MAX; n];
    let start = index(map, agent.pos);
    dist[start] = 0;
    let mut q = BinaryHeap::from([Reverse((0u32, start))]);
    while let Some(Reverse((d, i))) = q.pop() {
        if d != dist[i] {
            continue;
        }
        if i == index(map, goal) {
            return Some(d);
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
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected CASE_JSON CANDIDATE KINDS".into());
    }
    let case: CaseSpec = serde_json::from_str(&std::fs::read_to_string(&args[1])?)?;
    let pattern = bench::kinds::parse_kinds(&args[3])?;
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a")?;
    let mut state = MatchState::new(case.to_engine_setting_with_kinds(&team, Some(&kinds))?)?;
    let candidate = CandidateRegistry::builtin().build(&args[2])?;
    let solver = candidate.build_solver()?;
    let sim = MatchSim::new(case.sim_config()?);
    // series -> list of spot coords
    let mut series_spots: BTreeMap<u32, Vec<HexCoord>> = BTreeMap::new();
    for c in state.setting.map.cells() {
        if let Some(spot) = &c.spot {
            series_spots.entry(spot.series).or_default().push(c.coord);
        }
    }
    while !state.is_finished() {
        let day = state.day;
        let day_steps = ctx_day_steps(&state);
        let acquired_today = state.series_acquired_by_day.get(&day).cloned().unwrap_or_default();
        let patrols: Vec<usize> = (0..state.agents.len())
            .filter(|&i| state.agents[i].kind == engine::model::AgentKind::Patrol)
            .collect();
        // per-series min cost from any patrol, plus which patrol
        let mut report = Vec::new();
        for (&series, spots) in &series_spots {
            if acquired_today.contains(&series) {
                continue;
            }
            let mut best: Option<(u32, usize)> = None;
            for &p in &patrols {
                for &pos in spots {
                    if let Some(c) = min_route_cost(&state.setting.map, &state.agents[p], pos) {
                        if best.is_none_or(|(b, _)| c < b) {
                            best = Some((c, p));
                        }
                    }
                }
            }
            report.push((series, best));
        }
        let feasible: Vec<_> = report
            .iter()
            .filter(|(_, b)| b.is_some_and(|(c, _)| c < day_steps))
            .map(|(s, _)| s)
            .collect();
        let infeasible: Vec<_> = report
            .iter()
            .filter(|(_, b)| b.is_none_or(|(c, _)| c >= day_steps))
            .map(|(s, b)| (s, b))
            .collect();
        println!(
            "day={day} day_steps={day_steps} series_total={} already={} feasible_now={} infeasible={:?}",
            series_spots.len(),
            acquired_today.len(),
            feasible.len(),
            infeasible,
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

fn ctx_day_steps(state: &MatchState) -> u32 {
    // day budget in steps; MatchSim knows it via ctx, mirror from setting
    state.setting.day_steps_for(state.day)
}
