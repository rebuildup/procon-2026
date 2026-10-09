//! Single-step transition primitives.
//!
//! Each function corresponds to one phase of Q&A Q6's order. The official
//! day is a tick timeline `0..=day_steps` where tick 0 is action-phase-only,
//! ticks `1..=N-1` run reflection → action, and the final tick `N` runs the
//! reflection only:
//!
//! 1. [`phase_wait`] — interpret missing/invalid actions as `Wait`.
//! 2. [`try_reserve_move`] — accept a `Move` at the action phase. The move is
//!    *reserved*; it lands at the reflection of
//!    `tick + terrain_step_cost(origin)` (Q10 / 募集要項図4).
//! 3. [`apply_due_moves`] — the reflection half of the move: fuel is charged
//!    and the position updates **at the due tick**, in that order (Q6:
//!    燃料の消費 → 移動の反映). Out-of-bounds / impassable / fuel-short
//!    orders are rejected at reservation and reported as
//!    `StepEvent::IllegalMove`.
//! 4. [`phase_spawn`] — refill spot stock at the start of a new day.
//! 5. [`phase_acquire`] — pick up balls from spots the agents stand on.
//!    Patrol only: `AgentKind::Supply` cannot collect 玉 (issue #234).
//! 6. [`phase_fuel`] — deduct the fuel cost of a move at its reflection.
//! 7. [`refill_fuel_if_supply_present`] — same-cell supply contact refuels
//!    after the acquire (Q6: 燃料の補給 is last of the four).
//! 8. [`record_stay_steps`] — traffic counts *stay* steps at the post-move
//!    position of every reflection (Q27).
//! 9. [`phase_traffic_update`] — at end of day, recompute every road cell's
//!    [`TrafficState`] from the all-team stay-step average over the past 2
//!    days (Q&A Q13 + 募集要項「道路セルと交通量」).
//!
//! The functions are independent so tests can exercise each phase in
//! isolation; [`super::sim::MatchSim::step`] stitches them together in the
//! Q6 order.

use std::collections::{BTreeMap, HashMap};

use crate::model::{
    Action, ActionKind, Agent, AgentKind, Cell, Direction, HexCoord, MapGrid, MatchState,
    PendingAction, Spot, Terrain, TrafficState,
};
use crate::rules::{
    SPOT_DAILY_PICK_PER_AGENT, TrafficThresholds, agent_at_daily_pick_cap, fuel_cost,
};

// ---------------------------------------------------------------------------
// Outcome events
// ---------------------------------------------------------------------------

/// One discrete thing that happened during a step. Useful for tests and
/// (later) replay inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepEvent {
    /// An agent's requested action was not legal; it was downgraded to a
    /// no-op `Wait` (Q&A Q6 — invalid answers degrade rather than abort the
    /// whole day).
    IllegalMove { agent_id: u8, reason: String },
    /// An agent moved from `from` to `to`.
    Moved { agent_id: u8, from: HexCoord, to: HexCoord },
    /// An agent stayed put (action was `Wait`, or was downgraded).
    Waited { agent_id: u8 },
    /// An agent picked up a ball of `series` at `coord`.
    Picked { agent_id: u8, coord: HexCoord, series: u32 },
    /// A patrol agent ran out of fuel and could not complete its move.
    FuelShort { agent_id: u8 },
    /// The daily pickup cap was hit — no pick happened this step.
    PickCapReached { agent_id: u8, coord: HexCoord },
    /// The end-of-day traffic recompute produced these new road states.
    TrafficUpdated { updates: Vec<(HexCoord, TrafficState)> },
}

// ---------------------------------------------------------------------------
// 1. Wait — interpret the action list as a canonical per-agent map.
// ---------------------------------------------------------------------------

/// Filter the action list down to a canonical `(agent_id, action)` map.
/// Unknown agent ids and duplicate entries are dropped (treated as `Wait`).
///
/// The output is ordered by agent id. `MatchSim::step` iterates it to emit
/// moves, so a `HashMap` here made the `Vec<StepEvent>` order depend on the
/// per-process hash seed: same input, different event order between runs.
#[must_use]
pub fn phase_wait(actions: &[Action], known_ids: &[u8]) -> BTreeMap<u8, Action> {
    let mut out = BTreeMap::new();
    for action in actions {
        if !known_ids.contains(&action.agent_id) {
            continue;
        }
        // Last write wins, which is fine for our shape rule.
        out.insert(action.agent_id, *action);
    }
    out
}

// ---------------------------------------------------------------------------
// 2. Move — apply moves with bounds + terrain + fuel checks.
// ---------------------------------------------------------------------------

/// Result of applying a single `Move` action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// Agent moved to `to`. Fuel cost is the move's terrain cost; caller
    /// deducts it in phase 5.
    Moved { to: HexCoord, cost: u32 },
    /// Action was downgraded to `Wait` for `reason`. Cost is 0.
    Wait { reason: &'static str },
}

impl MoveOutcome {
    #[must_use]
    pub const fn cost(&self) -> u32 {
        match self {
            Self::Moved { cost, .. } => *cost,
            Self::Wait { .. } => 0,
        }
    }

