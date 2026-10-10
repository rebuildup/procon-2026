//! Print route cost/first-step from each agent to each spot.
use bench::CaseSpec;
use engine::model::{AgentKind, HexCoord, MapGrid, MatchState, Terrain};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

fn step_cost(terrain: Terrain) -> u32 {
    engine::wire_plan::terrain_step_cost(terrain).unwrap_or(u32::MAX)
}
fn index(map: &MapGrid, p: HexCoord) -> usize {
    usize::try_from(p.row).expect("row") * usize::from(map.size.width)
        + usize::try_from(p.col).expect("col")
}
fn routes(
    map: &MapGrid,
    agent: &engine::model::Agent,
    escorted: bool,
) -> Vec<(u32, u32, Option<engine::model::Direction>)> {
    let mut result = vec![(u32::MAX, u32::MAX, None); map.cells().len()];
    let start = index(map, agent.pos);
    result[start] = (0, 0, None);
    let mut queue = BinaryHeap::from([Reverse((0u32, 0u32, start))]);
    while let Some(Reverse((cost, fuel, i))) = queue.pop() {
        if (cost, fuel) != (result[i].0, result[i].1) {
            continue;
        }
        let origin = &map.cells()[i];
        for dir in engine::model::Direction::ALL {
            let to = dir.step(origin.coord);
            let Some(cell) = map.get(to) else { continue };
            if cell.terrain == Terrain::Pond {
                continue;
            }
            let required = engine::rules::fuel_cost(origin.terrain)
                .max(engine::rules::fuel_cost(cell.terrain));
            if agent.kind == AgentKind::Patrol
                && (required > agent.fuel_cap
                    || (!escorted && fuel.saturating_add(required) > agent.fuel))
            {
                continue;
            }
            let next =
                (cost + step_cost(origin.terrain), fuel + engine::rules::fuel_cost(origin.terrain));
            let j = index(map, to);
            if next < (result[j].0, result[j].1) {
                result[j] = (next.0, next.1, if i == start { Some(dir) } else { result[i].2 });
                queue.push(Reverse((next.0, next.1, j)));
            }
        }
    }
    result
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
    let state = MatchState::new(case.to_engine_setting_with_kinds(&team, Some(&kinds))?)?;
    let map = &state.setting.map;
    for (i, agent) in state.agents.iter().enumerate() {
        println!(
            "agent{i} {:?} pos=({},{}) fuel={}",
            agent.kind, agent.pos.col, agent.pos.row, agent.fuel
        );
        if agent.kind != AgentKind::Patrol {
            continue;
        }
        let r = routes(map, agent, true);
        for c in map.cells() {
            if let Some(spot) = &c.spot {
                let (cost, fuel, first) = r[index(map, c.coord)];
                println!(
                    "  s{}@({},{}) cost={} fuel={} first={:?}",
                    spot.series, c.coord.col, c.coord.row, cost, fuel, first
                );
            }
        }
    }
    Ok(())
}
