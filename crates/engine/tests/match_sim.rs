//! Integration scenarios for the `engine::MatchSim` day-step simulator.
//!
//! These complement the unit tests in `src/sim.rs` by exercising the full
//! public surface (`MatchSim::run`) on hand-crafted scenarios that map to
//! the Q&A Q6 phase order.
//!
//! Each scenario asserts an externally visible property (final state shape,
//! score breakdown, or determinism) rather than internal events.

use std::collections::HashMap;

use engine::model::{
    Action, Agent, AgentActions, Cell, Direction, HexCoord, MapGrid, MapSize, MatchSetting, Solver,
    SolverContext, Spot, Terrain, TrafficState,
};
use engine::{MatchSim, SimConfig};

use domain::MatchId;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn id(s: &str) -> domain::PlayerId {
    domain::PlayerId::new(s).expect("valid id")
}

fn match_id(s: &str) -> MatchId {
    MatchId::new(s).expect("valid id")
}

/// 8x8 plain map with one spot at `(0,0)` (series 1, max=3) and one spot at
/// `(7,7)` (series 2, max=3). All other cells are plain.
fn two_spot_plain_map() -> MapGrid {
    let size = MapSize::new(8, 8).unwrap();
    let mut cells = Vec::new();
    for row in 0..i32::from(size.height) {
        for col in 0..i32::from(size.width) {
            let spot = if (col, row) == (0, 0) {
                Some(Spot { series: 1, pos: HexCoord::new(0, 0), max_stock: 3, current_stock: 3 })
            } else if (col, row) == (7, 7) {
                Some(Spot { series: 2, pos: HexCoord::new(7, 7), max_stock: 3, current_stock: 3 })
            } else {
                None
            };
            cells.push(Cell { coord: HexCoord::new(col, row), terrain: Terrain::Plain, spot });
        }
    }
    MapGrid::new(size, cells).unwrap()
}

/// A solver that walks every agent in `direction` every step.
struct WalkSolver {
    name: &'static str,
    direction: Direction,
}

impl WalkSolver {
    const fn new(name: &'static str, direction: Direction) -> Self {
        Self { name, direction }
    }
}

impl Solver for WalkSolver {
    fn name(&self) -> &'static str {
        self.name
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let mut actions = Vec::with_capacity(ctx.day_steps as usize * ctx.agent_ids.len());
        for _ in 0..ctx.day_steps {
            for id in &ctx.agent_ids {
                actions.push(Action::step(*id, self.direction));
            }
        }
        AgentActions::new(actions)
    }
}

/// A solver that picks a per-agent direction (e.g., agent 0 goes Left,
/// agent 1 goes Right). Falls back to a Wait action for agents with no
/// configured direction.
struct DirectedSolver {
    name: &'static str,
    directions: HashMap<u8, Direction>,
}

impl DirectedSolver {
    fn new(name: &'static str, pairs: &[(u8, Direction)]) -> Self {
        let mut map = HashMap::new();
        for (id, dir) in pairs {
            map.insert(*id, *dir);
        }
        Self { name, directions: map }
    }
}

impl Solver for DirectedSolver {
    fn name(&self) -> &'static str {
        self.name
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let mut actions = Vec::with_capacity(ctx.day_steps as usize * ctx.agent_ids.len());
        for _ in 0..ctx.day_steps {
            for id in &ctx.agent_ids {
                match self.directions.get(id).copied() {
                    Some(dir) => actions.push(Action::step(*id, dir)),
                    None => actions.push(Action::wait(*id)),
                }
            }
        }
        AgentActions::new(actions)
    }
}

/// Pad a `Vec<Agent>` to the spec minimum (3) with idle supply agents at
/// `(3, 3)` so the `MatchSetting::validate` invariant passes.
fn pad_to_min_team(agents: Vec<Agent>) -> Vec<Agent> {
    let mut agents = agents;
    while agents.len() < 3 {
        // Length is bounded by `MatchSetting::validate` (3..=8), so the
        // cast to u8 is safe.
        #[allow(clippy::cast_possible_truncation)]
        let next_id = agents.len() as u8;
        agents.push(Agent::supply(next_id, HexCoord::new(3, 3)));
    }
    agents
}

// ---------------------------------------------------------------------------
// Scenario 1 — full match walks both series with two cooperating agents.
// ---------------------------------------------------------------------------

