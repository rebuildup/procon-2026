//! Structural simulated annealing over patrol spot visit orders.
//!
//! This is an independent Rust candidate inspired by the *idea* of route-level
//! optimization in the team's yakin amashi research, not a source-code port.
//! The official MatchSim is the only acceptance and scoring authority.
//! Supplies initially retain the safe baseline schedule; rendezvous mutation
//! and multi-day rollouts are separate, unimplemented hypotheses.
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::time::{Duration, Instant};

use crate::adapters::coverage_solver::CoverageCandidate;
use crate::adapters::walk::BudgetMode;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::MatchSim;
use engine::model::{
    Action, ActionKind, AgentActions, AgentKind, Direction, HexCoord, MapGrid, Solver, SolverContext,
};

pub struct StructuralFactory;
const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "trials",
        required: false,
        doc: "deterministic count of route mutations (1..=10000)",
    },
    ParamSpec {
        key: "budget_ms",
        required: false,
        doc: "wall-clock per-day exploration budget in ms, capped at 59000",
    },
    ParamSpec { key: "seed", required: false, doc: "deterministic search seed (default 42)" },
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
];

impl CandidateFactory for StructuralFactory {
    fn algorithm(&self) -> &'static str {
        "structural"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn describe(&self) -> &'static str {
        "Spot-order simulated annealing (insertion/swap/2-opt/transfer), canonical replay"
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("structural requires budget=official".into()));
        }
        let trials = parse_u32(spec, "trials", 0)?;
        let budget_ms = parse_u32(spec, "budget_ms", 0)?;
        if trials > 10_000 {
            return Err(CandidateError::Build("trials must be <= 10000".into()));
        }
        if budget_ms > 59_000 {
            return Err(CandidateError::Build("budget_ms must be <= 59000".into()));
        }
        if trials == 0 && budget_ms == 0 {
            return Err(CandidateError::Build("set trials or budget_ms".into()));
        }
        let seed = match spec.param("seed") {
            None => 42,
            Some(raw) => raw
                .parse::<u64>()
                .map_err(|_| CandidateError::Build(format!("invalid seed {raw:?}")))?,
        };
        Ok(Box::new(StructuralCandidate { trials, budget_ms, seed }))
    }
}

fn parse_u32(spec: &CandidateSpec, key: &str, default: u32) -> Result<u32, CandidateError> {
    spec.param(key).map_or(Ok(default), |raw| {
        raw.parse::<u32>().map_err(|_| CandidateError::Build(format!("invalid {key} {raw:?}")))
    })
}

