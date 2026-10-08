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
        Ok(Box::new(CoverageCandidate { variant: 0 }))
    }
}
pub struct CoverageCandidate {
    pub(crate) variant: u64,
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
        let mut state = fresh_day_state(ctx);
        let n = state.agents.len();
        let patrols = patrol_order(&state, self.variant);
        let (_pairs, assigned) = supply_pairs(&state, &patrols);
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
                let target = map
                    .cells()
                    .iter()
                    .filter_map(|c| c.spot.as_ref())
                    .filter(|s| {
                        s.current_stock > 0
                            && !agent_at_daily_pick_cap(agent, s.pos, &state.spot_picks_today)
                    })
                    .filter_map(|s| {
                        let route = paths[index(map, s.pos)];
                        (route.cost <= ctx.day_steps - used[i] && route.first.is_some()).then_some(
                            (
                                reserved.contains(&s.series),
                                route.cost,
                                s.series,
                                index(map, s.pos),
                                route.first,
                            ),
                        )
                    })
                    .min_by_key(|x| {
                        let jitter =
                            if self.variant == 0 { 0 } else { mix(self.variant ^ x.3 as u64) % 8 };
                        (x.0, u64::from(x.1) * (8 + jitter), x.2, x.3)
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
                if state.agents[i].kind != AgentKind::Supply {
                    continue;
                }
                // Refuel duty (Issue #311): a Supply serves the *neediest*
                // Patrol each step — lowest tank ratio first, nearest on
                // ties — instead of shadowing one fixed escort. With one
                // Supply and several Patrols the fixed pair left the rest
                // dry (live 1787 day 3: three of four Patrols parked the
                // whole day); the roaming target keeps the fleet moving,
                // and co-location refuels top the tank up automatically.
                let paths = routes(&state.setting.map, &state.agents[i], false);
                let needy = (0..n)
                    .filter(|&p| state.agents[p].kind == AgentKind::Patrol)
                    .min_by(|&a, &b| {
                        let (fa, fb) = (&state.agents[a], &state.agents[b]);
                        let ra = (u64::from(fa.fuel), u64::from(fa.fuel_cap.max(1)));
                        let rb = (u64::from(fb.fuel), u64::from(fb.fuel_cap.max(1)));
                        // fuel/cap ascending, then walk cost, then index.
                        ra.0
                            .saturating_mul(rb.1)
                            .cmp(&rb.0.saturating_mul(ra.1))
                            .then_with(|| {
                                let ca = paths[index(&state.setting.map, fa.pos)].cost;
                                let cb = paths[index(&state.setting.map, fb.pos)].cost;
                                ca.cmp(&cb)
                            })
                            .then_with(|| a.cmp(&b))
                    });
                let Some(p) = needy else { continue };
                let goal = match actions[p].kind {
                    ActionKind::Move(dir) => dir.step(state.agents[p].pos),
                    ActionKind::Wait => state.agents[p].pos,
                };
                actions[i] = legal_step(
                    &state.agents[i],
                    paths[index(&state.setting.map, goal)].first,
                    &state.setting.map,
                    ctx.day_steps - used[i],
                );
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

pub(crate) fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn fresh_day_state(ctx: &SolverContext<'_>) -> engine::model::MatchState {
    let mut state = ctx.state.clone();
    // Live bridge supplies fresh day stock. Offline day-start state still
    // has yesterday's residual stock until the first MatchSim step.
    for cell in state.setting.map.cells_mut() {
        if let Some(spot) = &mut cell.spot {
            spot.current_stock = spot.max_stock;
        }
    }
    state
}

fn patrol_order(state: &engine::model::MatchState, variant: u64) -> Vec<usize> {
    let n = state.agents.len();
    let mut patrols: Vec<_> =
        (0..n).filter(|&i| state.agents[i].kind == AgentKind::Patrol).collect();
    if !patrols.is_empty() {
        let rotation = usize::try_from(variant % patrols.len() as u64).expect("bounded rotation");
        patrols.rotate_left(rotation);
    }
    patrols
}
