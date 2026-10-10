//! Official-rule conformance tests (Issue #342 — evaluator parity).
//!
//! One section per suspected gap from the official Q&A audit (Q6 / Q7 /
//! Q13 / Q23 / Q25 / Q27 + 募集要項「道路セルと交通量」「移動と燃料」):
//!
//! 1. **Move application timing** — a Move is applied after the *departure*
//!    cell terrain's step count elapses; the per-reflection order is
//!    fuel → move-apply → acquire → refuel (Q6).
//! 2. **Road traffic classification** — traffic = (all teams' stay-steps on
//!    the road over the past 2 days) / (team count); stay-steps are counted
//!    at the post-move position of each reflection (Q13 / Q27).
//! 3. **Invalid daily response semantics** — one invalid / fuel-short /
//!    step-mismatched action invalidates the WHOLE response (Q6); the
//!    evaluation path must report that as a status flag instead of only
//!    silently demoting the action to Wait.
//!
//! Each test documents the pre-fix behaviour in comments. They are written
//! against the public `engine` surface only.

use engine::model::{
    Action, Agent, AgentActions, Cell, Direction, HexCoord, MapGrid, MapSize, MatchSetting,
    MatchState, Solver, SolverContext, Spot, Terrain, TrafficState,
};
use engine::rules::TrafficThresholds;
use engine::{MatchSim, SimConfig};

use domain::MatchId;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn match_id(s: &str) -> MatchId {
    MatchId::new(s).expect("valid id")
}

fn team(s: &str) -> domain::PlayerId {
    domain::PlayerId::new(s).expect("valid id")
}

/// Pad to the spec minimum of 3 agents with idle Supply agents at (3, 3).
fn pad_to_min_team(agents: Vec<Agent>) -> Vec<Agent> {
    let mut agents = agents;
    while agents.len() < 3 {
        #[allow(clippy::cast_possible_truncation)]
        let next_id = agents.len() as u8;
        agents.push(Agent::supply(next_id, HexCoord::new(3, 3)));
    }
    agents
}

/// 8x8 all-plain map with a single series-1 spot at `spot_at`.
fn plain_map_with_spot(spot_at: (i32, i32)) -> MapGrid {
    let size = MapSize::new(8, 8).unwrap();
    let mut cells = Vec::new();
    for row in 0..i32::from(size.height) {
        for col in 0..i32::from(size.width) {
            let spot = if (col, row) == spot_at {
                Some(Spot {
                    series: 1,
                    pos: HexCoord::new(col, row),
                    max_stock: 3,
                    current_stock: 3,
                })
            } else {
                None
            };
            cells.push(Cell { coord: HexCoord::new(col, row), terrain: Terrain::Plain, spot });
        }
    }
    MapGrid::new(size, cells).unwrap()
}

/// 8x8 map whose cell at `road_at` is a smooth road; everything else plain.
fn plain_map_with_road(road_at: (i32, i32)) -> MapGrid {
    let size = MapSize::new(8, 8).unwrap();
    let mut cells = Vec::new();
    for row in 0..i32::from(size.height) {
        for col in 0..i32::from(size.width) {
            let terrain = if (col, row) == road_at {
                Terrain::Road(TrafficState::Smooth)
            } else {
                Terrain::Plain
            };
            cells.push(Cell { coord: HexCoord::new(col, row), terrain, spot: None });
        }
    }
    MapGrid::new(size, cells).unwrap()
}

fn setting(map: MapGrid, agents: Vec<Agent>, day_steps: u32, num_days: u32) -> MatchSetting {
    MatchSetting {
        match_id: match_id("official-rule-parity"),
        team: team("player-a"),
        map,
        agents: pad_to_min_team(agents),
        day_steps,
        num_days,
        day_steps_by_day: Vec::new(),
    }
}

/// One `MatchSim::step` with `agent 0` doing `first`, everyone else waiting.
fn step0(sim: &MatchSim, state: &mut MatchState, first: Action) {
    let ids = state.setting.agent_ids();
    let actions = ids.iter().map(|id| if *id == 0 { first } else { Action::wait(*id) }).collect();
    sim.step(state, &AgentActions::new(actions));
}

fn wait_step(sim: &MatchSim, state: &mut MatchState) {
    let ids = state.setting.agent_ids();
    sim.step(state, &AgentActions::wait_all(&ids, 1));
}

// ---------------------------------------------------------------------------
// 1. Move application timing (Q6 reflection order + departure-terrain delay)
// ---------------------------------------------------------------------------

