//! Daily coverage first, followed by marginal harvest, with joint fuel replay.
//! Planning uses the same engine rules as evaluation and live `CandidateBridge`.
//!
//! # The official tick clock (Issue #342 re-tune)
//!
//! The official day is a tick timeline: a `Move` reserved at tick `t` lands at
//! `t + terrain_step_cost(origin)` (Q6/Q10), and the daily answer is a
//! *sequential* per-agent action list — the next order starts when the
//! previous one completes (Q23/Q24). Consecutive list entries therefore pack
//! back-to-back and the weighted sum `Σ step costs + Σ waits` is both the
//! budget and the timeline. Planning here mirrors exactly that: each agent's
//! decisions are made at its **landing ticks** against a shadow timeline, and
//! the decided actions are emitted as one packed per-agent sequence (waits
//! only fill the tail after an agent's budget is spent). Deciding per element
//! from the live state — the pre-#342 loop — instead acted on the stale
//! pre-landing `pos`, double-booked in-flight moves, and starved long legs
//! through its route-budget filter (701 day 0: 31/46/5/7/9 moves with 83 of
//! 146 weighted units idle, versus the element-clock planner's
//! 121/120/99/103/103 moves consuming the budget exactly).
//!
//! `park=none` (default) harvests the nearest reserved spot after coverage.
//! `park=far` keeps a day-scoped claim set across steps and deprioritizes
//! already-claimed series, so patrols spread across unclaimed series instead
//! of converging (Issue #331). The spreading window opens only on medium+
//! days (today and tomorrow both >= 70 steps) whose map edge reaches 24.
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{
    Action, ActionKind, Agent, AgentActions, AgentKind, Direction, HexCoord, MapGrid, Solver,
    SolverContext, Terrain,
};
use engine::rules::{agent_at_daily_pick_cap, fuel_cost, move_fuel};
use engine::transitions::{phase_acquire, refill_fuel_if_supply_present};
fn step_cost(terrain: Terrain) -> u32 {
    engine::wire_plan::terrain_step_cost(terrain).unwrap_or(u32::MAX)
}