    #[must_use]
    pub const fn is_moved(&self) -> bool {
        matches!(self, Self::Moved { .. })
    }
}

/// Try to move one agent. Does *not* mutate state — the caller applies the
/// returned outcome (or treats the `Wait` outcome as a no-op).
#[must_use]
pub fn try_move(
    agent: &Agent,
    action: Action,
    map: &MapGrid,
    moves_out: &mut Vec<MoveOutcome>,
    events: &mut Vec<StepEvent>,
) -> MoveOutcome {
    let ActionKind::Wait = action.kind else {
        let ActionKind::Move(dir) = action.kind else { unreachable!() };
        let target = dir.step(agent.pos);
        match classify_move(agent, target, map) {
            MoveOk::Traversable { cost } => {
                moves_out.push(MoveOutcome::Moved { to: target, cost });
                events.push(StepEvent::Moved { agent_id: agent.id, from: agent.pos, to: target });
                return MoveOutcome::Moved { to: target, cost };
            }
            MoveOk::OutOfBounds => {
                events.push(StepEvent::IllegalMove {
                    agent_id: agent.id,
                    reason: format!("out of bounds: {target:?}"),
                });
            }
            MoveOk::Impassable(terrain) => {
                events.push(StepEvent::IllegalMove {
                    agent_id: agent.id,
                    reason: format!("impassable terrain {terrain:?} at {target:?}"),
                });
            }
            MoveOk::FuelShort { cost } => {
                events.push(StepEvent::FuelShort { agent_id: agent.id });
                events.push(StepEvent::IllegalMove {
                    agent_id: agent.id,
                    reason: format!("fuel short (need {cost}, have {})", agent.fuel),
                });
            }
        }
        return MoveOutcome::Wait { reason: "illegal" };
    };
    events.push(StepEvent::Waited { agent_id: agent.id });
    MoveOutcome::Wait { reason: "wait" }
}

/// Internal classification for a move attempt. Used by [`try_move`] to keep
/// the event emission logic in one place.
#[derive(Debug, Clone, Copy)]
enum MoveOk {
    Traversable { cost: u32 },
    OutOfBounds,
    Impassable(Terrain),
    FuelShort { cost: u32 },
}

fn classify_move(agent: &Agent, target: HexCoord, map: &MapGrid) -> MoveOk {
    let Some(cell) = map.get(target) else {
        return MoveOk::OutOfBounds;
    };
    if !cell.terrain.traversable() {
        return MoveOk::Impassable(cell.terrain);
    }
    let Some(origin) = map.get(agent.pos) else { return MoveOk::OutOfBounds };
    let Some(cost) = crate::rules::move_fuel(agent, origin.terrain, cell.terrain) else {
        // Settled boundary (Issue #342 P0-C): the need is the *departure*
        // cost only — the destination's fuel is never billed.
        return MoveOk::FuelShort { cost: fuel_cost(origin.terrain) };
    };
    MoveOk::Traversable { cost }
}

/// Apply the resolved move outcomes to the agent vector. Returns the fuel
/// costs in agent-id order so the fuel phase can deduct them deterministically.
pub fn apply_moves(agents: &mut [Agent], outcomes: &HashMap<u8, MoveOutcome>) -> Vec<(u8, u32)> {
    let mut costs = Vec::with_capacity(outcomes.len());
    for agent in agents.iter_mut() {
        if let Some(outcome) = outcomes.get(&agent.id) {
            if let MoveOutcome::Moved { to, .. } = outcome {
                agent.pos = *to;
            }
        }
    }
    for (id, outcome) in outcomes {
        costs.push((*id, outcome.cost()));
    }
    costs.sort_by_key(|(id, _)| *id);
    costs
}

// ---------------------------------------------------------------------------
// 2b. Reserve — the Q6 action phase accepts a move without applying it.
// ---------------------------------------------------------------------------

/// Why a `Move` order was refused at the action phase (Q6: an illegal or
/// step-mismatched order invalidates the WHOLE daily response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveReject {
    /// The destination is off the board or impassable terrain.
    Illegal,
    /// The departure terrain's step count does not fit in the rest of the
    /// day — the move could not land within the day's last reflection. The
    /// caller truncates the plan here (the same repair `wire_plan` applies
    /// before submission) rather than flagging the response.
    Truncated,
}

