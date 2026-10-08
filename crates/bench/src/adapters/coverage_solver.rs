//! Daily coverage first, followed by marginal harvest, with joint fuel replay.
//! Planning uses the same engine rules as evaluation and live `CandidateBridge`.
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::MatchSim;
use engine::model::{
    Action, ActionKind, Agent, AgentActions, AgentKind, Direction, HexCoord, MapGrid, Solver,
    SolverContext, Terrain,
};
use engine::rules::{agent_at_daily_pick_cap, fuel_cost, move_fuel};
fn step_cost(terrain: Terrain) -> u32 {
    engine::wire_plan::terrain_step_cost(terrain).unwrap_or(u32::MAX)
}

pub struct CoverageFactory;
const PARAMS: &[ParamSpec] =
    &[ParamSpec { key: "budget", required: true, doc: "official (required)" }];
impl CandidateFactory for CoverageFactory {
    fn algorithm(&self) -> &'static str {
        "coverage"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "Daily series-first joint patrol and supply planning"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("coverage requires budget=official".into()));
        }
        Ok(Box::new(CoverageCandidate { repeat_tour: false }))
    }
}
pub struct CoverageCandidate {
    pub(crate) repeat_tour: bool,
}

#[derive(Clone, Copy)]
struct Route {
    cost: u32,
    fuel: u32,
    first: Option<Direction>,
}
fn index(map: &MapGrid, p: HexCoord) -> usize {
    usize::try_from(p.row).expect("in-bounds row") * usize::from(map.size.width)
        + usize::try_from(p.col).expect("in-bounds column")
}
// One shortest-path tree per active vehicle. Stable queue and direction order
// make ties reproducible. Fuel is a conservative feasibility bound, not score.
fn routes(map: &MapGrid, agent: &Agent, escorted: bool) -> Vec<Route> {
    let mut result = vec![Route { cost: u32::MAX, fuel: u32::MAX, first: None }; map.cells().len()];
    let start = index(map, agent.pos);
    result[start] = Route { cost: 0, fuel: 0, first: None };
    let mut queue = BinaryHeap::from([Reverse((0u32, 0u32, start))]);
    while let Some(Reverse((cost, fuel, i))) = queue.pop() {
        if (cost, fuel) != (result[i].cost, result[i].fuel) {
            continue;
        }
        let origin = &map.cells()[i];
        for dir in Direction::ALL {
            let to = dir.step(origin.coord);
            let Some(cell) = map.get(to) else { continue };
            if cell.terrain == Terrain::Pond {
                continue;
            }
            let required = fuel_cost(origin.terrain).max(fuel_cost(cell.terrain));
            if agent.kind == AgentKind::Patrol
                && (required > agent.fuel_cap
                    || (!escorted && fuel.saturating_add(required) > agent.fuel))
            {
                continue;
            }
            let next = (cost + step_cost(origin.terrain), fuel + fuel_cost(origin.terrain));
            let j = index(map, to);
            if next < (result[j].cost, result[j].fuel) {
                result[j] = Route {
                    cost: next.0,
                    fuel: next.1,
                    first: if i == start { Some(dir) } else { result[i].first },
                };
                queue.push(Reverse((next.0, next.1, j)));
            }
        }
    }
    result
}
fn legal_step(
    agent: &Agent,
    direction: Option<Direction>,
    map: &MapGrid,
    remaining: u32,
) -> Action {
    if let Some(dir) = direction {
        if let (Some(origin), Some(to)) = (map.get(agent.pos), map.get(dir.step(agent.pos))) {
            if to.terrain != Terrain::Pond
                && step_cost(origin.terrain) <= remaining
                && move_fuel(agent, origin.terrain, to.terrain).is_some()
            {
                return Action::step(agent.id, dir);
            }
        }
    }
    Action::wait(agent.id)
}

