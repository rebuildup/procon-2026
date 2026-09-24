//! `MatchSim` — local, deterministic, day-step match simulator.
//!
//! Mirrors Q&A Q6's six-phase processing order per step:
//!
//! 1. **Wait** — missing or invalid actions are downgraded to `Wait`.
//! 2. **Move** — apply every agent's `Move` action (bounds/terrain/fuel).
//! 3. **Spawn** — at day boundaries, refill every spot to `max_stock`.
//! 4. **Acquire** — agents on stocked spots pick up one ball (capped at 1
//!    per agent per spot per day).
//! 5. **Fuel** — deduct move costs from patrols. Supply agents are
//!    unaffected.
//! 6. **Traffic update** — at end of day, recompute every road cell's
//!    [`TrafficState`] from today's + yesterday's counts (Q&A Q13).
//!
//! The phases are exposed both as one combined [`MatchSim::step`] (a single
//! time unit) and as the underlying primitives in [`crate::transitions`] so
//! tests can exercise each in isolation.
//!
//! # Determinism
//!
//! Given the same [`MatchSetting`], [`crate::model::AgentActions`] sequence,
//! and [`crate::model::Solver`], the produced [`MatchState`] and
//! [`RunResult`] are byte-identical across runs. There is no RNG, no clock,
//! and no I/O in the simulator.

use std::collections::HashMap;

use crate::model::{AgentActions, MatchSetting, MatchState, RunResult, Solver, SolverContext};
use crate::rules::TrafficThresholds;
use crate::transitions::{
    MoveOutcome, StepEvent, apply_moves, begin_day, phase_acquire, phase_fuel, phase_spawn,
    phase_traffic_update, phase_wait, record_traversal, refill_fuel_if_supply_present,
};

/// Configuration for [`MatchSim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimConfig {
    pub traffic_thresholds: TrafficThresholds,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { traffic_thresholds: TrafficThresholds::default() }
    }
}

/// One day-step simulator for a single team.
///
/// `MatchSim` is the entry point for [`MatchSim::run`] and [`MatchSim::step`].
/// It owns no state of its own — the live state lives in the [`MatchState`]
/// passed in — so the same simulator instance can drive many matches in
/// sequence (which matters for batch benchmarks).
#[derive(Debug, Clone, Copy)]
pub struct MatchSim {
    config: SimConfig,
}

impl Default for MatchSim {
    fn default() -> Self {
        Self::new(SimConfig::default())
    }
}

impl MatchSim {
    #[must_use]
    pub const fn new(config: SimConfig) -> Self {
        Self { config }
    }

    /// Step the simulator by one time unit.
    ///
    /// `decisions` is one step's worth of actions for the calling team
    /// (one action per controlled agent, in agent-id ascending order). Any
    /// deviation from that shape degrades to "every agent waits" for that
    /// step (Q&A Q6 — invalid answers are rejected as a whole).
    pub fn step(&self, state: &mut MatchState, decisions: &AgentActions) -> Vec<StepEvent> {
        let mut events = Vec::new();
        let agent_ids = state.setting.agent_ids();
        let well_shaped = decisions.is_well_shaped(&agent_ids, 1);
        if !well_shaped {
            // Replace with all-wait for this step.
            let waits = AgentActions::wait_all(&agent_ids, 1);
            return self.step(state, &waits);
        }

        // 3. Spawn — at the *start* of each new day, refill spot stock.
        //    Day 0 step 0 has already been initialised by [`MatchState::new`],
        //    so this only fires from day 1 onwards.
        if state.day_step == 0 && state.day > 0 && state.day < state.num_days() {
            phase_spawn(&mut state.setting.map, &mut events);
        }

        // 1. Wait — filter unknown / duplicate ids.
        let resolved = phase_wait(&decisions.actions, &agent_ids);

        // 2. Move — apply each agent's Move; downgrade illegal moves to Wait.
        let mut move_outcomes: HashMap<u8, MoveOutcome> = HashMap::new();
        let agents_snapshot = state.agents.clone();
        for (id, action) in &resolved {
            let Some(agent) = agents_snapshot.iter().find(|a| a.id == *id) else {
                continue;
            };
            let mut per_step = Vec::new();
            let outcome = crate::transitions::try_move(
                agent,
                *action,
                &state.setting.map,
                &mut per_step,
                &mut events,
            );
            move_outcomes.insert(*id, outcome);
        }
        let costs = apply_moves(&mut state.agents, &move_outcomes);

        // Record traffic for each successful traversal.
        for (id, outcome) in &move_outcomes {
            if let MoveOutcome::Moved { to, .. } = outcome {
                record_traversal(state, *to);
            }
            let _ = id; // suppress unused
        }

        // 4. Acquire — agents on stocked spots pick up.
        let agents_for_acquire = state.agents.clone();
        for agent in &agents_for_acquire {
            phase_acquire(agent, state, &mut events);
        }

        // 5. Fuel — deduct move costs from patrols.
        phase_fuel(&mut state.agents, &costs, &mut events);

        // Supply refuel — share-cell rescue.
        let agent_positions: Vec<_> = state.agents.iter().map(|a| a.pos).collect();
        for pos in agent_positions {
            refill_fuel_if_supply_present(&mut state.agents, pos, &mut events);
        }

        // Advance clock.
        state.day_step += 1;
        if state.day_step >= state.day_steps() {
            // 6. Traffic update — at the end of the current day.
            let updates = phase_traffic_update(state, self.config.traffic_thresholds);
            if !updates.is_empty() {
                events.push(StepEvent::TrafficUpdated { updates });
            }
            begin_day(state);
            state.day += 1;
            state.day_step = 0;
        }

        events
    }