/// Validate and *reserve* one agent's `Move` at an action phase (Q6: 「移動
/// を予約,燃料は反映フェーズで消費」).
///
/// Nothing is mutated: the caller stores the returned [`PendingAction`] and
/// [`apply_due_moves`] charges the fuel and moves the agent at `due_step`.
/// `now` is the current tick; `day_limit` is the day's final tick
/// (`day_steps`) — a move whose due tick would land past it cannot be
/// reflected within the day.
///
/// Fuel is deliberately **not** checked here: the official executability
/// rule (`fuel >= fuel_cost(origin)`, Issue #342 P0-C) is evaluated at the
/// reflection that lands the move, so a supply contact on an intervening
/// tick can still pay for a move that was ordered with an empty tank
/// (Q22 — 燃料の消費は各ステップの補給より前; `wire_plan`'s cursor models
/// the same timing).
///
/// # Errors
/// [`ReserveReject`] with the official rejection class. `Illegal` invalidates
/// the whole response; `Truncated` is the element clock's budget repair (see
/// the variant docs). A fuel-short landing is flagged by [`apply_due_moves`].
pub fn try_reserve_move(
    agent: &Agent,
    dir: Direction,
    map: &MapGrid,
    now: u32,
    day_limit: u32,
) -> Result<PendingAction, ReserveReject> {
    let target = dir.step(agent.pos);
    let Some(origin) = map.get(agent.pos) else {
        return Err(ReserveReject::Illegal);
    };
    // The move lands after the *departure* terrain's step count elapses
    // (Q10 / 募集要項図4). `terrain_step_cost` is the same table the wire
    // budget bills (`crate::wire_plan::terrain_step_cost`).
    let Some(step_cost) = crate::wire_plan::terrain_step_cost(origin.terrain) else {
        return Err(ReserveReject::Illegal);
    };
    let Some(cell) = map.get(target) else {
        return Err(ReserveReject::Illegal);
    };
    if !cell.terrain.traversable() {
        return Err(ReserveReject::Illegal);
    }
    let Some(due_step) = now.checked_add(step_cost) else {
        return Err(ReserveReject::Truncated);
    };
    if due_step > day_limit {
        return Err(ReserveReject::Truncated);
    }
    Ok(PendingAction::Move { to: target, due_step, fuel_cost: fuel_cost(origin.terrain) })
}

/// Accept one action at the action phase: `Wait` always reserves;
/// `Move` goes through [`try_reserve_move`].
///
/// # Errors
/// The [`ReserveReject`] of a refused move. `Wait` never fails.
pub fn try_reserve(
    agent: &Agent,
    action: Action,
    map: &MapGrid,
    now: u32,
    day_limit: u32,
) -> Result<PendingAction, ReserveReject> {
    match action.kind {
        ActionKind::Wait => Ok(PendingAction::Wait { until_step: now.saturating_add(1) }),
        ActionKind::Move(dir) => try_reserve_move(agent, dir, map, now, day_limit),
    }
}

/// The reflection half of a move (Q6: 燃料の消費 → 移動の反映): charge the
/// departure terrain's fuel and land every in-flight move whose `due_step`
/// is `now`. Agents are processed in id order so the event stream stays
/// deterministic.
///
/// The fuel check runs here — at the landing reflection — so an intervening
/// supply contact can pay for the move (Q22). A move that still cannot pay
/// is a `FuelShort` rejection of the whole response and the agent stays put.
///
/// Waiting in-flight actions are simply retired. Move events are emitted at
/// the landing step — the point the official timeline considers the agent to
/// have arrived (and therefore the point stay-steps and うどん acquisition
/// see the new cell).
pub fn apply_due_moves(state: &mut MatchState, now: u32, events: &mut Vec<StepEvent>) {
    let due: Vec<u8> =
        state.in_flight.iter().filter(|(_, p)| p.due_step() == now).map(|(id, _)| *id).collect();
    for id in due {
        let Some(pending) = state.in_flight.remove(&id) else { continue };
        let PendingAction::Move { to, fuel_cost, .. } = pending else {
            continue; // Wait retires silently
        };
        let Some(agent) = state.agents.iter().find(|a| a.id == id) else {
            continue;
        };
        // Settled executability boundary (Issue #342 P0-C): the patrol must
        // afford the *departure* cost. Supply agents never pay fuel.
        if agent.kind != AgentKind::Supply && agent.fuel < fuel_cost {
            events.push(StepEvent::FuelShort { agent_id: id });
            events.push(StepEvent::IllegalMove {
                agent_id: id,
                reason: "fuel short at the move's reflection (Q6: whole response rejected)"
                    .to_owned(),
            });
            state.illegal_action_count = state.illegal_action_count.saturating_add(1);
            state.fuel_short_count = state.fuel_short_count.saturating_add(1);
            state.day_response_invalid = true;
            continue; // the agent stays at its origin
        }
        let Some(agent) = state.agents.iter_mut().find(|a| a.id == id) else {
            continue;
        };
        if agent.kind != AgentKind::Supply {
            agent.fuel = agent.fuel.saturating_sub(fuel_cost);
        }
        let from = agent.pos;
        agent.pos = to;
        events.push(StepEvent::Moved { agent_id: id, from, to });
    }
}

// ---------------------------------------------------------------------------
// 3. Spawn — refill spot stock at day boundaries.
// ---------------------------------------------------------------------------

/// Refill every spot on the map to its `max_stock`.
///
/// Spec: Q&A Q15 + 募集要項 — spots refill each day. Called once at the
/// start of each day (the `step` driver handles the day boundary).
pub fn phase_spawn(map: &mut MapGrid, events: &mut Vec<StepEvent>) {
    for cell in map.cells_mut() {
        if let Some(spot) = &mut cell.spot {
            spot.current_stock = spot.max_stock;
        }
    }
    let _ = events; // Spawn is currently silent; kept for symmetry.
}