#[test]
fn scenario_two_agents_collect_two_series_across_three_days() {
    let map = two_spot_plain_map();
    let setting = MatchSetting {
        match_id: match_id("scenario-1"),
        team: id("player-a"),
        map,
        agents: pad_to_min_team(vec![
            Agent::patrol(0, HexCoord::new(1, 0), 200, 3),
            Agent::patrol(1, HexCoord::new(6, 7), 200, 3),
        ]),
        day_steps: 10,
        num_days: 3,
        day_steps_by_day: Vec::new(),
    };
    let sim = MatchSim::default();
    let solver = DirectedSolver::new("two-agents", &[(0, Direction::Left), (1, Direction::Right)]);
    let result = sim.run(setting, &solver).unwrap();

    // Both series were visited at least once across the match.
    assert!(result.variety >= 2, "expected variety >= 2, got {}", result.variety);
    // At least one ball from each series (one pick per day per spot per
    // agent, capped at 1).
    assert!(result.total_balls >= 2, "expected total_balls >= 2, got {}", result.total_balls);
    // daily_variety_sum: distinct series taken per day, summed.
    assert!(
        result.daily_variety_sum >= 2,
        "expected daily_variety_sum >= 2, got {}",
        result.daily_variety_sum
    );
    // Match fully ran (day advanced past num_days).
    assert!(result.final_state.is_finished());
    // The two patrols actually ended on the spot cells.
    assert_eq!(result.final_state.agents[0].pos, HexCoord::new(0, 0));
    assert_eq!(result.final_state.agents[1].pos, HexCoord::new(7, 7));
}

// ---------------------------------------------------------------------------
// Scenario 2 — determinism: same solver on same map yields identical results.
// ---------------------------------------------------------------------------

#[test]
fn scenario_run_is_deterministic_across_repeated_invocation() {
    let map = two_spot_plain_map();
    let mk = || MatchSetting {
        match_id: match_id("scenario-2"),
        team: id("player-a"),
        map: map.clone(),
        agents: pad_to_min_team(vec![Agent::patrol(0, HexCoord::new(3, 3), 100, 3)]),
        day_steps: 8,
        num_days: 2,
        day_steps_by_day: Vec::new(),
    };
    let solver = WalkSolver::new("walk-upper-left", Direction::UpperLeft);
    let sim = MatchSim::default();
    let a = sim.run(mk(), &solver).unwrap();
    let b = sim.run(mk(), &solver).unwrap();

    assert_eq!(a.final_state, b.final_state);
    assert_eq!(a.variety, b.variety);
    assert_eq!(a.daily_variety_sum, b.daily_variety_sum);
    assert_eq!(a.total_balls, b.total_balls);
    assert_eq!(a.last_acquisition_at, b.last_acquisition_at);
}

// ---------------------------------------------------------------------------
// Scenario 3 — Q6 phase order is observable end-to-end:
//   1. wait phase degrades malformed input gracefully,
//   2. move phase advances positions and consumes fuel,
//   3. acquire phase picks up a ball,
//   4. fuel carry-over (Q&A Q19) keeps fuel across days,
//   5. traffic update phase (Q&A Q13) promotes Smooth -> Busy when yesterday
//      counts are high.
// ---------------------------------------------------------------------------

#[test]
fn scenario_phase_order_observable_through_final_state() {
    // Build an 8x8 map with a road at (0,0) and a spot also at (0,0).
    // (Per Q&A Q18, a cell can host both road + spot.)
    let size = MapSize::new(8, 8).unwrap();
    let mut cells = Vec::new();
    for row in 0..i32::from(size.height) {
        for col in 0..i32::from(size.width) {
            let (terrain, spot) = if (col, row) == (0, 0) {
                (
                    Terrain::Road(TrafficState::Smooth),
                    Some(Spot {
                        series: 1,
                        pos: HexCoord::new(0, 0),
                        max_stock: 5,
                        current_stock: 5,
                    }),
                )
            } else {
                (Terrain::Plain, None)
            };
            cells.push(Cell { coord: HexCoord::new(col, row), terrain, spot });
        }
    }
    let map = MapGrid::new(size, cells).unwrap();

    // Two patrol agents start at (1,0) (next to the spot+road combo) and
    // (7,7) (far away). Fuel cap = day_steps * 3 = 24, plenty for a few
    // moves. The match is intentionally short (1 day) so the end-of-day
    // traffic update promotes the road to Busy and that promotion is the
    // final state we observe.
    let setting = MatchSetting {
        match_id: match_id("scenario-3"),
        team: id("player-a"),
        map,
        agents: pad_to_min_team(vec![
            Agent::patrol(0, HexCoord::new(1, 0), 8, 3),
            Agent::patrol(1, HexCoord::new(7, 7), 8, 3),
        ]),
        day_steps: 8,
        num_days: 1,
        day_steps_by_day: Vec::new(),
    };

    let sim = MatchSim::new(SimConfig::default());
    // Pretend yesterday's road at (0,0) was busy (3 traversals) — should
    // promote to Busy even if today has 0 traversals (Q&A Q13: max over
    // 2 days).
    let mut state = engine::model::MatchState::new(setting.clone()).unwrap();
    state.traffic_yesterday.insert(HexCoord::new(0, 0), 3);

    let solver = DirectedSolver::new("to-spot", &[(0, Direction::Left), (1, Direction::UpperLeft)]);
    let result = sim.run_from_state(&mut state, &solver).unwrap();

    // Phase 4 (acquire) — agent 0 picked up at least one series-1 ball.
    assert!(
        result.final_state.agents[0].total_inventory() >= 1,
        "agent 0 should have at least 1 ball, inventory = {:?}",
        result.final_state.agents[0].inventory
    );

    // Phase 5 (fuel carry-over) — agent 0's fuel < initial cap (24)
    // because moves were deducted, but leftover still exists (Q19).
    let agent_0 = &result.final_state.agents[0];
    assert!(
        agent_0.fuel < 24,
        "fuel should have been consumed from initial 24, got {}",
        agent_0.fuel
    );
    assert!(agent_0.fuel > 0, "fuel should not be fully drained");

    // Phase 6 (traffic update) — the road at (0,0) was promoted to Busy
    // because yesterday had 3 traversals (>= busy threshold 2).
    let road_cell = result.final_state.setting.map.get(HexCoord::new(0, 0)).unwrap();
    assert!(
        matches!(road_cell.terrain, Terrain::Road(TrafficState::Busy)),
        "expected road to be Busy, got {:?}",
        road_cell.terrain
    );
}

