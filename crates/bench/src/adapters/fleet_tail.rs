//! Tail extension: spend a finished day plan's idle time on more balls
//! (Issue #406).
//!
//! A day plan leaves patrols standing once their route ends: the search
//! values tomorrow's tank over a ball it could still reach today. In the
//! round-one live run (2026-10-10, match 3) patrols stood for 16-20 ticks
//! with a tank a Supply had just filled while stocked spots lay two moves
//! away.
//!
//! This pass takes a replay-clean plan and, per Patrol, looks at the time
//! after its last move has landed and its last refuel is done. From there it
//! tries a path to every stocked spot the patrol has not picked today and
//! replays the whole day through [`engine::MatchSim`] for each. An extension
//! is kept only when the replay is clean, variety and today's series do not
//! drop and today's balls rise. On the last day the tank is free; on other
//! days an extension may burn at most `fuel_per_ball` fuel per ball gained.
//!
//! Nothing before a patrol's free tick is changed, so every pick and every
//! supply contact of the original plan still happens.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use engine::MatchSim;
use engine::model::{
    Action, AgentActions, AgentKind, Direction, HexCoord, MatchState, PendingAction, SolverContext,
    Terrain,
};
use engine::rules::fuel_cost;
use engine::transitions::StepEvent;

/// Whole-day replays one call may spend: each is one `MatchSim` day.
const MAX_REPLAYS: usize = 600;

/// One started move: `(start tick, landing tick, direction)`.
type Move = (u32, u32, Direction);

/// One day replayed through the official simulator, with what the tail
/// search needs to know about it.
struct Trace {
    clean: bool,
    /// `(variety, today's series, today's balls)`.
    score: (usize, usize, u32),
    pos: Vec<HexCoord>,
    fuel: Vec<u32>,
    moves: Vec<Vec<Move>>,
    /// Tick from which the agent neither moves nor is refuelled.
    free_at: Vec<u32>,
    picked: BTreeSet<(u8, HexCoord)>,
    taken: BTreeMap<HexCoord, u32>,
}

fn direction(from: HexCoord, to: HexCoord) -> Option<Direction> {
    Direction::ALL.into_iter().find(|d| d.step(from) == to)
}

fn step_cost(state: &MatchState, at: HexCoord) -> Option<u32> {
    state.setting.map.get(at).and_then(|c| engine::wire_plan::terrain_step_cost(c.terrain))
}