/// Counterfactual day-boundary tank refill (Issue #243's `oracle_refill`).
///
/// Restores every **Patrol** agent to `fuel_cap` at a day boundary, the way a
/// hypothetical always-available supply point would. Supply agents are
/// skipped (their tank is `u32::MAX` already, so topping up is a no-op that
/// would only burn cycles).
///
/// This is **analysis-only**: the official rules refuel a Patrol tank only on
/// supply contact ([`refill_fuel_if_supply_present`]), and the ablation in
/// Issue #243 exists precisely to separate "fuel ran out" from "supply never
/// came". The knob defaults to `false` everywhere, so no default evidence
/// path can reach this function.
pub fn phase_oracle_refill(agents: &mut [Agent], events: &mut Vec<StepEvent>) {
    for agent in agents.iter_mut() {
        if agent.kind != AgentKind::Patrol || agent.fuel >= agent.fuel_cap {
            continue;
        }
        agent.fuel = agent.fuel_cap;
        events.push(StepEvent::Picked {
            agent_id: agent.id,
            coord: agent.pos,
            series: u32::MAX, // sentinel: refill, not a ball pick (same as supply refuel)
        });
    }
}

/// Reset the per-day traffic + pickup bookkeeping.
///
/// Also retires any in-flight / queued actions: the day's answer is consumed
/// by the day, and unstarted trailing actions are dropped exactly the way
/// `wire_plan::normalize_day_plan` truncates a plan to the day's budget.
pub fn begin_day(state: &mut MatchState) {
    state.traffic_yesterday = std::mem::take(&mut state.traffic_today);
    state.spot_picks_today.clear();
    state.in_flight.clear();
    state.action_queues.clear();
    state.day_response_invalid = false;
}

// ---------------------------------------------------------------------------
// 4. Acquire — Patrol agents on spots pick up balls.
// ---------------------------------------------------------------------------

/// Attempt to acquire a ball from the spot at `agent.pos`. Records the
/// per-day pickup count and emits the appropriate event.
///
/// Kind semantics (spec: `domain::AgentKind::Supply`, 募集要項 §エージェント) —
/// Supply agents **cannot** collect 玉. Issue #234: before this guard the
/// offline path let every agent collect, so a Supply standing on a stocked
/// spot decremented `current_stock` (stealing from Patrol) and scored into
/// `inventory` / `series_taken` / `series_acquired_by_day`, while live
/// (`from_wire` alternation: 3 Patrol / 2 Supply on a 5-agent team) did not.
/// The guard is the *first* statement so a Supply produces zero side effects:
/// no stock decrement, no inventory, no `series_*` bookkeeping, no `Picked`
/// event, and no `PickCapReached` event either.
pub fn phase_acquire(agent: &Agent, state: &mut MatchState, events: &mut Vec<StepEvent>) {
    if agent.kind == AgentKind::Supply {
        return;
    }
    let coord = agent.pos;
    let Some(cell) = state.setting.map.get(coord) else {
        return;
    };
    let Some(spot) = &cell.spot else {
        return;
    };
    if spot.current_stock == 0 {
        return;
    }
    if agent_at_daily_pick_cap(agent, coord, &state.spot_picks_today) {
        events.push(StepEvent::PickCapReached { agent_id: agent.id, coord });
        return;
    }
    // Mutate: drop one from spot, bump agent inventory, track series + time.
    let series = spot.series;
    if let Some(cell_mut) = state.setting.map.get_mut(coord) {
        if let Some(spot_mut) = cell_mut.spot.as_mut() {
            spot_mut.current_stock = spot_mut.current_stock.saturating_sub(1);
        }
    }
    let entry = state.spot_picks_today.entry((agent.id, coord)).or_insert(0);
    *entry += 1;
    if let Some(agent_mut) = state.agents.iter_mut().find(|a| a.id == agent.id) {
        *agent_mut.inventory.entry(series).or_insert(0) += 1;
    }
    state.series_taken.insert(series);
    state.series_first_taken_at.entry(series).or_insert((state.day, state.day_step));
    state.series_acquired_by_day.entry(state.day).or_default().insert(series);
    events.push(StepEvent::Picked { agent_id: agent.id, coord, series });
}

// ---------------------------------------------------------------------------
// 5. Fuel — deduct move costs from agents.
// ---------------------------------------------------------------------------

/// Deduct fuel costs collected during the move phase.
///
/// `Supply` agents are uncapped, so their `cost` is recorded but no fuel is
/// removed (Q&A — 補給車 has no fuel limit).
pub fn phase_fuel(agents: &mut [Agent], costs: &[(u8, u32)], events: &mut Vec<StepEvent>) {
    for (id, cost) in costs {
        if *cost == 0 {
            continue;
        }
        if let Some(agent) = agents.iter_mut().find(|a| a.id == *id) {
            if agent.kind == AgentKind::Supply {
                continue;
            }
            agent.fuel = agent.fuel.saturating_sub(*cost);
        }
    }
    let _ = events; // Reserved for future fuel-short reporting at deduction time.
}