/// Minimal repro from the Q&A audit.
///
/// `daySteps = 2`, a Patrol on a spot-less 平地 issues Move onto the adjacent
/// 平地 spot at tick 0 and Wait afterwards.
///
/// Official (Q6 + 募集要項図4): the move is *reserved* at the tick-0 action
/// phase and applied when the departure terrain's step count (平地 = 2) has
/// elapsed — i.e. at the reflection of step 2, which is the day's final
/// reflection. The ball is acquired on that arrival step (`series_first_taken_at
/// == (0, 2)`).
///
/// Pre-fix behaviour: `MatchSim::step` applied the move immediately in the
/// first `step()` call and acquired in the same call (`series_first_taken_at
/// == (0, 0)`), i.e. "arrives/acquires on step 1" of the day.
#[test]
fn move_is_applied_after_the_departure_terrain_step_count() {
    let map = plain_map_with_spot((0, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 2, 1);
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::default();

    // Tick 0 — action phase only: the Move is reserved, not reflected.
    step0(&sim, &mut state, Action::step(0, Direction::Left));
    assert_eq!(
        state.agents[0].pos,
        HexCoord::new(1, 0),
        "the move must still be in flight after the tick-0 action phase \
         (pre-fix: already at (0, 0))"
    );
    assert!(
        state.series_first_taken_at.is_empty(),
        "no acquisition may happen before the move lands \
         (pre-fix: picked in the first step)"
    );
    assert_eq!(state.agents[0].fuel, 15, "fuel is charged at the reflection, not at the order");

    // Tick 1 — reflection (nothing due) + action; then the final reflection
    // at step 2 applies the move and acquires.
    wait_step(&sim, &mut state);
    assert_eq!(state.agents[0].pos, HexCoord::new(0, 0), "the move lands on its due step");
    assert_eq!(
        state.agents[0].inventory.get(&1).copied(),
        Some(1),
        "the arrival step acquires from the spot"
    );
    assert_eq!(
        state.series_first_taken_at.get(&1).copied(),
        Some((0, 2)),
        "the acquisition is timestamped on the arrival step (step 2), \
         not on the order step (pre-fix: (0, 0))"
    );
    assert_eq!(
        state.agents[0].fuel, 14,
        "the departure-terrain fuel was charged at the reflection"
    );
    assert!(state.is_finished(), "daySteps=2 runs exactly one day");
}

/// The delay is the *departure* terrain's step count (Q10 / 募集要項図4),
/// not a fixed one-step lag: a 平地 departure takes 2 steps to land.
#[test]
fn move_delay_follows_the_departure_terrain_step_count() {
    // (1, 0) is a smooth road; agent starts at (2, 0) on 平地 and steps Left
    // onto the road. 平地 departure = 2 steps, so the move lands at step 2.
    let map = plain_map_with_road((1, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(2, 0), 5, 3)], 3, 1);
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::default();

    // tick 0: reserve (2,0) -> (1,0); 平地 departure = 2 steps, due at 2.
    step0(&sim, &mut state, Action::step(0, Direction::Left));
    assert_eq!(state.agents[0].pos, HexCoord::new(2, 0), "in flight after tick 0");

    // tick 1: nothing lands (due 2).
    wait_step(&sim, &mut state);
    assert_eq!(state.agents[0].pos, HexCoord::new(2, 0), "still in flight after tick 1");

    // tick 2's reflection: the move lands on the road.
    wait_step(&sim, &mut state);
    assert_eq!(state.agents[0].pos, HexCoord::new(1, 0), "landed on the road at its due step");
}

/// The reflection order is fuel → move-apply → acquire → refuel (Q6):
/// the fuel bill is charged at the reflection step (not when the order is
/// given), and a supply agent landing on the same cell refuels *after* the
/// acquisition, so the tank ends the step at cap.
#[test]
fn reflection_order_is_fuel_then_move_then_acquire_then_refuel() {
    let map = plain_map_with_spot((0, 0));
    // Patrol and Supply share (1, 0) and both step Left onto the spot at
    // (0, 0); the patrol carries exactly the 平地 departure fuel.
    let agents = vec![
        Agent::patrol_with_fuel(0, HexCoord::new(1, 0), 1),
        Agent::supply(1, HexCoord::new(1, 0)),
    ];
    let setting = setting(map, agents, 2, 1);
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::default();

    let step_a = AgentActions::new(vec![
        Action::step(0, Direction::Left),
        Action::step(1, Direction::Left),
        Action::wait(2),
    ]);
    sim.step(&mut state, &step_a);

    // Fuel is NOT charged at the order tick — the reflection charges it.
    assert_eq!(state.agents[0].fuel, 1, "pre-reflection the tank is untouched");
    assert_eq!(state.agents[0].pos, HexCoord::new(1, 0), "the move has not landed yet");

    // Final reflection at step 2: fuel -> move -> acquire -> refuel.
    wait_step(&sim, &mut state);
    assert_eq!(state.agents[0].pos, HexCoord::new(0, 0));
    assert_eq!(
        state.agents[0].inventory.get(&1).copied(),
        Some(1),
        "acquire runs before refuel but both run on the arrival step"
    );
    assert_eq!(
        state.agents[0].fuel, state.agents[0].fuel_cap,
        "refuel runs after the move lands (same-cell supply contact)"
    );
}