    /// Drive a full match from `setting` using `solver` for every day.
    ///
    /// Returns the final state and the three *deterministic* competitive
    /// tallies of 募集要項「順位決定」:
    ///
    /// 1. `variety` (distinct series ever acquired) — primary.
    /// 2. `daily_variety_sum` (Σ per-day distinct series) — secondary.
    /// 3. `total_balls` — tertiary.
    ///
    /// Tier 4 — cumulative per-day response time — is **not** produced here:
    /// the simulator has no clock by design (see the determinism section
    /// above). The evaluation layer measures solver latency and combines it
    /// with these three into [`domain::Score`].
    ///
    /// `solver` may be a `Box<dyn Solver>` (typical) or any concrete type.
    pub fn run<S: Solver + ?Sized>(
        &self,
        setting: MatchSetting,
        solver: &S,
    ) -> Result<RunResult, String> {
        let mut state = MatchState::new(setting)?;
        let last_acq = self.run_inner(&mut state, solver)?;
        let (variety, daily_variety_sum, total_balls) = score_state(&state);
        Ok(RunResult {
            final_state: state,
            variety,
            daily_variety_sum,
            total_balls,
            last_acquisition_at: last_acq,
        })
    }

    /// Drive a full match from an already-prepared [`MatchState`]. Useful for
    /// batch drivers that share initial state across solvers.
    pub fn run_from_state<S: Solver + ?Sized>(
        &self,
        state: &mut MatchState,
        solver: &S,
    ) -> Result<RunResult, String> {
        let last_acq = self.run_inner(state, solver)?;
        let (variety, daily_variety_sum, total_balls) = score_state(state);
        Ok(RunResult {
            final_state: state.clone(),
            variety,
            daily_variety_sum,
            total_balls,
            last_acquisition_at: last_acq,
        })
    }

    fn run_inner<S: Solver + ?Sized>(
        &self,
        state: &mut MatchState,
        solver: &S,
    ) -> Result<Option<(u32, u32)>, String> {
        // The solver returns ONE day's worth of actions (step-major:
        // agent_count actions per step, day_steps blocks). We call
        // [`Self::step`] for each block.
        let total_days = state.num_days();
        let day_steps = state.day_steps();
        for _ in 0..total_days {
            if state.is_finished() {
                break;
            }
            let ctx = SolverContext::from_state(state);
            let day_plan = solver.decide(&ctx);
            // Slice the per-day plan into per-step blocks and feed each to
            // [`Self::step`]. An under-/over-sized plan is treated as
            // "all-wait" so a misbehaving solver cannot crash the match.
            let agent_count = ctx.agent_ids.len();
            let agent_ids_owned = ctx.agent_ids.clone();
            let per_step = day_steps as usize;
            for s in 0..per_step {
                if state.is_finished() {
                    break;
                }
                let start = s * agent_count;
                let end = start + agent_count;
                let block = if day_plan.actions.len() >= end {
                    day_plan.actions[start..end].to_vec()
                } else {
                    AgentActions::wait_all(&agent_ids_owned, 1).actions
                };
                self.step(state, &AgentActions::new(block));
            }
        }
        Ok(latest_acquisition(state))
    }
}