/// Refill a patrol agent's fuel to its cap. Triggered when a supply agent is
/// on the same cell at the end of a step.
///
/// Spec: 募集要項「補給」 + issue #61 — supply agent sharing a cell refills
/// the patrol. We expose this as a primitive here so issue #61 can build on it.
pub fn refill_fuel_if_supply_present(
    agents: &mut [Agent],
    coord: HexCoord,
    events: &mut Vec<StepEvent>,
) {
    let has_supply = agents.iter().any(|a| a.pos == coord && a.kind == AgentKind::Supply);
    if !has_supply {
        return;
    }
    for agent in agents.iter_mut() {
        if agent.pos == coord && agent.kind == AgentKind::Patrol && agent.fuel < agent.fuel_cap {
            agent.fuel = agent.fuel_cap;
            events.push(StepEvent::Picked {
                agent_id: agent.id,
                coord,
                series: u32::MAX, // sentinel: "this event is a refill, not a ball pick"
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Traffic update — at end of day, recompute road states.
// ---------------------------------------------------------------------------

/// Count one stay-step per agent at its **post-move** position (Q27: 「移動
/// 後の滞在ステップ数」). Called once per reflection — step 0 is
/// action-phase-only and does not count, the day's final reflection does.
///
/// Spec: 募集要項「道路セルと交通量」 — the traffic volume of a road cell is
/// the per-team average of stay-steps over the past 2 days. Only the *own*
/// team's stays are observed by the offline simulator; other teams' stays are
/// never fabricated from guessed trajectories (see
/// [`phase_traffic_update`] for the averaging and the prediction caveat).
pub fn record_stay_steps(state: &mut MatchState) {
    for agent in &state.agents {
        if let Some(cell) = state.setting.map.get(agent.pos) {
            if matches!(cell.terrain, Terrain::Road(_)) {
                *state.traffic_today.entry(agent.pos).or_insert(0) += 1;
            }
        }
    }
}

/// Recompute each road cell's [`TrafficState`] from the **all-team stay-step
/// average** over the past 2 days (Q&A Q13 + 募集要項「道路セルと交通量」):
///
/// ```text
/// traffic = Σ_teams(stay-steps on the cell over yesterday + the day before)
///           / team_count
/// ```
///
/// `traffic_today` / `traffic_yesterday` hold the **own team's** raw
/// stay-steps ([`record_stay_steps`]). Other teams' contributions are taken
/// as 0 here: the offline simulator observes only its own agents, and
/// fabricating opponents' movement would turn a rollout into a prediction.
/// Callers that hold an observed all-team total (e.g. a replayed live
/// match) should pre-scale their counts accordingly; callers that *estimate*
/// future traffic must label that estimate a prediction and keep it out of
/// this observation path.
///
/// `team_count` is the match's team count (`players` on the wire). A `0`
/// value is treated as 1 so a single-team setting keeps its plain sum.
///
/// Mutates the map in place and returns the updates for callers / events.
/// The result is fixed for the whole next day — the official road state is
/// published at day start and never changes mid-day.
pub fn phase_traffic_update(
    state: &mut MatchState,
    thresholds: TrafficThresholds,
    team_count: u32,
) -> Vec<(HexCoord, TrafficState)> {
    let mut updates = Vec::new();
    let divisor = team_count.max(1);
    // Collect coords first to avoid borrow conflict on `map`.
    let road_coords: Vec<HexCoord> = state.setting.map.roads().map(|c| c.coord).collect();
    for coord in road_coords {
        let today = state.traffic_today.get(&coord).copied().unwrap_or(0);
        let yesterday = state.traffic_yesterday.get(&coord).copied().unwrap_or(0);
        let average = today.saturating_add(yesterday) / divisor;
        let new_state = thresholds.classify(average);
        if let Some(cell) = state.setting.map.get_mut(coord) {
            if let Terrain::Road(ref mut current) = cell.terrain {
                // Only *changes* are reported: `MatchSim::step` gates
                // `StepEvent::TrafficUpdated` on this vec being non-empty, so
                // including unchanged roads made the event fire on every
                // day-end and the change detection dead.
                if *current != new_state {
                    *current = new_state;
                    updates.push((coord, new_state));
                }
            }
        }
    }
    updates
}

/// Convenience: classify for a single coord given arbitrary counts.
///
/// Exposed so the unit tests don't need to fabricate a [`MatchState`].
#[must_use]
pub fn classify_traffic(thresholds: TrafficThresholds, today: u32, yesterday: u32) -> TrafficState {
    thresholds.classify_pair(today, yesterday)
}

// ---------------------------------------------------------------------------
// Convenience helpers for tests / callers.
// ---------------------------------------------------------------------------

/// Count the number of balls in a [`Spot`].
#[must_use]
pub const fn spot_stock(spot: &Spot) -> u32 {
    spot.current_stock
}

/// `true` iff `cell.terrain` is passable for a `Move` action.
#[must_use]
pub const fn cell_traversable(cell: &Cell) -> bool {
    cell.terrain.traversable()
}

// `SPOT_DAILY_PICK_PER_AGENT` lives in `rules` — re-exported under its
// qualified name here for callers that already imported `crate::transitions`.
#[allow(dead_code)]
pub(crate) const _SPOT_DAILY_PICK_PER_AGENT: u32 = SPOT_DAILY_PICK_PER_AGENT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Agent, AgentKind, MapGrid, MapSize, Spot, Terrain, TrafficState};

    fn plain_map(width: u8, height: u8) -> MapGrid {
        let size = MapSize::new(width, height).unwrap();
        let cells = (0..i32::from(height))
            .flat_map(|row| {
                (0..i32::from(width)).map(move |col| crate::model::Cell {
                    coord: HexCoord::new(col, row),
                    terrain: Terrain::Plain,
                    spot: None,
                })
            })
            .collect();
        MapGrid::new(size, cells).unwrap()
    }

    #[test]
    fn phase_wait_filters_unknown_ids() {
        let known = vec![0, 1, 2];
        let actions = vec![
            Action::step(0, crate::model::Direction::Right),
            Action::wait(99), // unknown — dropped
            Action::step(2, crate::model::Direction::Left),
        ];
        let resolved = phase_wait(&actions, &known);
        assert_eq!(resolved.len(), 2);
        assert!(resolved.contains_key(&0));
        assert!(resolved.contains_key(&2));
        assert!(!resolved.contains_key(&99));
    }

    #[test]
    fn try_move_rejects_out_of_bounds() {
        let map = plain_map(8, 8);
        let agent = Agent::patrol(0, HexCoord::new(0, 0), 5, 3);
        let mut events = Vec::new();
        let mut outcomes = Vec::new();
        let outcome = try_move(
            &agent,
            Action::step(0, crate::model::Direction::UpperLeft),
            &map,
            &mut outcomes,
            &mut events,
        );
        assert!(matches!(outcome, MoveOutcome::Wait { .. }));
        assert!(events.iter().any(|e| matches!(e, StepEvent::IllegalMove { .. })));
    }

    #[test]
    fn try_move_rejects_pond() {
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..8 {
            for col in 0..8 {
                let terrain = if (col, row) == (1, 0) { Terrain::Pond } else { Terrain::Plain };
                cells.push(crate::model::Cell {
                    coord: HexCoord::new(col, row),
                    terrain,
                    spot: None,
                });
            }
        }
        let map = MapGrid::new(size, cells).unwrap();
        let agent = Agent::patrol(0, HexCoord::new(0, 0), 5, 3);
        let mut events = Vec::new();
        let mut outcomes = Vec::new();
        let outcome = try_move(
            &agent,
            Action::step(0, crate::model::Direction::Right),
            &map,
            &mut outcomes,
            &mut events,
        );
        assert!(matches!(outcome, MoveOutcome::Wait { .. }));
        assert!(events.iter().any(
            |e| matches!(e, StepEvent::IllegalMove { reason, .. } if reason.contains("impassable"))
        ));
    }

    #[test]
    fn try_move_rejects_fuel_short() {
        let map = plain_map(8, 8);
        let mut agent = Agent::patrol(0, HexCoord::new(0, 0), 5, 3);
        agent.fuel = 0;
        let mut events = Vec::new();
        let mut outcomes = Vec::new();
        let outcome = try_move(
            &agent,
            Action::step(0, crate::model::Direction::Right),
            &map,
            &mut outcomes,
            &mut events,
        );
        let _ = outcomes;
        assert!(matches!(outcome, MoveOutcome::Wait { .. }));
        assert!(events.iter().any(|e| matches!(e, StepEvent::FuelShort { .. })));
    }

    #[test]
    fn apply_moves_advances_positions_and_records_costs() {
        let mut agents = vec![Agent::patrol(0, HexCoord::new(0, 0), 5, 3)];
        let mut outcomes_map = HashMap::new();
        outcomes_map.insert(0, MoveOutcome::Moved { to: HexCoord::new(1, 0), cost: 1 });
        let costs = apply_moves(&mut agents, &outcomes_map);
        assert_eq!(agents[0].pos, HexCoord::new(1, 0));
        assert_eq!(costs, vec![(0, 1)]);
    }

    #[test]
    fn phase_fuel_deducts_patrol_only() {
        let mut agents = vec![
            Agent {
                id: 0,
                kind: AgentKind::Patrol,
                pos: HexCoord::new(0, 0),
                fuel: 10,
                fuel_cap: 10,
                inventory: Default::default(),
            },
            Agent {
                id: 1,
                kind: AgentKind::Supply,
                pos: HexCoord::new(0, 0),
                fuel: u32::MAX,
                fuel_cap: u32::MAX,
                inventory: Default::default(),
            },
        ];
        let costs = vec![(0, 3), (1, 3)];
        let mut events = Vec::new();
        phase_fuel(&mut agents, &costs, &mut events);
        assert_eq!(agents[0].fuel, 7);
        assert_eq!(agents[1].fuel, u32::MAX);
    }

    /// Issue #243 — `phase_oracle_refill` tops up only Patrol tanks that are
    /// actually below cap, leaves Supply alone, and says so with the same
    /// refill sentinel the supply-contact path uses.
    #[test]
    fn phase_oracle_refill_refills_patrol_only_and_emits_the_refill_sentinel() {
        let mut agents = vec![
            Agent {
                id: 0,
                kind: AgentKind::Patrol,
                pos: HexCoord::new(0, 0),
                fuel: 3,
                fuel_cap: 10,
                inventory: Default::default(),
            },
            // A Patrol already at cap must not be touched (and must not
            // produce a second event).
            Agent {
                id: 1,
                kind: AgentKind::Patrol,
                pos: HexCoord::new(1, 0),
                fuel: 10,
                fuel_cap: 10,
                inventory: Default::default(),
            },
            Agent {
                id: 2,
                kind: AgentKind::Supply,
                pos: HexCoord::new(0, 0),
                fuel: u32::MAX,
                fuel_cap: u32::MAX,
                inventory: Default::default(),
            },
        ];
        let mut events = Vec::new();
        phase_oracle_refill(&mut agents, &mut events);
        assert_eq!(agents[0].fuel, 10, "depleted patrol must be refilled to cap");
        assert_eq!(agents[1].fuel, 10, "patrol at cap is untouched");
        assert_eq!(agents[2].fuel, u32::MAX, "supply is never touched");
        assert_eq!(events.len(), 1, "exactly the refilled patrol reports");
        assert!(
            events.iter().any(|e| matches!(e, StepEvent::Picked { agent_id: 0, series, .. }
                if *series == u32::MAX)),
            "refills use the u32::MAX sentinel, like supply refuelling"
        );
    }

    #[test]
    fn phase_acquire_enforces_daily_cap() {
        use crate::model::MatchSetting;
        use domain::PlayerId;
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..8 {
            for col in 0..8 {
                let (terrain, spot) = if (col, row) == (0, 0) {
                    (
                        Terrain::Plain,
                        Some(Spot {
                            series: 1,
                            pos: HexCoord::new(0, 0),
                            max_stock: 3,
                            current_stock: 3,
                        }),
                    )
                } else {
                    (Terrain::Plain, None)
                };
                cells.push(crate::model::Cell { coord: HexCoord::new(col, row), terrain, spot });
            }
        }
        let map = MapGrid::new(size, cells).unwrap();
        // The acquisition is driven manually, so we don't need a 3-agent
        // match — but the spec still requires it. Use supply padding.
        let setting = MatchSetting {
            match_id: domain::MatchId::new("match-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::patrol(0, HexCoord::new(1, 0), 5, 3),
                Agent::supply(1, HexCoord::new(3, 3)),
                Agent::supply(2, HexCoord::new(3, 3)),
            ],
            day_steps: 5,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let mut state = MatchState::new(setting).unwrap();
        let mut events = Vec::new();
        // First, simulate the agent moving onto the spot.
        state.agents[0].pos = HexCoord::new(0, 0);
        let mut agent_clone = state.agents[0].clone();
        phase_acquire(&agent_clone, &mut state, &mut events);
        assert!(events.iter().any(|e| matches!(e, StepEvent::Picked { series: 1, .. })));
        assert_eq!(state.agents[0].inventory.get(&1).copied().unwrap_or(0), 1);
        // Second attempt same day — cap reached.
        agent_clone = state.agents[0].clone();
        phase_acquire(&agent_clone, &mut state, &mut events);
        assert!(events.iter().any(|e| matches!(e, StepEvent::PickCapReached { .. })));
        assert_eq!(state.agents[0].inventory.get(&1).copied().unwrap_or(0), 1);
    }

    /// Issue #234 — `AgentKind::Supply` "cannot collect 玉"
    /// (`domain/src/agent.rs`). A Supply standing on a *stocked* spot must
    /// produce **zero** side effects: no inventory, no `current_stock`
    /// decrement (that ball belongs to Patrol), no `series_*` bookkeeping,
    /// and no `Picked` **or** `PickCapReached` event.
    #[test]
    fn phase_acquire_skips_supply_with_zero_side_effects() {
        use crate::model::MatchSetting;
        use domain::PlayerId;
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..8 {
            for col in 0..8 {
                let (terrain, spot) = if (col, row) == (0, 0) {
                    (
                        Terrain::Plain,
                        Some(Spot {
                            series: 1,
                            pos: HexCoord::new(0, 0),
                            max_stock: 3,
                            current_stock: 3,
                        }),
                    )
                } else {
                    (Terrain::Plain, None)
                };
                cells.push(crate::model::Cell { coord: HexCoord::new(col, row), terrain, spot });
            }
        }
        let map = MapGrid::new(size, cells).unwrap();
        let setting = MatchSetting {
            match_id: domain::MatchId::new("match-supply").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::supply(0, HexCoord::new(1, 0)),
                Agent::supply(1, HexCoord::new(3, 3)),
                Agent::supply(2, HexCoord::new(3, 3)),
            ],
            day_steps: 5,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let mut state = MatchState::new(setting).unwrap();
        let mut events = Vec::new();
        // The supply steps onto the stocked spot.
        state.agents[0].pos = HexCoord::new(0, 0);
        let supply_clone = state.agents[0].clone();

        // Repeated calls (the Supply parks on the spot all day) stay inert —
        // including when the Patrol daily cap would have fired.
        for _ in 0..5 {
            phase_acquire(&supply_clone, &mut state, &mut events);
        }

        assert!(events.is_empty(), "Supply must emit no events at all, got {events:?}");
        assert_eq!(
            state
                .setting
                .map
                .get(HexCoord::new(0, 0))
                .and_then(|c| c.spot.as_ref())
                .map(|s| s.current_stock),
            Some(3),
            "Supply must not decrement spot stock"
        );
        assert!(state.agents[0].inventory.is_empty(), "Supply inventory must stay empty");
        assert!(state.series_taken.is_empty(), "series_taken must not record a Supply pickup");
        assert!(state.series_first_taken_at.is_empty(), "series_first_taken_at must stay empty");
        assert!(state.series_acquired_by_day.is_empty(), "series_acquired_by_day must stay empty");
        assert!(state.spot_picks_today.is_empty(), "no daily-cap bookkeeping for Supply");
    }

    /// Contrast for `phase_acquire_skips_supply_with_zero_side_effects`: a
    /// Patrol on the *same* stocked spot still picks, so the inertness above
    /// is a kind guard rather than a dead code path.
    #[test]
    fn phase_acquire_still_collects_for_patrol_on_the_same_spot() {
        use crate::model::MatchSetting;
        use domain::PlayerId;
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..8 {
            for col in 0..8 {
                let (terrain, spot) = if (col, row) == (0, 0) {
                    (
                        Terrain::Plain,
                        Some(Spot {
                            series: 1,
                            pos: HexCoord::new(0, 0),
                            max_stock: 3,
                            current_stock: 3,
                        }),
                    )
                } else {
                    (Terrain::Plain, None)
                };
                cells.push(crate::model::Cell { coord: HexCoord::new(col, row), terrain, spot });
            }
        }
        let map = MapGrid::new(size, cells).unwrap();
        let setting = MatchSetting {
            match_id: domain::MatchId::new("match-patrol").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::patrol(0, HexCoord::new(1, 0), 5, 3),
                Agent::supply(1, HexCoord::new(3, 3)),
                Agent::supply(2, HexCoord::new(3, 3)),
            ],
            day_steps: 5,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let mut state = MatchState::new(setting).unwrap();
        let mut events = Vec::new();
        state.agents[0].pos = HexCoord::new(0, 0);
        let patrol_clone = state.agents[0].clone();
        phase_acquire(&patrol_clone, &mut state, &mut events);

        assert!(
            events.iter().any(|e| matches!(e, StepEvent::Picked { agent_id: 0, series: 1, .. }))
        );
        assert_eq!(state.agents[0].inventory.get(&1).copied(), Some(1));
        assert_eq!(
            state
                .setting
                .map
                .get(HexCoord::new(0, 0))
                .and_then(|c| c.spot.as_ref())
                .map(|s| s.current_stock),
            Some(2),
            "Patrol still decrements spot stock"
        );
        assert!(state.series_taken.contains(&1));
        assert_eq!(state.series_acquired_by_day.get(&0).map(|s| s.len()), Some(1));
    }

    #[test]
    fn phase_traffic_update_classifies_from_two_days() {
        use crate::model::MatchSetting;
        use domain::PlayerId;
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..8 {
            for col in 0..8 {
                let terrain = if (col, row) == (0, 0) {
                    Terrain::Road(TrafficState::Smooth)
                } else {
                    Terrain::Plain
                };
                cells.push(crate::model::Cell {
                    coord: HexCoord::new(col, row),
                    terrain,
                    spot: None,
                });
            }
        }
        let map = MapGrid::new(size, cells).unwrap();
        let setting = MatchSetting {
            match_id: domain::MatchId::new("match-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::patrol(0, HexCoord::new(7, 7), 5, 3),
                Agent::supply(1, HexCoord::new(3, 3)),
                Agent::supply(2, HexCoord::new(3, 3)),
            ],
            day_steps: 5,
            num_days: 2,
            day_steps_by_day: Vec::new(),
        };
        let mut state = MatchState::new(setting).unwrap();
        state.traffic_yesterday.insert(HexCoord::new(0, 0), 3); // yesterday was Busy
        state.traffic_today.insert(HexCoord::new(0, 0), 0); // today is quiet
        let updates = phase_traffic_update(&mut state, TrafficThresholds::default(), 1);
        // Outdated wording below kept from the pre-#342 comment: the rule is
        // not a max — it is the 2-day stay-step sum averaged over the team
        // count. Here (0 + 3) / 1 team = 3 → Busy.
        let r = updates.iter().find(|(c, _)| *c == HexCoord::new(0, 0)).unwrap();
        assert_eq!(r.1, TrafficState::Busy);
    }
}