/// Fuel executability is evaluated at the move's *reflection*, not at the
/// order (Q22: 燃料の消費は各ステップの補給より前; an intervening supply
/// contact on a waiting vehicle can still pay for the move — the same timing
/// `wire_plan`'s tick cursor models).
#[test]
fn intervening_supply_refuel_pays_for_a_waiting_move() {
    let map = plain_map_with_spot((0, 0));
    // Patrol with an empty tank shares (1, 0) with a Supply. The move is
    // ordered at tick 0 and lands at step 2; the tick-1 reflection refuels
    // the waiting patrol, so the landing reflection can pay the 平地 cost.
    let agents = vec![
        Agent::patrol_with_fuel(0, HexCoord::new(1, 0), 1),
        Agent::supply(1, HexCoord::new(1, 0)),
    ];
    let setting = setting(map, agents, 2, 1);
    let mut state = MatchState::new(setting).unwrap();
    state.agents[0].fuel = 0;
    let sim = MatchSim::default();

    step0(&sim, &mut state, Action::step(0, Direction::Left));
    assert_eq!(state.agents[0].fuel, 0, "no fuel is charged at the order tick");

    // Tick 1's reflection refuels the waiting patrol (same-cell supply);
    // the final reflection at step 2 lands the move on the refuelled tank.
    wait_step(&sim, &mut state);
    assert_eq!(
        state.agents[0].pos,
        HexCoord::new(0, 0),
        "the move lands on the fuel refuelled while it was in flight \
         (a reserve-time fuel check would have wrongly refused it)"
    );
    assert_eq!(state.rejected_response_count, 0, "a refuelled landing is not a rejection");
    assert_eq!(state.fuel_short_count, 0);
}

// ---------------------------------------------------------------------------
// 2. Road traffic classification (Q13 / Q27 + 募集要項「道路セルと交通量」)
// ---------------------------------------------------------------------------

/// The official traffic value of a road cell is
/// `(all teams' stay-steps over the past 2 days) / (team count)`, where a
/// stay-step is counted at the agent's post-move position of each reflection
/// (step 0 is action-only and does not count; the final step does — Q27).
///
/// Minimal repro from the Q&A audit: 2 teams, thresholds busy=2 / jammed=5;
/// own vehicle spends 4 steps on a road on day 0, the opponent none — the
/// official day-1 traffic is `(4 + 0) / 2 = 2` → Busy.
///
/// Pre-fix behaviour: `record_traversal` counted own-team *entry* events
/// only and `classify_roads` had no team average, so a vehicle that entered
/// once and stood there contributed 1 and the day-1 state stayed Smooth.
#[test]
fn traffic_averages_all_team_stay_steps_over_the_team_count() {
    // The patrol starts on the road at (0, 0) and waits all day: 4 reflections
    // (steps 1..=4 of a daySteps=4 day) all count a stay-step at (0, 0).
    let map = plain_map_with_road((0, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(0, 0), 5, 3)], 4, 2);
    let mut state = MatchState::new(setting).unwrap();

    let sim = MatchSim::new(SimConfig {
        team_count: 2,
        traffic_thresholds: TrafficThresholds { busy_threshold: 2, jammed_threshold: 5 },
        ..SimConfig::default()
    });

    // One all-wait day (4 steps). Pre-fix this recorded 0 traversals — an
    // entry-counted vehicle that never enters a road is invisible.
    for _ in 0..4 {
        wait_step(&sim, &mut state);
    }
    assert_eq!(state.day, 1, "the day boundary has run");

    let road = state.setting.map.get(HexCoord::new(0, 0)).expect("road cell");
    assert_eq!(
        road.terrain,
        Terrain::Road(TrafficState::Busy),
        "day-1 traffic is (4 own stay-steps + 0 opponent) / 2 teams = 2 → Busy; \
         pre-fix the entry-count saw 0 events and left the road Smooth"
    );
}