#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed })
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn index(&mut self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            let bound = u64::try_from(len).expect("usize always fits u64 on supported targets");
            usize::try_from(self.next() % bound).expect("random index is smaller than len")
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "shifting away 11 bits leaves exactly 53 random bits")]
    fn probability(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

type VisitOrder = Vec<Vec<usize>>;
type PathCache = HashMap<(HexCoord, HexCoord, u32), Option<Vec<Direction>>>;

fn cell_idx(map: &MapGrid, p: HexCoord) -> usize {
    usize::try_from(p.row).expect("position is in bounds") * usize::from(map.size.width)
        + usize::try_from(p.col).expect("position is in bounds")
}

/// Weighted Dijkstra: duration is primary; lambda creates alternative
/// low-fuel paths, without using a non-official terrain/time model.
fn shortest_path(
    map: &MapGrid,
    from: HexCoord,
    to: HexCoord,
    fuel_weight: u32,
) -> Option<Vec<Direction>> {
    if from == to {
        return Some(Vec::new());
    }
    if map.get(from).is_none() || map.get(to).is_none() {
        return None;
    }
    let len = map.cells().len();
    let start = cell_idx(map, from);
    let dest = cell_idx(map, to);
    let mut cost = vec![u32::MAX; len];
    let mut parent = vec![None::<(usize, Direction)>; len];
    let mut queue = BinaryHeap::from([Reverse((0u32, start))]);
    cost[start] = 0;
    while let Some(Reverse((dist, i))) = queue.pop() {
        if cost[i] != dist {
            continue;
        }
        if i == dest {
            break;
        }
        let origin = &map.cells()[i];
        let Some(dt) = engine::wire_plan::terrain_step_cost(origin.terrain) else {
            continue;
        };
        let weight =
            dt.saturating_mul(8) + engine::rules::fuel_cost(origin.terrain) * fuel_weight;
        for dir in Direction::ALL {
            let next = dir.step(origin.coord);
            if !map.get(next).is_some_and(|c| c.terrain.traversable()) {
                continue;
            }
            let j = cell_idx(map, next);
            let new_cost = dist.saturating_add(weight);
            if new_cost < cost[j] {
                cost[j] = new_cost;
                parent[j] = Some((i, dir));
                queue.push(Reverse((new_cost, j)));
            }
        }
    }
    if cost[dest] == u32::MAX {
        return None;
    }
    let mut rev = Vec::new();
    let mut cur = dest;
    while cur != start {
        let (prev, dir) = parent[cur]?;
        rev.push(dir);
        cur = prev;
    }
    rev.reverse();
    Some(rev)
}

fn path(
    cache: &mut PathCache,
    map: &MapGrid,
    from: HexCoord,
    to: HexCoord,
    fuel_weight: u32,
) -> Option<Vec<Direction>> {
    cache
        .entry((from, to, fuel_weight))
        .or_insert_with(|| shortest_path(map, from, to, fuel_weight))
        .clone()
}

fn baseline_visit_order(
    ctx: &SolverContext<'_>,
    baseline: &AgentActions,
    spots: &[HexCoord],
) -> VisitOrder {
    let n = ctx.agent_ids.len();
    let mut orders = vec![Vec::<usize>::new(); n];
    for (i, agent_id) in ctx.agent_ids.iter().enumerate() {
        let Some(agent) = ctx.state.agents.iter().find(|a| a.id == *agent_id) else {
            continue;
        };
        if agent.kind != AgentKind::Patrol {
            continue;
        }
        let mut here = agent.pos;
        if let Some(idx) = spots.iter().position(|s| *s == here) {
            orders[i].push(idx);
        }
        for actions in baseline.actions.chunks_exact(n) {
            if let ActionKind::Move(dir) = actions[i].kind {
                here = dir.step(here);
                if let Some(idx) = spots.iter().position(|s| *s == here) {
                    if !orders[i].contains(&idx) {
                        orders[i].push(idx);
                    }
                }
            }
        }
    }
    orders
}

fn mutate(order: &mut VisitOrder, patrols: &[usize], spot_count: usize, rng: &mut Rng) {
    if patrols.is_empty() || spot_count == 0 {
        return;
    }
    let p = patrols[rng.index(patrols.len())];
    let len = order[p].len();
    match rng.index(7) {
        0 | 1 => {
            let s = rng.index(spot_count);
            if !order[p].contains(&s) && len < spot_count {
                let at = rng.index(len + 1);
                order[p].insert(at, s);
            }
        }
        2 if len >= 2 => order[p].swap(rng.index(len), rng.index(len)),
        3 if len >= 2 => {
            let a = rng.index(len);
            let b = rng.index(len);
            order[p][a.min(b)..=a.max(b)].reverse();
        }
        4 if len > 0 => {
            order[p].remove(rng.index(len));
        }
        5 if len > 0 && patrols.len() > 1 => {
            let q = patrols[rng.index(patrols.len())];
            if q != p {
                let s = order[p].remove(rng.index(len));
                let at = rng.index(order[q].len() + 1);
                order[q].insert(at, s);
            }
        }
        6 if len > 0 => {
            let at = rng.index(len);
            let s = rng.index(spot_count);
            if !order[p].contains(&s) {
                order[p][at] = s;
            }
        }
        _ => {}
    }
}

/// Rebuild a Patrol's full-day per-agent sequential actions from a visit order.
/// The fuel budget here is deliberately conservative (initial tank only);
/// canonical replay may still reject impossible meetings and other effects.
fn patrol_actions(
    ctx: &SolverContext<'_>,
    agent_idx: usize,
    order: &[usize],
    spots: &[HexCoord],
    fuel_weight: u32,
    cache: &mut PathCache,
) -> Vec<Action> {
    let id = ctx.agent_ids[agent_idx];
    let Some(agent) = ctx.state.agents.iter().find(|a| a.id == id) else {
        return vec![Action::wait(id); ctx.day_steps as usize];
    };
    let map = &ctx.state.setting.map;
    let mut actions = Vec::new();
    let mut current = agent.pos;
    let mut spent_time = 0u32;
    let mut spent_fuel = 0u32;
    'targets: for &target_index in order {
        let Some(&target) = spots.get(target_index) else {
            continue;
        };
        let Some(directions) = path(cache, map, current, target, fuel_weight) else {
            continue;
        };
        for direction in directions {
            let Some(origin) = map.get(current) else { break 'targets };
            let Some(step_time) = engine::wire_plan::terrain_step_cost(origin.terrain) else {
                break 'targets;
            };
            let next_fuel = spent_fuel.saturating_add(engine::rules::fuel_cost(origin.terrain));
            if spent_time.saturating_add(step_time) > ctx.day_steps || next_fuel > agent.fuel {
                break 'targets;
            }
            actions.push(Action::step(id, direction));
            spent_time += step_time;
            spent_fuel = next_fuel;
            current = direction.step(current);
        }
    }
    while spent_time < ctx.day_steps {
        actions.push(Action::wait(id));
        spent_time += 1;
    }
    actions
}

fn build_plan(
    ctx: &SolverContext<'_>,
    baseline: &AgentActions,
    order: &VisitOrder,
    spots: &[HexCoord],
    fuel_weight: u32,
    cache: &mut PathCache,
) -> AgentActions {
    let n = ctx.agent_ids.len();
    let mut result = baseline.actions.clone();
    for (i, id) in ctx.agent_ids.iter().copied().enumerate() {
        let is_patrol = ctx
            .state
            .agents
            .iter()
            .any(|a| a.id == id && a.kind == AgentKind::Patrol);
        if !is_patrol {
            continue;
        }
        let moves = patrol_actions(ctx, i, &order[i], spots, fuel_weight, cache);
        for tick in 0..ctx.day_steps as usize {
            result[tick * n + i] =
                moves.get(tick).copied().unwrap_or_else(|| Action::wait(id));
        }
    }
    BudgetMode::Official.apply(ctx, AgentActions::new(result))
}

type Score = (u32, u32, u32, u32);
fn replay_score(ctx: &SolverContext<'_>, candidate: &AgentActions) -> Option<Score> {
    let mut state = ctx.state.clone();
    let sim = MatchSim::default();
    for actions in candidate.actions.chunks_exact(ctx.agent_ids.len()) {
        sim.step(&mut state, &AgentActions::new(actions.to_vec()));
    }
    if state.illegal_action_count > ctx.state.illegal_action_count
        || state.fuel_short_count > ctx.state.fuel_short_count
        || state.rejected_response_count > ctx.state.rejected_response_count
    {
        return None;
    }
    let variety = state.series_taken.len() as u32;
    let today = state.series_acquired_by_day.get(&ctx.day).map_or(0, |s| s.len() as u32);
    let balls: u32 = state.agents.iter().map(engine::model::Agent::total_inventory).sum();
    let fuel: u32 = state
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Patrol)
        .map(|a| a.fuel.min(100))
        .sum();
    Some((variety, today, balls, fuel))
}