// ---------------------------------------------------------------------------
// Issue #234 — Supply agents must never acquire (offline/live kind parity).
//
// `domain::AgentKind::Supply` is specified as "cannot collect 玉 but can
// refuel Patrol". Before this issue `engine::transitions::phase_acquire`
// ignored the kind, so offline every agent collected while live (where
// `from_wire` alternates kinds: 5 agents -> 3 Patrol / 2 Supply) did not.
//
// Scenario: a 5-agent team in the live alternating shape (3 Patrol / 2
// Supply) on an 8x8 plain map with two stocked spots. A Supply moves onto
// the series-1 spot on step 0 and parks there for the whole day, next to a
// Patrol that also parks there.
// ---------------------------------------------------------------------------

/// 8x8 plain map with two spots: series 1 at `(0,0)`, series 2 at `(7,7)`,
/// both fully stocked with 5 balls (the team size).
fn two_spot_5_stock_map() -> MapGrid {
    let size = MapSize::new(8, 8).unwrap();
    let mut cells = Vec::new();
    for row in 0..i32::from(size.height) {
        for col in 0..i32::from(size.width) {
            let spot = if (col, row) == (0, 0) {
                Some(Spot { series: 1, pos: HexCoord::new(0, 0), max_stock: 5, current_stock: 5 })
            } else if (col, row) == (7, 7) {
                Some(Spot { series: 2, pos: HexCoord::new(7, 7), max_stock: 5, current_stock: 5 })
            } else {
                None
            };
            cells.push(Cell { coord: HexCoord::new(col, row), terrain: Terrain::Plain, spot });
        }
    }
    MapGrid::new(size, cells).unwrap()
}

/// Move each configured agent once on step 0, then wait for the rest of the
/// day — so agents can *park* on a spot without generating illegal-move
/// downgrades on later steps.
struct ParkSolver {
    directions: HashMap<u8, Direction>,
}

impl ParkSolver {
    fn new(pairs: &[(u8, Direction)]) -> Self {
        Self { directions: pairs.iter().copied().collect() }
    }
}

impl Solver for ParkSolver {
    fn name(&self) -> &'static str {
        "park"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let mut actions = Vec::with_capacity(ctx.day_steps as usize * ctx.agent_ids.len());
        for step in 0..ctx.day_steps {
            for id in &ctx.agent_ids {
                if step == 0 {
                    if let Some(dir) = self.directions.get(id) {
                        actions.push(Action::step(*id, *dir));
                        continue;
                    }
                }
                actions.push(Action::wait(*id));
            }
        }
        AgentActions::new(actions)
    }
}