/// The division by the team count is real: the same 5 own stay-steps are
/// Busy with 2 teams (`5 / 2 = 2`) but Jammed with 1 team (`5 / 1 = 5`).
/// A missing average cannot produce both verdicts.
#[test]
fn traffic_is_averaged_not_just_summed_per_own_team() {
    let busy_jam = TrafficThresholds { busy_threshold: 2, jammed_threshold: 5 };

    let run = |team_count: u32, day_steps: u32| -> TrafficState {
        // Park the patrol on the road and wait `day_steps` reflections.
        let map = plain_map_with_road((0, 0));
        let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(0, 0), 5, 3)], day_steps, 2);
        let mut state = MatchState::new(setting).unwrap();
        let sim = MatchSim::new(SimConfig {
            team_count,
            traffic_thresholds: busy_jam,
            ..SimConfig::default()
        });
        for _ in 0..day_steps {
            wait_step(&sim, &mut state);
        }
        let road = state.setting.map.get(HexCoord::new(0, 0)).expect("road cell");
        match road.terrain {
            Terrain::Road(s) => s,
            other => panic!("road terrain changed to {other:?}"),
        }
    };

    // 5 stay-steps (daySteps=5 → reflections 1..=5).
    assert_eq!(run(1, 5), TrafficState::Jammed, "5 / 1 = 5 → Jammed");
    assert_eq!(run(2, 5), TrafficState::Busy, "5 / 2 = 2 → Busy");
}

/// Stay-steps count *time spent on* the road, not entries (Q27): a vehicle
/// that enters the road once and stands there for 4 reflections contributes
/// 4, not 1. With 2 teams that is `(4 + 0) / 2 = 2` → Busy, while an
/// entry-event counter would report `(1 + 0) / 2 = 0` → Smooth.
#[test]
fn traffic_counts_stay_steps_not_entry_events() {
    // (1, 0) is the road; the patrol steps Right onto it at tick 0 and then
    // waits. daySteps=5: the move departs 平地 (2 steps), lands at step 2,
    // and reflections at steps 2..=5 count 4 stay-steps. Pre-fix: 1 entry.
    let map = plain_map_with_road((1, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(0, 0), 5, 3)], 5, 2);
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::new(SimConfig {
        team_count: 2,
        traffic_thresholds: TrafficThresholds { busy_threshold: 2, jammed_threshold: 5 },
        ..SimConfig::default()
    });

    step0(&sim, &mut state, Action::step(0, Direction::Right));
    for _ in 0..4 {
        wait_step(&sim, &mut state);
    }
    assert_eq!(state.day, 1);

    let road = state.setting.map.get(HexCoord::new(1, 0)).expect("road cell");
    assert_eq!(
        road.terrain,
        Terrain::Road(TrafficState::Busy),
        "4 stay-steps / 2 teams = 2 → Busy; pre-fix the single entry event gave 0 → Smooth"
    );
}

// ---------------------------------------------------------------------------
// 3. Invalid daily response semantics (Q6: one bad action rejects the answer)
// ---------------------------------------------------------------------------

