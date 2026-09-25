//! Single-step transition primitives.
//!
//! Each function corresponds to one phase of Q&A Q6's order:
//!
//! 1. [`phase_wait`] — interpret missing/invalid actions as `Wait`.
//! 2. [`phase_move`] — apply each agent's `Move` action (with bounds, terrain
//!    and fuel checks). Out-of-bounds / impassable / fuel-short actions
//!    degrade to a no-op `Wait` and are reported as `StepEvent::IllegalMove`.
//! 3. [`phase_spawn`] — refill spot stock at the start of a new day.
//! 4. [`phase_acquire`] — pick up balls from spots the agents stand on.
//! 5. [`phase_fuel`] — deduct the fuel cost of the move that was applied in
//!    phase 2 (separated so the move phase stays terrain/pure and the fuel
//!    phase owns the "did we have enough?" decision).
//! 6. [`phase_traffic_update`] — at end of day, recompute every road cell's
//!    [`TrafficState`] from today's + yesterday's counts (Q&A Q13).
//!
//! The functions are independent so tests can exercise each phase in
//! isolation; [`super::sim::MatchSim::step`] stitches them together in the
//! Q6 order.

use std::collections::HashMap;

use crate::model::{
    Action, ActionKind, Agent, AgentKind, Cell, HexCoord, MapGrid, MatchState, Spot, Terrain,
    TrafficState,
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
/// The output preserves no ordering — callers iterate it as they please.
#[must_use]
pub fn phase_wait(actions: &[Action], known_ids: &[u8]) -> HashMap<u8, Action> {
    let mut out = HashMap::new();
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
    let cost = fuel_cost(cell.terrain);
    if !agent.can_afford(cost) {
        return MoveOk::FuelShort { cost };
    }
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

/// Reset the per-day traffic + pickup bookkeeping.
pub fn begin_day(state: &mut MatchState) {
    state.traffic_yesterday = std::mem::take(&mut state.traffic_today);
    state.spot_picks_today.clear();
}

// ---------------------------------------------------------------------------
// 4. Acquire — agents on spots pick up balls.
// ---------------------------------------------------------------------------

/// Attempt to acquire a ball from the spot at `agent.pos`. Records the
/// per-day pickup count and emits the appropriate event.
pub fn phase_acquire(agent: &Agent, state: &mut MatchState, events: &mut Vec<StepEvent>) {
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

/// Increment today's traffic counter for `coord` (a road cell).
pub fn record_traversal(state: &mut MatchState, coord: HexCoord) {
    if let Some(cell) = state.setting.map.get(coord) {
        if matches!(cell.terrain, Terrain::Road(_)) {
            *state.traffic_today.entry(coord).or_insert(0) += 1;
        }
    }
}

/// Recompute each road cell's [`TrafficState`] from today's + yesterday's
/// counts (Q&A Q13 — over 2 days).
///
/// Mutates the map in place and returns the updates for callers / events.
pub fn phase_traffic_update(
    state: &mut MatchState,
    thresholds: TrafficThresholds,
) -> Vec<(HexCoord, TrafficState)> {
    let mut updates = Vec::new();
    // Collect coords first to avoid borrow conflict on `map`.
    let road_coords: Vec<HexCoord> = state.setting.map.roads().map(|c| c.coord).collect();
    for coord in road_coords {
        let today = state.traffic_today.get(&coord).copied().unwrap_or(0);
        let yesterday = state.traffic_yesterday.get(&coord).copied().unwrap_or(0);
        let new_state = thresholds.classify_pair(today, yesterday);
        if let Some(cell) = state.setting.map.get_mut(coord) {
            if let Terrain::Road(ref mut current) = cell.terrain {
                if *current != new_state {
                    *current = new_state;
                    updates.push((coord, new_state));
                } else {
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
        let updates = phase_traffic_update(&mut state, TrafficThresholds::default());
        // Max(0, 3) = 3, classified as Busy.
        let r = updates.iter().find(|(c, _)| *c == HexCoord::new(0, 0)).unwrap();
        assert_eq!(r.1, TrafficState::Busy);
    }
}