#[test]
fn scenario_supply_agent_never_acquires_on_a_stocked_spot() {
    use engine::transitions::StepEvent;

    let setting = MatchSetting {
        match_id: match_id("issue234-supply-no-acquire"),
        team: id("player-a"),
        map: two_spot_5_stock_map(),
        // Live `from_wire` alternation for a 5-agent team: even index ->
        // Patrol, odd index -> Supply (3 Patrol / 2 Supply).
        agents: vec![
            Agent::patrol(0, HexCoord::new(1, 0), 4, 3),
            Agent::supply(1, HexCoord::new(1, 1)),
            Agent::patrol(2, HexCoord::new(6, 7), 4, 3),
            Agent::supply(3, HexCoord::new(3, 3)),
            Agent::patrol(4, HexCoord::new(4, 4), 4, 3),
        ],
        day_steps: 4,
        num_days: 1,
        day_steps_by_day: Vec::new(),
    };
    // Patrol 0 -> the series-1 spot at (0,0); Supply 1 -> the SAME cell
    // (UpperLeft from (1,1) on an odd row lands on (0,0)); Patrol 2 -> the
    // series-2 spot at (7,7). Supply 3 / Patrol 4 idle in place.
    let solver =
        ParkSolver::new(&[(0, Direction::Left), (1, Direction::UpperLeft), (2, Direction::Right)]);

    // Drive the steps manually so the test can also assert on `StepEvent`s
    // (`MatchSim::run` only returns the final state).
    let mut state = engine::model::MatchState::new(setting).unwrap();
    let sim = MatchSim::default();
    let plan = solver.decide(&SolverContext::from_state(&state));
    let agent_count = state.setting.agent_ids().len();
    let step_count = usize::try_from(state.day_steps()).expect("day_steps fits usize");
    let mut events = Vec::new();
    for s in 0..step_count {
        let start = s * agent_count;
        let block = plan.actions[start..start + agent_count].to_vec();
        events.extend(sim.step(&mut state, &AgentActions::new(block)));
    }

    let by_id = |id: u8| state.agents.iter().find(|a| a.id == id).expect("agent exists");

    // (a) Supply inventory stays empty — both the parked Supply (1) and the
    //     idle one (3).
    for sid in [1u8, 3u8] {
        let supply = by_id(sid);
        assert_eq!(supply.kind, engine::AgentKind::Supply, "agent {sid} must be a Supply");
        assert!(
            supply.inventory.is_empty(),
            "Supply {sid} must not collect, got inventory {:?}",
            supply.inventory
        );
    }

    // (b) Spot stock is only ever reduced by Patrol picks. Supply 1 stood on
    //     the series-1 spot for the whole day; pre-#234 it would have taken
    //     one ball (daily cap 1), leaving 3 instead of 4.
    let spot_a = state
        .setting
        .map
        .get(HexCoord::new(0, 0))
        .and_then(|c| c.spot.as_ref())
        .expect("series-1 spot");
    assert_eq!(
        spot_a.current_stock, 4,
        "only Patrol 0's single capped pick may decrement the series-1 spot"
    );
    let spot_b = state
        .setting
        .map
        .get(HexCoord::new(7, 7))
        .and_then(|c| c.spot.as_ref())
        .expect("series-2 spot");
    assert_eq!(spot_b.current_stock, 4, "Patrol 2 picked exactly one from the series-2 spot");

    // (c) `series_taken` / `series_acquired_by_day` reflect Patrol pickups only.
    assert_eq!(
        state.series_taken.iter().copied().collect::<Vec<_>>(),
        vec![1, 2],
        "series_taken must contain exactly the Patrol-picked series"
    );
    assert_eq!(
        state.series_acquired_by_day.get(&0).map(|s| s.iter().copied().collect::<Vec<_>>()),
        Some(vec![1, 2]),
        "series_acquired_by_day[0] must contain exactly the Patrol-picked series"
    );
    assert_eq!(
        state.series_first_taken_at.keys().copied().collect::<Vec<_>>(),
        vec![1, 2],
        "series_first_taken_at must not record a Supply pickup"
    );

    // (d) Patrol picking + daily cap behave as before: Patrol 0 held the
    //     spot for 4 steps but the cap allows 1 ball per agent per spot per day.
    assert_eq!(by_id(0).inventory.get(&1).copied(), Some(1), "Patrol 0 picks exactly once/day");
    assert_eq!(by_id(2).inventory.get(&2).copied(), Some(1), "Patrol 2 picks exactly once/day");
    assert!(
        events.iter().any(|e| matches!(e, StepEvent::PickCapReached { agent_id: 0, .. })),
        "the daily cap must still fire for the parking Patrol"
    );

    // Events: no `Picked` and no `PickCapReached` for either Supply.
    for sid in [1u8, 3u8] {
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, StepEvent::Picked { agent_id, .. } if *agent_id == sid)),
            "Supply {sid} must emit no Picked event, got {events:?}"
        );
        assert!(
            !events.iter().any(
                |e| matches!(e, StepEvent::PickCapReached { agent_id, .. } if *agent_id == sid)
            ),
            "Supply {sid} must emit no PickCapReached event, got {events:?}"
        );
    }

    // Refuel is unaffected: Supply 1 shares (0,0) with Patrol 0, so Patrol 0
    // ends the day at its fuel cap (the one move was deducted, then refilled).
    let patrol = by_id(0);
    assert_eq!(
        patrol.fuel, patrol.fuel_cap,
        "a Supply parked on a spot must still refuel the Patrol beside it"
    );
    assert_eq!(state.fuel_short_count, 0);
    assert_eq!(state.illegal_action_count, 0, "the scenario must not generate illegal moves");
}