/// The official server rejects the WHOLE daily response when any action is
/// illegal / fuel-short (Q6: 「回答全体をリジェクトします」). The evaluator
/// must surface that as a per-response status flag — pre-fix it only demoted
/// the action to `Wait` and left no whole-response marker, so a run the
/// server would have refused could still look like a clean success.
#[test]
fn illegal_action_reports_whole_response_rejection() {
    let map = plain_map_with_spot((0, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 2, 1);
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::default();

    // UpperLeft from (1, 0) on an even row leaves the board — illegal.
    step0(&sim, &mut state, Action::step(0, Direction::UpperLeft));
    wait_step(&sim, &mut state);

    assert_eq!(
        state.rejected_response_count, 1,
        "the day's response must be reported as whole-response rejected \
         (pre-fix: only `illegal_action_count` moved, no response-level flag)"
    );
    assert!(state.illegal_action_count >= 1, "the per-action diagnostic still counts");
}

/// A fuel-short move is one of the official rejection reasons (Q6:
/// 「燃料切れなど不可能な移動が含まれる場合は,回答全体をリジェクトします」).
#[test]
fn fuel_short_reports_whole_response_rejection() {
    let map = plain_map_with_spot((0, 0));
    // A patrol with an empty tank cannot depart 平地 (cost 1). The spec
    // forbids a zero fuel *cap*, so the cap is set and the tank drained.
    let setting = setting(map, vec![Agent::patrol_with_fuel(0, HexCoord::new(1, 0), 1)], 2, 1);
    let mut state = MatchState::new(setting).unwrap();
    state.agents[0].fuel = 0;
    let sim = MatchSim::default();

    step0(&sim, &mut state, Action::step(0, Direction::Left));
    wait_step(&sim, &mut state);

    assert_eq!(state.rejected_response_count, 1, "fuel-short rejects the whole response");
    assert!(state.fuel_short_count >= 1);
}

/// A step-mismatched response (wrong block shape) is the third official
/// rejection reason (Q6: 「規定ステップ数を超過や不足するなど一致しない場合」).
/// Pre-fix, `MatchSim::step` silently replaced the block with all-wait.
#[test]
fn step_mismatched_response_reports_whole_response_rejection() {
    let map = plain_map_with_spot((0, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 2, 1);
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::default();

    // Only one action for a 3-agent team — not well-shaped.
    let malformed = AgentActions::new(vec![Action::wait(0)]);
    sim.step(&mut state, &malformed);
    wait_step(&sim, &mut state);

    assert_eq!(
        state.rejected_response_count, 1,
        "a step-mismatched block rejects the whole response (pre-fix: silent all-wait)"
    );
}

/// A clean response must NOT be flagged: the status is a rejection report,
/// not a counter of something benign.
#[test]
fn valid_response_is_not_flagged_as_rejected() {
    let map = plain_map_with_spot((0, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 2, 1);
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::default();

    step0(&sim, &mut state, Action::step(0, Direction::Left));
    wait_step(&sim, &mut state);

    assert_eq!(state.rejected_response_count, 0, "a legal response stays unflagged");
    assert_eq!(state.illegal_action_count, 0);
    assert_eq!(state.fuel_short_count, 0);
}

/// The rejection report survives a full `run` and reaches [`engine::model::RunResult`],
/// so the evaluator layer can read it without inspecting the final state.
#[test]
fn run_result_carries_the_whole_response_rejection_count() {
    struct OffBoard;
    impl Solver for OffBoard {
        fn name(&self) -> &'static str {
            "off-board"
        }
        fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
            let mut actions = Vec::new();
            for _ in 0..ctx.day_steps {
                for id in &ctx.agent_ids {
                    // Agent 0 walks off the board; the rest wait.
                    actions.push(if *id == 0 {
                        Action::step(*id, Direction::UpperLeft)
                    } else {
                        Action::wait(*id)
                    });
                }
            }
            AgentActions::new(actions)
        }
    }

    let map = plain_map_with_spot((0, 0));
    let setting = setting(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 3, 2);
    let sim = MatchSim::default();
    let result = sim.run(setting, &OffBoard).unwrap();

    assert_eq!(
        result.rejected_response_count, 2,
        "both days' responses are rejected whole and the count reaches RunResult"
    );
}

/// In-flight bookkeeping is per agent and deterministic: two patrols taking
/// different departure terrains land on different steps (Q10).
///
/// Guards the `in_flight` state against cross-agent bleed: agent 0's 平地
/// move (2 steps) must not be confused with agent 1's simultaneous move.
#[test]
fn in_flight_moves_are_tracked_per_agent() {
    // (1, 0) road; agent 0 at (0, 0) 平地 steps Right (due step 2),
    // agent 1 at (1, 0) road steps Right (due step 1).
    let map = plain_map_with_road((1, 0));
    let setting = setting(
        map,
        vec![
            Agent::patrol(0, HexCoord::new(0, 0), 5, 3),
            Agent::patrol(1, HexCoord::new(1, 0), 5, 3),
        ],
        3,
        1,
    );
    let mut state = MatchState::new(setting).unwrap();
    let sim = MatchSim::default();

    let ids = state.setting.agent_ids();
    let both = AgentActions::new(vec![
        Action::step(0, Direction::Right),
        Action::step(1, Direction::Right),
        Action::wait(2),
    ]);
    assert_eq!(ids.len(), 3);
    sim.step(&mut state, &both);

    assert_eq!(state.agents[0].pos, HexCoord::new(0, 0), "agent 0 still in flight (平地 = 2)");
    assert_eq!(state.agents[1].pos, HexCoord::new(1, 0), "agent 1 still in flight (due next)");

    // tick 1: agent 1's road move (1 step) lands; agent 0's does not.
    wait_step(&sim, &mut state);
    assert_eq!(state.agents[0].pos, HexCoord::new(0, 0), "agent 0 still in flight");
    assert_eq!(state.agents[1].pos, HexCoord::new(2, 0), "agent 1 landed after 1 step");

    // tick 2 (final reflection): agent 0 lands.
    wait_step(&sim, &mut state);
    assert_eq!(state.agents[0].pos, HexCoord::new(1, 0), "agent 0 landed after 2 steps");
}