fn energy(s: Score) -> f64 {
    // Soft search objective ONLY; final retention always uses official V/D/B.
    f64::from(s.0) * 3_000.0 + f64::from(s.1) * 150.0 + f64::from(s.2)
        + f64::from(s.3) / 250.0
}

struct StructuralCandidate {
    trials: u32,
    budget_ms: u32,
    seed: u64,
}
impl Solver for StructuralCandidate {
    fn name(&self) -> &'static str {
        "structural"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let baseline = CoverageCandidate { park_risk: true, variant: 0, fuel_aware: false }
            .decide(ctx);
        if ctx.agent_ids.is_empty() || ctx.day_steps == 0 {
            return baseline;
        }
        let Some(mut best_score) = replay_score(ctx, &baseline) else {
            return baseline;
        };
        let map = &ctx.state.setting.map;
        let spots: Vec<_> = map
            .cells()
            .iter()
            .filter_map(|c| c.spot.as_ref().map(|s| s.pos))
            .collect();
        let patrols: Vec<usize> = ctx
            .agent_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| ctx.state.agents.iter().find(|a| a.id == *id)
                .filter(|a| a.kind == AgentKind::Patrol).map(|_| i))
            .collect();
        if spots.is_empty() || patrols.is_empty() {
            return baseline;
        }
        let started = Instant::now();
        let deadline = if self.trials > 0 {
            None // fixed-trial mode contains no timing decisions
        } else {
            let safety = ctx.remaining_ms.saturating_sub(5_000);
            Some(started + Duration::from_millis(u64::from(self.budget_ms.min(safety))))
        };
        let mut rng = Rng::new(self.seed);
        let mut best = baseline.clone();
        let mut current = baseline_visit_order(ctx, &baseline, &spots);
        let mut current_score = best_score;
        let mut cache = PathCache::new();
        let mut trial = 0u32;
        loop {
            if self.trials > 0 {
                if trial >= self.trials { break; }
            } else if deadline.is_some_and(|limit| Instant::now() >= limit) {
                break;
            }
            trial += 1;
            let mut proposal = current.clone();
            mutate(&mut proposal, &patrols, spots.len(), &mut rng);
            // Alternate shortest-time and fuel-aware route choices.
            let fuel_weight = match rng.index(3) { 0 => 0, 1 => 4, _ => 12 };
            let plan = build_plan(ctx, &baseline, &proposal, &spots, fuel_weight, &mut cache);
            let Some(score) = replay_score(ctx, &plan) else {
                continue;
            };
            // Official lexicographic acceptance, with fuel only as a tie-break.
            if score > best_score {
                best_score = score;
                best = plan.clone();
            }
            let temperature = 2.0 + 80.0 / (1.0 + f64::from(trial) / 32.0);
            let delta = energy(score) - energy(current_score);
            if delta >= 0.0 || rng.probability() < (delta / temperature).exp() {
                current = proposal;
                current_score = score;
            }
        }
        // No structural mutation can worsen the returned official local score.
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateRegistry;

    #[test]
    fn structural_candidate_is_registered_and_deterministic() {
        let registry = CandidateRegistry::builtin();
        let spec = registry.build("structural:trials=8:seed=42:budget=official").unwrap();
        assert!(spec.build_solver().is_ok());
        assert!(registry.build("structural:budget=official").unwrap().build_solver().is_err());
    }

    #[test]
    fn structural_never_returns_worse_than_coverage() {
        let case = crate::CaseSpec::from_bundled_fixture("large-32x32").unwrap();
        let pattern = crate::kinds::parse_kinds("P,S,P,S,P").unwrap();
        let kinds = crate::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
        let team = domain::PlayerId::new("player-a").unwrap();
        let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).unwrap();
        let state = engine::model::MatchState::new(setting).unwrap();
        let ctx = engine::model::SolverContext::from_state(&state);
        let baseline = CoverageCandidate { park_risk: true, variant: 0, fuel_aware: false }
            .decide(&ctx);
        let solver = CandidateRegistry::builtin()
            .build("structural:trials=16:seed=42:budget=official")
            .unwrap()
            .build_solver()
            .unwrap();
        let result = solver.decide(&ctx);
        let bs = replay_score(&ctx, &baseline).unwrap();
        let rs = replay_score(&ctx, &result).unwrap();
        assert!(rs >= bs, "official local score must not regress: {rs:?} vs {bs:?}");
        assert!(result.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
    }
}