/// Compute the three integer scores from the final state.
///
/// `daily_variety_sum` is 募集要項「順位決定」tier 2 — 「各日の種類数の累積」:
/// the number of distinct series acquired *that day*, summed over all days.
/// A series re-acquired on a later day contributes again for that later day,
/// so this is **not** the count of first-acquisition days.
fn score_state(state: &MatchState) -> (u32, u32, u32) {
    let variety = state.series_taken.len() as u32;
    let total_balls: u32 = state.agents.iter().map(|a| a.total_inventory()).sum();
    let daily_variety_sum: u32 = state.daily_varieties().iter().sum();
    (variety, daily_variety_sum, total_balls)
}

/// `(day, step)` of the latest acquisition recorded in the state.
fn latest_acquisition(state: &MatchState) -> Option<(u32, u32)> {
    state.series_first_taken_at.values().max_by_key(|(d, s)| (*d, *s)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Action, Agent, Direction, HexCoord, MapGrid, MapSize, Spot, Terrain, TrafficState,
    };
    use domain::{MatchId, PlayerId};

    /// Tiny 8x8 plain map with one spot at (0, 0) of series 1.
    fn one_spot_map() -> MapGrid {
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..i32::from(size.height) {
            for col in 0..i32::from(size.width) {
                let (terrain, spot) = if (col, row) == (0, 0) {
                    (
                        Terrain::Plain,
                        Some(Spot {
                            series: 1,
                            pos: HexCoord::new(0, 0),
                            max_stock: 1,
                            current_stock: 1,
                        }),
                    )
                } else {
                    (Terrain::Plain, None)
                };
                cells.push(crate::model::Cell { coord: HexCoord::new(col, row), terrain, spot });
            }
        }
        MapGrid::new(size, cells).unwrap()
    }

    /// 8x8 plain map with one spot of series 1 at (0, 0) and one of series 2
    /// at (7, 7).
    fn two_spot_map() -> MapGrid {
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..i32::from(size.height) {
            for col in 0..i32::from(size.width) {
                let spot = if (col, row) == (0, 0) {
                    Some(Spot {
                        series: 1,
                        pos: HexCoord::new(0, 0),
                        max_stock: 3,
                        current_stock: 3,
                    })
                } else if (col, row) == (7, 7) {
                    Some(Spot {
                        series: 2,
                        pos: HexCoord::new(7, 7),
                        max_stock: 3,
                        current_stock: 3,
                    })
                } else {
                    None
                };
                cells.push(crate::model::Cell {
                    coord: HexCoord::new(col, row),
                    terrain: Terrain::Plain,
                    spot,
                });
            }
        }
        MapGrid::new(size, cells).unwrap()
    }

    fn setting_with(
        map: MapGrid,
        agents: Vec<Agent>,
        day_steps: u32,
        num_days: u32,
    ) -> MatchSetting {
        // Pad to 3 agents (spec minimum) if the caller only supplied 1 or 2.
        // Padding agents sit far from any spot so the spec's "no spot at
        // start" rule is honoured even on maps that have spots in the corners.
        let mut agents = agents;
        while agents.len() < 3 {
            let id = agents.len() as u8;
            agents.push(Agent::supply(id, HexCoord::new(3, 3)));
        }
        MatchSetting {
            match_id: MatchId::new("match-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents,
            day_steps,
            num_days,
        }
    }

    struct WaitSolver;

    impl Solver for WaitSolver {
        fn name(&self) -> &'static str {
            "wait"
        }
        fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
            AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
        }
    }

    #[test]
    fn step_advances_state_and_consumes_fuel() {
        let map = one_spot_map();
        let setting = setting_with(map, vec![Agent::patrol(0, HexCoord::new(7, 7), 5, 3)], 5, 1);
        let mut state = MatchState::new(setting).unwrap();
        let sim = MatchSim::default();
        // One step Right from (7,7) -> (8 doesn't exist; wrap fails). Use Left instead.
        // Direction::Left from (7,7) -> (6,7) on a plain (cost 1).
        let step_actions = AgentActions::new(vec![
            Action::step(0, Direction::Left),
            Action::wait(1),
            Action::wait(2),
        ]);
        sim.step(&mut state, &step_actions);
        assert_eq!(state.agents[0].pos, HexCoord::new(6, 7));
        assert_eq!(state.agents[0].fuel, 5 * 3 - 1);
        assert_eq!(state.day_step, 1);
    }

    #[test]
    fn six_phase_ordering_at_day_boundary() {
        // Use a 1-day map with day_steps=2; the second step rolls over the day.
        // Agent starts at (1,0) — one cell to the right of the spot at (0,0),
        // then waits step 0 (no pickup), then waits step 1 (end of day).
        // The pickup happens via a separate step from (0,0) first.
        let map = one_spot_map();
        let setting = setting_with(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 2, 1);
        let mut state = MatchState::new(setting).unwrap();
        let sim = MatchSim::default();
        // Step 0: move Left from (1,0) -> (0,0) (cost 1, plain).
        let step_a = AgentActions::new(vec![
            Action::step(0, Direction::Left),
            Action::wait(1),
            Action::wait(2),
        ]);
        let events = sim.step(&mut state, &step_a);
        assert!(events.iter().any(|e| matches!(e, StepEvent::Picked { series: 1, .. })));
        assert_eq!(state.agents[0].inventory.get(&1).copied().unwrap_or(0), 1);
        assert_eq!(state.day_step, 1);
        assert_eq!(state.day, 0);

        // Step 1: end of day -> traffic update + day advance.
        let wait = AgentActions::wait_all(&[0, 1, 2], 1);
        sim.step(&mut state, &wait);
        assert_eq!(state.day, 1);
        assert_eq!(state.day_step, 0);
        assert!(state.is_finished());
    }

    #[test]
    fn run_returns_scoring_summary() {
        let map = two_spot_map();
        let setting = setting_with(
            map,
            vec![
                Agent::patrol(0, HexCoord::new(1, 0), 200, 3),
                Agent::patrol(1, HexCoord::new(6, 7), 200, 3),
            ],
            20,
            3,
        );
        // Each agent walks toward its nearby spot, then waits.
        struct TwoSpotSolver;
        impl Solver for TwoSpotSolver {
            fn name(&self) -> &'static str {
                "two-spot"
            }
            fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
                let mut actions = Vec::new();
                for _ in 0..ctx.day_steps {
                    for id in &ctx.agent_ids {
                        // agent 0 walks Left toward (0,0); agent 1 walks Right toward (7,7).
                        let dir = if *id == 0 { Direction::Left } else { Direction::Right };
                        actions.push(Action::step(*id, dir));
                    }
                }
                AgentActions::new(actions)
            }
        }
        let sim = MatchSim::default();
        let result = sim.run(setting, &TwoSpotSolver).unwrap();
        // We expect at least the two distinct series to be picked.
        assert!(result.variety >= 2, "variety = {}", result.variety);
        assert!(result.total_balls >= 2);
    }

    /// 募集要項「順位決定」tier 2 is 「各日の種類数の累積」: the distinct
    /// series acquired *that day*, summed over all days. A series that is
    /// re-acquired on a later day contributes again for that later day —
    /// so `daily_variety_sum` must exceed `variety` once a match spans more
    /// than one day of collection. Regresses the older implementation that
    /// counted each series only on the day of its *first* acquisition.
    #[test]
    fn daily_variety_sum_counts_reseries_each_day() {
        let map = one_spot_map();
        // One spot (series 1, stock 1, refilled each day). One patrol starts
        // next to it at (1, 0) — spec forbids starting on a spot — and steps
        // Left every step. Step 0 of each day lands on the spot and picks up;
        // the following step's Left is out of bounds and degrades to Wait, so
        // the agent stays put and re-picks after each day's spawn refill.
        let setting = setting_with(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 8, 3)], 2, 3);
        struct AlwaysLeft;
        impl Solver for AlwaysLeft {
            fn name(&self) -> &'static str {
                "always-left"
            }
            fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
                let mut actions = Vec::new();
                for _ in 0..ctx.day_steps {
                    for id in &ctx.agent_ids {
                        actions.push(Action::step(*id, Direction::Left));
                    }
                }
                AgentActions::new(actions)
            }
        }
        let sim = MatchSim::default();
        let result = sim.run(setting, &AlwaysLeft).unwrap();
        // One distinct series ever -> variety == 1.
        assert_eq!(result.variety, 1);
        // But it is collected on more than one day, so the cumulative
        // per-day variety must be strictly greater than `variety`.
        assert!(
            result.daily_variety_sum > result.variety,
            "daily_variety_sum = {} must exceed variety = {} when a series \
             is re-acquired on later days",
            result.daily_variety_sum,
            result.variety
        );
        // Per-day breakdown agrees with the aggregate.
        assert_eq!(
            result.daily_variety_sum,
            result.final_state.daily_varieties().iter().sum::<u32>()
        );
    }

    #[test]
    fn run_is_deterministic() {
        let map = two_spot_map();
        let setting_a =
            setting_with(map.clone(), vec![Agent::patrol(0, HexCoord::new(3, 3), 200, 3)], 20, 2);
        let setting_b =
            setting_with(map, vec![Agent::patrol(0, HexCoord::new(3, 3), 200, 3)], 20, 2);
        let sim = MatchSim::default();
        let r1 = sim.run(setting_a, &WaitSolver).unwrap();
        let r2 = sim.run(setting_b, &WaitSolver).unwrap();
        assert_eq!(r1.final_state, r2.final_state);
        assert_eq!(r1.variety, r2.variety);
        assert_eq!(r1.total_balls, r2.total_balls);
    }

    #[test]
    fn illegal_move_is_downgraded_to_wait() {
        let map = one_spot_map();
        let setting = setting_with(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 1, 1);
        let mut state = MatchState::new(setting).unwrap();
        let sim = MatchSim::default();
        // Direction::UpperLeft from (1,0) -> row 0 even, UpperLeft = (0,-1) -> out of bounds.
        let bad = AgentActions::new(vec![
            Action::step(0, Direction::UpperLeft),
            Action::wait(1),
            Action::wait(2),
        ]);
        let events = sim.step(&mut state, &bad);
        assert!(events.iter().any(|e| matches!(e, StepEvent::IllegalMove { .. })));
        assert_eq!(state.agents[0].pos, HexCoord::new(1, 0));
        assert_eq!(state.agents[0].fuel, 5 * 3); // untouched
    }

    #[test]
    fn spot_refills_each_day() {
        // Verify the spawn phase: after a pickup empties the spot, the next
        // day's spawn refills it back to `max_stock`. We sidestep the
        // re-acquisition issue by moving the agent off the spot before the
        // day rolls over.
        let map = one_spot_map();
        let setting = setting_with(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 100, 3)], 3, 2);
        let mut state = MatchState::new(setting).unwrap();
        let sim = MatchSim::default();
        // Day 0 step 0: move onto the spot and pick up.
        sim.step(
            &mut state,
            &AgentActions::new(vec![
                Action::step(0, Direction::Left),
                Action::wait(1),
                Action::wait(2),
            ]),
        );
        assert_eq!(
            state
                .setting
                .map
                .get(HexCoord::new(0, 0))
                .unwrap()
                .spot
                .as_ref()
                .unwrap()
                .current_stock,
            0
        );
        // Day 0 step 1: step back off the spot so the day rolls over without
        // a second pickup (the cap would allow it, but we want to observe
        // the refilled stock).
        sim.step(
            &mut state,
            &AgentActions::new(vec![
                Action::step(0, Direction::Right),
                Action::wait(1),
                Action::wait(2),
            ]),
        );
        // Day 0 step 2 (wait) — ends the day. spawn fires at the start of
        // day 1, refilling stock — so we need one more step call to enter
        // day 1 step 0.
        sim.step(&mut state, &AgentActions::wait_all(&[0, 1, 2], 1));
        // Day 1 step 0: spawn fires, refilling stock to max=1. Agent is
        // off the spot (it moved Right on step 1) so no re-pickup happens.
        sim.step(&mut state, &AgentActions::wait_all(&[0, 1, 2], 1));
        // After day rollover + day 1 step 0, spawn should have refilled
        // stock to max=1.
        assert_eq!(state.day, 1);
        assert_eq!(
            state
                .setting
                .map
                .get(HexCoord::new(0, 0))
                .unwrap()
                .spot
                .as_ref()
                .unwrap()
                .current_stock,
            1
        );
    }

    #[test]
    fn traffic_state_promotes_when_over_threshold() {
        // Build an 8x8 map with a road at (0,0) Smooth; bump yesterday count.
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
        let setting = setting_with(map, vec![Agent::patrol(0, HexCoord::new(7, 7), 5, 3)], 1, 2);
        let mut state = MatchState::new(setting).unwrap();
        // Pretend yesterday the road was busy.
        state.traffic_yesterday.insert(HexCoord::new(0, 0), 3);
        let sim = MatchSim::default();
        // 1 day x 1 step -> end of day runs traffic update.
        let wait = AgentActions::wait_all(&[0, 1, 2], 1);
        let events = sim.step(&mut state, &wait);
        // Find the TrafficUpdated event and assert the road is now Busy.
        let traffic = events
            .iter()
            .find_map(|e| {
                if let StepEvent::TrafficUpdated { updates } = e { Some(updates) } else { None }
            })
            .expect("traffic update event");
        assert!(traffic.iter().any(|(c, s)| *c == HexCoord::new(0, 0) && *s == TrafficState::Busy));
    }
}