fn trace(ctx: &SolverContext<'_>, plan: &AgentActions) -> Option<Trace> {
    let n = ctx.agent_ids.len();
    let steps = ctx.day_steps;
    if n == 0 || plan.actions.len() != n * steps as usize {
        return None;
    }
    let mut state = ctx.state.clone();
    let before = (state.illegal_action_count, state.rejected_response_count);
    let held: u32 = state.agents.iter().map(engine::Agent::total_inventory).sum();
    let sim = MatchSim::default();
    let mut moves: Vec<Vec<Move>> = vec![Vec::new(); n];
    let mut landed = vec![0usize; n];
    let mut free_at = vec![0u32; n];
    let mut picked = BTreeSet::new();
    let mut taken: BTreeMap<HexCoord, u32> = BTreeMap::new();
    for (now, step) in (0..steps).zip(plan.actions.chunks(n)) {
        let last = now + 1 == steps;
        let events = sim.step(&mut state, &AgentActions::new(step.to_vec()));
        for event in events {
            match event {
                StepEvent::Moved { agent_id, from, to } => {
                    let a = usize::from(agent_id);
                    landed[a] += 1;
                    // Only a move started at the day's last action phase is
                    // not on record yet: the day rolled over before it could
                    // be read back from `in_flight`.
                    if landed[a] > moves[a].len() {
                        moves[a].push((now, steps, direction(from, to)?));
                        free_at[a] = steps;
                    }
                }
                StepEvent::Picked { agent_id, coord, series } => {
                    if series == u32::MAX {
                        // A refuel. The last call reflects two ticks; the
                        // later one is the safe reading.
                        let a = usize::from(agent_id);
                        free_at[a] = free_at[a].max(if last { steps } else { now });
                    } else {
                        picked.insert((agent_id, coord));
                        *taken.entry(coord).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
        if last {
            break;
        }
        for (a, agent) in state.agents.iter().enumerate() {
            if let Some(PendingAction::Move { to, due_step, .. }) = state.in_flight.get(&agent.id) {
                if moves[a].last().is_none_or(|m| m.1 < *due_step) {
                    moves[a].push((now, *due_step, direction(agent.pos, *to)?));
                    free_at[a] = free_at[a].max(*due_step);
                }
            }
        }
    }
    Some(Trace {
        clean: (state.illegal_action_count, state.rejected_response_count) == before,
        score: (
            state.series_taken.len(),
            state.series_acquired_by_day.get(&ctx.day).map_or(0, BTreeSet::len),
            state
                .agents
                .iter()
                .map(engine::Agent::total_inventory)
                .sum::<u32>()
                .saturating_sub(held),
        ),
        pos: state.agents.iter().map(|a| a.pos).collect(),
        fuel: state.agents.iter().map(|a| a.fuel).collect(),
        moves,
        free_at,
        picked,
        taken,
    })
}

/// The sequential answer list that starts every move at its recorded tick.
fn queue(id: u8, moves: &[Move], steps: u32) -> Option<Vec<Action>> {
    let mut out = Vec::with_capacity(steps as usize);
    let mut tick = 0;
    for &(start, due, dir) in moves {
        if start < tick || due > steps {
            return None;
        }
        out.extend((tick..start).map(|_| Action::wait(id)));
        out.push(Action::step(id, dir));
        tick = due;
    }
    if out.len() > steps as usize {
        return None;
    }
    out.resize(steps as usize, Action::wait(id));
    Some(out)
}

fn assemble(lists: &[Vec<Action>], steps: u32) -> AgentActions {
    let mut out = Vec::with_capacity(lists.len() * steps as usize);
    for step in 0..steps as usize {
        out.extend(lists.iter().map(|list| list[step]));
    }
    AgentActions::new(out)
}

/// Cheapest paths from `source` over today's terrain: by time when `lean`
/// is false, by fuel otherwise. Returns `cell -> (time, fuel, previous
/// cell, direction taken)`.
fn paths(
    state: &MatchState,
    source: HexCoord,
    lean: bool,
) -> BTreeMap<HexCoord, (u32, u32, HexCoord, Direction)> {
    let map = &state.setting.map;
    let mut best: BTreeMap<HexCoord, (u64, u32, u32)> = BTreeMap::from([(source, (0, 0, 0))]);
    let mut prev = BTreeMap::new();
    let mut heap = BinaryHeap::from([Reverse((0u64, source))]);
    while let Some(Reverse((key, at))) = heap.pop() {
        let (k, time, fuel) = best[&at];
        if k != key {
            continue;
        }
        let Some(cell) = map.get(at) else { continue };
        let Some(t) = engine::wire_plan::terrain_step_cost(cell.terrain) else { continue };
        let f = fuel_cost(cell.terrain);
        let edge = if lean {
            u64::from(f) * 1024 + u64::from(t)
        } else {
            u64::from(t) * 1024 + u64::from(f)
        };
        for dir in Direction::ALL {
            let to = dir.step(at);
            if map.get(to).is_none_or(|c| c.terrain == Terrain::Pond) {
                continue;
            }
            if best.get(&to).is_none_or(|b| key + edge < b.0) {
                best.insert(to, (key + edge, time + t, fuel + f));
                prev.insert(to, (time + t, fuel + f, at, dir));
                heap.push(Reverse((key + edge, to)));
            }
        }
    }
    prev
}

/// Extend the idle tails of `plan`. Returns the plan unchanged when nothing
/// verified improves it. `fuel_per_ball == 0` touches the last day only.
#[must_use]
pub fn extend(ctx: &SolverContext<'_>, plan: AgentActions, fuel_per_ball: u32) -> AgentActions {
    let steps = ctx.day_steps;
    let last_day = ctx.day + 1 >= ctx.state.setting.num_days;
    if !last_day && fuel_per_ball == 0 {
        return plan;
    }
    let Some(mut base) = trace(ctx, &plan) else { return plan };
    if !base.clean {
        return plan;
    }
    // Work on the timed form of the plan; it must replay to the same day.
    let n = ctx.agent_ids.len();
    let mut lists: Vec<Vec<Action>> = Vec::with_capacity(n);
    for (a, &id) in ctx.agent_ids.iter().enumerate() {
        let Some(list) = queue(id, &base.moves[a], steps) else { return plan };
        lists.push(list);
    }
    match trace(ctx, &assemble(&lists, steps)) {
        Some(t) if t.clean && t.score == base.score && t.pos == base.pos && t.fuel == base.fuel => {
        }
        _ => return plan,
    }
    let spots: Vec<(HexCoord, u32)> = ctx
        .state
        .setting
        .map
        .cells()
        .iter()
        .filter_map(|c| c.spot.as_ref().map(|s| (c.coord, s.max_stock)))
        .collect();
    let mut replays = 0;
    let mut improved = false;
    loop {
        // Best verified extension of this round: lowest cost per ball.
        let mut best: Option<((usize, Reverse<u64>, u32), Vec<Vec<Action>>, Trace)> = None;
        for (a, agent) in ctx.state.agents.iter().enumerate() {
            let free = base.free_at[a];
            if agent.kind != AgentKind::Patrol || free >= steps {
                continue;
            }
            for lean in [false, true] {
                let reach = paths(ctx.state, base.pos[a], lean);
                for &(spot, stock) in &spots {
                    if base.picked.contains(&(agent.id, spot))
                        || base.taken.get(&spot).copied().unwrap_or(0) >= stock
                    {
                        continue;
                    }
                    let Some(&(time, fuel, _, _)) = reach.get(&spot) else { continue };
                    if free + time > steps || fuel > base.fuel[a] || replays >= MAX_REPLAYS {
                        continue;
                    }
                    let mut legs = Vec::new();
                    let mut at = spot;
                    while at != base.pos[a] {
                        let Some(&(_, _, from, dir)) = reach.get(&at) else { break };
                        legs.push((from, dir));
                        at = from;
                    }
                    legs.reverse();
                    let mut moves = base.moves[a].clone();
                    let mut tick = free;
                    for (from, dir) in legs {
                        let Some(cost) = step_cost(ctx.state, from) else { break };
                        moves.push((tick, tick + cost, dir));
                        tick += cost;
                    }
                    let Some(list) = queue(agent.id, &moves, steps) else { continue };
                    let mut trial = lists.clone();
                    trial[a] = list;
                    replays += 1;
                    let Some(t) = trace(ctx, &assemble(&trial, steps)) else { continue };
                    let gain = t.score.2.saturating_sub(base.score.2);
                    let burnt = base.fuel[a].saturating_sub(t.fuel[a]);
                    if !t.clean
                        || t.score.0 < base.score.0
                        || t.score.1 < base.score.1
                        || gain == 0
                        || (!last_day && burnt > fuel_per_ball.saturating_mul(gain))
                    {
                        continue;
                    }
                    let cost = u64::from(if last_day { time } else { burnt });
                    let key = (t.score.1, Reverse(cost * 1000 / u64::from(gain)), gain);
                    if best.as_ref().is_none_or(|b| key > b.0) {
                        best = Some((key, trial, t));
                    }
                }
            }
        }
        let Some((_, trial, t)) = best else { break };
        lists = trial;
        base = t;
        improved = true;
    }
    if improved { assemble(&lists, steps) } else { plan }
}