fn supply_pairs(
    state: &engine::model::MatchState,
    patrols: &[usize],
) -> (Vec<Option<usize>>, BTreeSet<usize>) {
    let n = state.agents.len();
    let mut pairs = vec![None; n];
    let mut assigned = BTreeSet::new();
    for (i, agent) in state.agents.iter().enumerate().filter(|(_, a)| a.kind == AgentKind::Supply) {
        let paths = routes(&state.setting.map, agent, false);
        if let Some(&p) = patrols.iter().min_by_key(|&&p| {
            (assigned.contains(&p), paths[index(&state.setting.map, state.agents[p].pos)].cost, p)
        }) {
            pairs[i] = Some(p);
            assigned.insert(p);
        }
    }
    (pairs, assigned)
}
impl Solver for CoverageCandidate {
    fn name(&self) -> &'static str {
        "coverage"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let mut state = ctx.state.clone();
        // Live bridge supplies fresh day stock. Offline day-start state still
        // has yesterday's residual stock until the first MatchSim step.
        for cell in state.setting.map.cells_mut() {
            if let Some(spot) = &mut cell.spot {
                spot.current_stock = spot.max_stock;
            }
        }
        let n = state.agents.len();
        let patrols: Vec<_> =
            (0..n).filter(|&i| state.agents[i].kind == AgentKind::Patrol).collect();
        let (pairs, assigned) = supply_pairs(&state, &patrols);
        let ranks = self.repeat_tour.then(|| tour_ranks(&state.setting.map, &state.agents));
        let mut used = vec![0u32; n];
        let mut output = Vec::with_capacity(n * ctx.day_steps as usize);
        let sim = MatchSim::default();
        for _ in 0..ctx.day_steps {
            let mut actions: Vec<_> = state.agents.iter().map(|a| Action::wait(a.id)).collect();
            let mut reserved =
                state.series_acquired_by_day.get(&ctx.day).cloned().unwrap_or_default();
            for &i in &patrols {
                if used[i] >= ctx.day_steps {
                    continue;
                }
                let agent = &state.agents[i];
                let map = &state.setting.map;
                let can_pick_here =
                    map.get(agent.pos).and_then(|c| c.spot.as_ref()).is_some_and(|s| {
                        s.current_stock > 0
                            && !agent_at_daily_pick_cap(agent, agent.pos, &state.spot_picks_today)
                    });
                if can_pick_here {
                    if let Some(spot) = map.get(agent.pos).and_then(|c| c.spot.as_ref()) {
                        reserved.insert(spot.series);
                    }
                    continue;
                }
                let paths = routes(map, agent, assigned.contains(&i));
                let target =
                    map.cells()
                        .iter()
                        .filter_map(|c| c.spot.as_ref())
                        .filter(|s| {
                            s.current_stock > 0
                                && !agent_at_daily_pick_cap(agent, s.pos, &state.spot_picks_today)
                        })
                        .filter_map(|s| {
                            let route = paths[index(map, s.pos)];
                            (route.cost <= ctx.day_steps - used[i] && route.first.is_some())
                                .then_some((
                                    reserved.contains(&s.series),
                                    route.cost,
                                    s.series,
                                    index(map, s.pos),
                                    route.first,
                                ))
                        })
                        .min_by_key(|x| {
                            (
                                ranks.is_none() && x.0,
                                ranks.as_ref().map_or(x.1 as usize, |r| r[i][x.3]),
                                x.2,
                                x.3,
                            )
                        });
                if let Some((_, _, series, _, first)) = target {
                    reserved.insert(series);
                    actions[i] = legal_step(agent, first, map, ctx.day_steps - used[i]);
                }
            }
            for i in 0..n {
                if used[i] >= ctx.day_steps {
                    continue;
                }
                if let Some(p) = pairs[i] {
                    let goal = match actions[p].kind {
                        ActionKind::Move(dir) => dir.step(state.agents[p].pos),
                        ActionKind::Wait => state.agents[p].pos,
                    };
                    let paths = routes(&state.setting.map, &state.agents[i], false);
                    actions[i] = legal_step(
                        &state.agents[i],
                        paths[index(&state.setting.map, goal)].first,
                        &state.setting.map,
                        ctx.day_steps - used[i],
                    );
                }
            }
            for i in 0..n {
                if used[i] < ctx.day_steps {
                    used[i] += match actions[i].kind {
                        ActionKind::Move(_) => step_cost(
                            state
                                .setting
                                .map
                                .get(state.agents[i].pos)
                                .expect("agent on map")
                                .terrain,
                        ),
                        ActionKind::Wait => 1,
                    };
                }
            }
            output.extend(actions.iter().copied());
            sim.step(&mut state, &AgentActions::new(actions));
        }
        AgentActions::new(output)
    }
}

fn tour_ranks(map: &MapGrid, agents: &[Agent]) -> Vec<Vec<usize>> {
    agents
        .iter()
        .map(|agent| {
            let mut cursor = Agent::supply(agent.id, agent.pos);
            let mut ranks = vec![usize::MAX; map.cells().len()];
            for rank in 0..map.cells().len() {
                let paths = routes(map, &cursor, true);
                let next = map
                    .cells()
                    .iter()
                    .enumerate()
                    .filter(|(i, c)| c.spot.is_some() && ranks[*i] == usize::MAX)
                    .min_by_key(|(i, _)| (paths[*i].cost, *i));
                let Some((i, cell)) = next else { break };
                if paths[i].cost == u32::MAX {
                    break;
                }
                ranks[i] = rank;
                cursor.pos = cell.coord;
            }
            ranks
        })
        .collect()
}