pub struct CoverageFactory;
const PARAMS: &[ParamSpec] = &[
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
    ParamSpec { key: "park", required: false, doc: "none (default) | far" },
    ParamSpec { key: "fuelaware", required: false, doc: "false (default) | true" },
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
            Some("far" | "risk") => true,
            Some(other) => {
                return Err(CandidateError::Build(format!("unknown park mode {other:?}")));
            }
        };
        let fuel_aware = match spec.param("fuelaware") {
            None | Some("false") => false,
            Some("true") => true,
            Some(other) => {
                return Err(CandidateError::Build(format!("unknown fuelaware mode {other:?}")));
            }
        };
        Ok(Box::new(CoverageCandidate { park_risk, variant: 0, fuel_aware }))
    }
}
pub struct CoverageCandidate {
    /// Use fuel-aware supply pairing (Issue #342).
    pub fuel_aware: bool,
    /// `park=far` / `park=risk` spreading mode (Issue #331). Post-#342 both
    /// park modes share the day-scoped series claim policy — the per-step
    /// re-decision loop whose claim expiry let `park=none` converge on
    /// already-claimed series was a defect of the pre-parity element clock,
    /// and the spreading window's penalty is subsumed by the persistent
    /// claim set. The parameter stays accepted so canonical ids keep their
    /// shape; it selects the same measured-best policy.
    pub(crate) park_risk: bool,
    /// Search-solver trial seed: rotates patrol order and jitters the
    /// coverage cost key when non-zero (release-side #312/#313 tooling).
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

/// Fuel-aware supply pairing (Issue #342).
/// Assigns supplies to fuel-needy patrols first, then by proximity.
/// This prevents the #339 scenario where one patrol runs out of fuel
/// while supplies are nearby but assigned to other patrols.
fn supply_pairs_fuel_aware(
    state: &engine::model::MatchState,
    patrols: &[usize],
) -> (Vec<Option<usize>>, BTreeSet<usize>) {
    let n = state.agents.len();
    let mut pairs = vec![None; n];
    let mut assigned = BTreeSet::new();

    // Sort patrols by fuel (lowest first = most needy).
    let mut patrol_order: Vec<usize> = patrols.to_vec();
    patrol_order.sort_by_key(|&p| (state.agents[p].fuel, p));

    // Assign supplies to neediest patrols first.
    let mut supply_indices: Vec<usize> =
        (0..n).filter(|&i| state.agents[i].kind == AgentKind::Supply).collect();

    for &p in &patrol_order {
        if supply_indices.is_empty() {
            break;
        }
        let paths = routes(&state.setting.map, &state.agents[p], true);
        let best = supply_indices
            .iter()
            .enumerate()
            .min_by_key(|&(_, &s)| (paths[index(&state.setting.map, state.agents[s].pos)].cost, s))
            .map(|(idx, _)| idx);
        if let Some(idx) = best {
            let s = supply_indices.remove(idx);
            pairs[s] = Some(p);
            assigned.insert(p);
        }
    }
    (pairs, assigned)
}

/// Coverage-target key policy for one day: the park spreading window plus the
/// search-trial jitter seed (Issue #331 + release-side #312/#313).
#[derive(Clone, Copy)]
struct TargetKey {
    /// Park-mode spreading across unclaimed series is active this day.
    spread_window: bool,
    /// Search-solver trial seed; 0 for every registered `coverage` candidate.
    variant: u64,
}

/// One patrol step: harvest in place, walk to a coverage target, or walk to
/// the nearest harvest target. `key.spread_window` enables park-mode spreading
/// across unclaimed series (Issue #331); the caller decides when that window
/// is open. `key.variant` jitters the coverage cost key for search-solver
/// trials (#312/#313). Updates `reserved` / `day_claimed` with the chosen
/// series.
fn patrol_step(
    state: &engine::model::MatchState,
    agent: &Agent,
    paths: &[Route],
    remaining: u32,
    key: TargetKey,
    day_claimed: &mut BTreeSet<u32>,
    reserved: &mut BTreeSet<u32>,
) -> Action {
    let map = &state.setting.map;
    let can_pick_here = map.get(agent.pos).and_then(|c| c.spot.as_ref()).is_some_and(|s| {
        s.current_stock > 0 && !agent_at_daily_pick_cap(agent, agent.pos, &state.spot_picks_today)
    });
    if can_pick_here {
        if let Some(spot) = map.get(agent.pos).and_then(|c| c.spot.as_ref()) {
            reserved.insert(spot.series);
        }
        return Action::wait(agent.id);
    }
    let target = map
        .cells()
        .iter()
        .filter_map(|c| c.spot.as_ref())
        .filter(|s| {
            s.current_stock > 0 && !agent_at_daily_pick_cap(agent, s.pos, &state.spot_picks_today)
        })
        .filter_map(|s| {
            let route = paths[index(map, s.pos)];
            (route.cost <= remaining && route.first.is_some()).then_some((
                reserved.contains(&s.series),
                // Park: spread across unclaimed series only on medium+ days
                // whose tomorrow is also medium+. Short days scatter patrols;
                // spreading the day before a short day can strand the next
                // start (Issue #331).
                key.spread_window && day_claimed.contains(&s.series),
                route.cost,
                s.series,
                index(map, s.pos),
                route.first,
            ))
        })
        .min_by_key(|x| {
            let jitter = if key.variant == 0 { 0 } else { mix(key.variant ^ x.4 as u64) % 8 };
            (x.0, x.1, u64::from(x.2) * (8 + jitter), x.3, x.4)
        });
    if let Some((_, _, _, series, _, first)) = target {
        reserved.insert(series);
        day_claimed.insert(series);
        return legal_step(agent, first, map, remaining);
    }
    Action::wait(agent.id)
}

impl Solver for CoverageCandidate {
    fn name(&self) -> &'static str {
        "coverage"
    }
    #[allow(
        clippy::too_many_lines,
        reason = "daily decide loop grew with the Issue #342/#339 fuel-aware pairing and fuel-return override"
    )]
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let mut state = fresh_day_state(ctx);
        let n = state.agents.len();
        let day_steps = ctx.day_steps;
        let patrols = patrol_order(&state, self.variant);
        // Use fuel-aware pairing only when fuel pressure exists (Issue #342).
        // This avoids disrupting normal assignment when no fuel issue exists.
        let min_patrol_fuel =
            patrols.iter().map(|&p| state.agents[p].fuel).min().unwrap_or(u32::MAX);
        let has_fuel_pressure = min_patrol_fuel < 50; // threshold
        let (pairs, assigned) = if self.fuel_aware && has_fuel_pressure {
            supply_pairs_fuel_aware(&state, &patrols)
        } else {
            supply_pairs(&state, &patrols)
        };
        let tomorrow_budget = {
            let tomorrow = ctx.day.saturating_add(1);
            if tomorrow < state.setting.num_days {
                state.setting.day_steps_for(tomorrow)
            } else {
                0
            }
        };
        // Park spreading opens only on medium+ days whose tomorrow is also
        // medium+, and only on maps where nearest-first cannot cover all
        // series (Issue #331). Small boards are covered by greedy proximity.
        let size = &state.setting.map.size;
        let spread_window = self.park_risk
            && (usize::from(size.width) >= 24 || usize::from(size.height) >= 24)
            && day_steps >= 70
            && tomorrow_budget >= 70;

        // Shadow timeline (the official tick clock, Q6/Q10/Q23/Q24): a
        // decision is made only when the agent is free, from its **landing**
        // position and fuel — never from a stale pre-landing cell. Because the
        // decided actions execute back-to-back, the tick an agent frees up
        // equals the weighted budget it has consumed; `clock[i]` is both.
        let mut agents = state.agents.clone();
        let mut clock = vec![0u32; n];
        // Destination of an in-flight move (the landing cell), until the
        // landing reflection processes it at tick `clock[i]`. Supplies follow
        // this cell, not the stale `pos` of a moving patrol.
        let mut landings = vec![None::<HexCoord>; n];
        // Day-scoped claim set: once a patrol targets a series, no other
        // patrol re-targets it. Park mode keeps claims across the whole day so
        // agents spread across series instead of converging (Issue #331).
        let mut day_claimed: BTreeSet<u32> =
            state.series_acquired_by_day.get(&ctx.day).cloned().unwrap_or_default();
        let mut reserved = day_claimed.clone();
        // Per-agent sequential answer lists (the wire's per-agent bodies).
        let mut lists: Vec<Vec<Action>> = vec![Vec::new(); n];
        // Decision priority: patrols in the rotated `patrols` order, then
        // supplies in id order — the same priority the per-step loop gave the
        // patrols' claims (the search-trial rotation, #312/#313, stays live).
        let rank: Vec<usize> = (0..n)
            .map(|i| patrols.iter().position(|&p| p == i).unwrap_or(patrols.len() + i))
            .collect();
        let mut events = Vec::new(); // sink for acquire / refuel events

        while let Some(i) =
            (0..n).filter(|&i| clock[i] < day_steps).min_by_key(|&i| (clock[i], rank[i]))
        {
            // Reflection at tick `clock[i]` (Q6 order: fuel → move →
            // acquire → refuel). A landing reflection processes the in-flight
            // move; a standing reflection (tick >= 1, agent free) acquires
            // and refuels where it stands. `phase_acquire` /
            // `refill_fuel_if_supply_present` are the simulator's own
            // primitives, so the shadow's pick, cap and refuel bookkeeping is
            // the official one by construction.
            if let Some(to) = landings[i].take() {
                if agents[i].kind == AgentKind::Patrol {
                    let origin = state.setting.map.get(agents[i].pos).expect("agent on map");
                    agents[i].fuel = agents[i].fuel.saturating_sub(fuel_cost(origin.terrain));
                }
                agents[i].pos = to;
                let standing = agents[i].pos;
                phase_acquire(&agents[i], &mut state, &mut events);
                refill_fuel_if_supply_present(&mut agents, standing, &mut events);
                claim_picked_series(&agents[i], &state, &mut day_claimed, &mut reserved);
            } else if clock[i] > 0 {
                let standing = agents[i].pos;
                phase_acquire(&agents[i], &mut state, &mut events);
                refill_fuel_if_supply_present(&mut agents, standing, &mut events);
                claim_picked_series(&agents[i], &state, &mut day_claimed, &mut reserved);
            }

            let remaining = day_steps - clock[i];
            let action = if agents[i].kind == AgentKind::Patrol {
                // Fuel-return override (Issue #339): a patrol with critically
                // low fuel heads for its paired Supply's **landing cell**
                // instead of pursuing targets — the Supply is chasing but
                // never catching a target-pursuing patrol, so the return must
                // be the patrol's own decision. Truth is the Supply's landing
                // cell (`landings[s]`), never its stale pre-landing `pos`.
                let fuel_return_supply = if self.fuel_aware && agents[i].fuel < 30 {
                    (0..n).find(|&s| pairs[s] == Some(i))
                } else {
                    None
                };
                if let Some(s) = fuel_return_supply {
                    let goal = landings[s].unwrap_or(agents[s].pos);
                    let paths = routes(&state.setting.map, &agents[i], true);
                    legal_step(
                        &agents[i],
                        paths[index(&state.setting.map, goal)].first,
                        &state.setting.map,
                        remaining,
                    )
                } else {
                    let paths = routes(&state.setting.map, &agents[i], assigned.contains(&i));
                    patrol_step(
                        &state,
                        &agents[i],
                        &paths,
                        remaining,
                        TargetKey { spread_window, variant: self.variant },
                        &mut day_claimed,
                        &mut reserved,
                    )
                }
            } else if let Some(p) = pairs[i] {
                // Follow the paired patrol's landing cell: where the patrol
                // will stand when its in-flight move completes, or where it
                // stands when it is free.
                let goal = landings[p].unwrap_or(agents[p].pos);
                let paths = routes(&state.setting.map, &agents[i], false);
                legal_step(
                    &agents[i],
                    paths[index(&state.setting.map, goal)].first,
                    &state.setting.map,
                    remaining,
                )
            } else {
                Action::wait(agents[i].id)
            };

            match action.kind {
                ActionKind::Move(dir) => {
                    let origin = state.setting.map.get(agents[i].pos).expect("agent on map");
                    // `legal_step` already bounded this by `remaining`, so the
                    // move always lands inside the day (`ReserveReject::
                    // Truncated` can never fire on the emitted list).
                    clock[i] += step_cost(origin.terrain);
                    landings[i] = Some(dir.step(agents[i].pos));
                }
                ActionKind::Wait => {
                    clock[i] += 1;
                    landings[i] = None;
                }
            }
            lists[i].push(action);
        }

        // Assemble the step-major day plan: each agent's decided actions pack
        // into consecutive blocks (the sequential answer the official clock
        // executes back-to-back); once its budget is spent the remaining
        // blocks are idle waits. Re-normalizing is then a no-op: every move
        // fits the weighted budget and the trailing wait runs trim to the
        // same blocks (`normalize_day_plan` never drops a planned move).
        let mut output = Vec::with_capacity(n * day_steps as usize);
        for step in 0..day_steps as usize {
            for i in 0..n {
                output.push(
                    lists[i].get(step).copied().unwrap_or_else(|| Action::wait(agents[i].id)),
                );
            }
        }
        // Apply official budget truncation so the plan is always within the
        // terrain-weighted day budget. Without this the server rejects the
        // plan with `revision = -1` (Issue #331 live match 1796). This is the
        // single normalization path: `BudgetMode::Official` re-runs
        // `engine::wire_plan::normalize_day_plan`, the same rule the wire
        // applies (#255 / #324), so decide() output is a fixed point of it.
        let plan = AgentActions::new(output);
        crate::adapters::walk::BudgetMode::Official.apply(ctx, plan)
    }
}

/// Record the series an agent just picked as claimed for today, so no patrol
/// re-targets it. `phase_acquire` keeps the official cap / stock bookkeeping
/// in `state`; this mirrors the claim side of the old per-step loop.
fn claim_picked_series(
    agent: &Agent,
    state: &engine::model::MatchState,
    day_claimed: &mut BTreeSet<u32>,
    reserved: &mut BTreeSet<u32>,
) {
    if state.spot_picks_today.get(&(agent.id, agent.pos)).copied().unwrap_or(0) > 0 {
        if let Some(spot) = state.setting.map.get(agent.pos).and_then(|c| c.spot.as_ref()) {
            reserved.insert(spot.series);
            day_claimed.insert(spot.series);
        }
    }
}

pub(crate) fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn patrol_order(state: &engine::model::MatchState, variant: u64) -> Vec<usize> {
    let mut patrols: Vec<_> =
        (0..state.agents.len()).filter(|&i| state.agents[i].kind == AgentKind::Patrol).collect();
    if !patrols.is_empty() {
        let rotation = usize::try_from(variant % patrols.len() as u64).expect("bounded rotation");
        patrols.rotate_left(rotation);
    }
    patrols
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
