//! Daily coverage first, followed by marginal harvest or risk repositioning,
//! with joint fuel replay. Planning uses the same engine rules as evaluation
//! and live `CandidateBridge`.
//!
//! `park=none` (default) harvests the nearest reserved spot after coverage.
//! `park=risk` instead drives end-of-day movement toward series that would be
//! out of reach for the next (shorter) day, so the following day starts close
//! to them (Issue #331).
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
const PARAMS: &[ParamSpec] = &[
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
    ParamSpec { key: "park", required: false, doc: "none (default) | risk" },
];
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
        let park_risk = match spec.param("park") {
            None | Some("none") => false,
            Some("risk") | Some("far") => true,
            Some(other) => {
                return Err(CandidateError::Build(format!("unknown park mode {other:?}")));
            }
        };
        Ok(Box::new(CoverageCandidate { park_risk }))
    }
}
pub struct CoverageCandidate {
    park_risk: bool,
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
        let tomorrow_budget = {
            let tomorrow = ctx.day.saturating_add(1);
            if tomorrow < state.setting.num_days {
                state.setting.day_steps_for(tomorrow)
            } else {
                0
            }
        };
        // Only spread on maps where nearest-first cannot cover all series
        // (Issue #331). Small boards are already covered by greedy proximity.
        let spread_map = {
            let size = &state.setting.map.size;
            usize::from(size.width) >= 24 || usize::from(size.height) >= 24
        };
        let mut used = vec![0u32; n];
        let mut output = Vec::with_capacity(n * ctx.day_steps as usize);
        let sim = MatchSim::default();
        // Day-scoped claim set: once a patrol targets a series, no other patrol
        // (or later step) re-targets it. Park mode keeps this across steps so
        // agents spread across series instead of converging (Issue #331).
        let mut day_claimed: BTreeSet<u32> =
            state.series_acquired_by_day.get(&ctx.day).cloned().unwrap_or_default();
        for _ in 0..ctx.day_steps {
            let mut actions: Vec<_> = state.agents.iter().map(|a| Action::wait(a.id)).collect();
            let mut reserved =
                state.series_acquired_by_day.get(&ctx.day).cloned().unwrap_or_default();
            let mut claimed: BTreeSet<u32> = BTreeSet::new();
            let patrol_paths: Vec<(usize, Vec<Route>)> = patrols
                .iter()
                .map(|&i| (i, routes(&state.setting.map, &state.agents[i], assigned.contains(&i))))
                .collect();
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
                let paths =
                    &patrol_paths.iter().find(|(j, _)| *j == i).expect("patrol route computed").1;
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
                                    // Park: spread across unclaimed series only on
                                    // medium+ days whose tomorrow is also medium+.
                                    // Short days scatter patrols; spreading the day
                                    // before a short day can strand the next start
                                    // (Issue #331).
                                    self.park_risk
                                        && spread_map
                                        && ctx.day_steps >= 70
                                        && tomorrow_budget >= 70
                                        && day_claimed.contains(&s.series),
                                    route.cost,
                                    s.series,
                                    index(map, s.pos),
                                    route.first,
                                ))
                        })
                        .min_by_key(|x| (x.0, x.1, x.2, x.3, x.4));
                if let Some((_, _, _, series, _, first)) = target {
                    reserved.insert(series);
                    claimed.insert(series);
                    day_claimed.insert(series);
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
