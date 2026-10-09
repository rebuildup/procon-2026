//! `MatchSim` — local, deterministic, day-step match simulator.
//!
//! Mirrors Q&A Q6's tick timeline. A day of `day_steps = N` runs the ticks
//! `0..=N`:
//!
//! * tick 0 — action phase only (no reflection, no うどん acquisition: Q7);
//! * ticks `1..=N-1` — reflection phase, then action phase;
//! * tick `N` (the day's final step) — reflection phase only.
//!
//! The reflection phase is (Q6):
//!
//! 1. **Fuel** — charge every in-flight move its departure terrain's fuel.
//! 2. **Move** — land the in-flight moves due at this tick (a Move reserved
//!    at tick `t` lands at `t + terrain_step_cost(origin)` — Q10).
//! 3. **Acquire** — **Patrol** agents on stocked spots pick up one ball
//!    (capped at 1 per agent per spot per day). `AgentKind::Supply` cannot
//!    collect 玉 and is skipped entirely (issue #234).
//! 4. **Refuel** — supply agents sharing a cell refill patrols.
//! 5. **Stay-count** — one stay-step per agent at its post-move position,
//!    for the road-traffic average (Q27).
//!
//! The action phase reserves each agent's next action into a per-agent queue
//! (the daily answer is a *sequential* action list — Q23/Q24: the next order
//! is reserved only when the previous one completes). At the day boundary the
//! road states are recomputed from the all-team stay-step average over the
//! past 2 days (Q&A Q13) and spot stock refills ([`crate::transitions::
//! phase_spawn`]).
//!
//! The phases are exposed both as one combined [`MatchSim::step`] (a single
//! tick) and as the underlying primitives in [`crate::transitions`] so
//! tests can exercise each in isolation.
//!
//! # Answer validity (Q6)
//!
//! One illegal / fuel-short / step-mismatched action rejects the WHOLE daily
//! response. The simulator demotes the offending actions to `Wait` so the
//! match still completes deterministically, but records the official verdict
//! in [`MatchState::rejected_response_count`] — an evaluator must not rank a
//! run the live server would have refused.
//!
//! # Determinism
//!
//! Given the same [`MatchSetting`], [`crate::model::AgentActions`] sequence,
//! and [`crate::model::Solver`], the produced [`MatchState`] and
//! [`RunResult`] are byte-identical across runs. There is no RNG, no clock,
//! and no I/O in the simulator.

use crate::model::{
    AgentActions, MatchSetting, MatchState, PendingAction, RunResult, Solver, SolverContext,
};
use crate::rules::TrafficThresholds;
use crate::transitions::{
    ReserveReject, StepEvent, apply_due_moves, begin_day, phase_acquire, phase_oracle_refill,
    phase_spawn, phase_traffic_update, phase_wait, record_stay_steps,
    refill_fuel_if_supply_present, try_reserve,
};

/// Configuration for [`MatchSim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimConfig {
    pub traffic_thresholds: TrafficThresholds,
    /// Counterfactual day-boundary tank refill (Issue #243, `oracle_refill`).
    ///
    /// `false` (the default) is the official rule and must stay
    /// byte-identical: the only thing that ever refuels a Patrol tank in a
    /// real match is supply contact ([`crate::transitions::
    /// refill_fuel_if_supply_present`]). `true` additionally restores every
    /// Patrol tank to its cap at each day boundary — an **analysis-only**
    /// counterfactual used to measure how much of a result fuel exhaustion
    /// explains, never a default evidence path.
    pub oracle_refill: bool,
    /// Number of teams in the match (the wire's `players`).
    ///
    /// The official road-traffic value divides the 2-day stay-step sum by
    /// the team count (募集要項「道路セルと交通量」). Offline evaluation
    /// observes only its own team's stay-steps — other teams' contributions
    /// are counted as 0 rather than fabricated from guessed trajectories —
    /// so live callers must set this from the match's `players` to classify
    /// roads the way the server does. `0` is treated as `1`.
    pub team_count: u32,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            traffic_thresholds: TrafficThresholds::default(),
            oracle_refill: false,
            team_count: 1,
        }
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

    /// Advance the simulator by one tick of the official timeline.
    ///
    /// `decisions` is one tick's worth of actions for the calling team
    /// (one action per controlled agent, in agent-id ascending order) —
    /// the actions *accepted* at this tick's action phase. They join each
    /// agent's queue and start when the agent is free (Q6: the daily answer
    /// is a sequential action list).
    ///
    /// Any shape deviation is a step-mismatched answer: the block is
    /// replaced with all-wait and the whole response is flagged as rejected
    /// (Q&A Q6 — invalid answers are rejected as a whole).
    pub fn step(&self, state: &mut MatchState, decisions: &AgentActions) -> Vec<StepEvent> {
        let mut events = Vec::new();
        let agent_ids = state.setting.agent_ids();
        if !decisions.is_well_shaped(&agent_ids, 1) {
            // Step-mismatched answer: flag the whole response and continue
            // with all-wait so the match still completes.
            state.day_response_invalid = true;
            let waits = AgentActions::wait_all(&agent_ids, 1);
            return self.step(state, &waits);
        }

        let now = state.day_step;
        let day_limit = state.day_steps();

        // Spawn — at the *start* of each new day, refill spot stock.
        // Day 0 step 0 has already been initialised by [`MatchState::new`],
        // so this only fires from day 1 onwards.
        if now == 0 && state.day > 0 && state.day < state.num_days() {
            phase_spawn(&mut state.setting.map, &mut events);
            // Counterfactual (Issue #243): only under `oracle_refill`, also
            // top every Patrol tank up to its cap at the same boundary. Gated
            // on the flag so the default path is untouched, byte for byte.
            if self.config.oracle_refill {
                phase_oracle_refill(&mut state.agents, &mut events);
            }
        }

        // Reflection phase (Q6) — tick 0 is action-phase-only (Q7), so the
        // first reflection of a day runs at tick 1.
        if now > 0 {
            self.reflect(state, now, &mut events);
        }

        // Action phase (Q6): accept this tick's actions into the queues and
        // start whatever the agents are free for.
        self.accept(state, decisions, now, day_limit, &mut events);

        // Advance clock. When the day's last action phase (tick N-1) is done,
        // tick N's reflection closes the day.
        state.day_step += 1;
        if state.day_step >= day_limit {
            self.reflect(state, state.day_step, &mut events);
            // Whole-response verdict (Q6): flush the day's flag.
            if state.day_response_invalid {
                state.rejected_response_count = state.rejected_response_count.saturating_add(1);
                state.rejected_days.insert(state.day);
            }
            // 6. Traffic update — at the end of the current day.
            let updates =
                phase_traffic_update(state, self.config.traffic_thresholds, self.config.team_count);
            if !updates.is_empty() {
                events.push(StepEvent::TrafficUpdated { updates });
            }
            begin_day(state);
            state.day += 1;
            state.day_step = 0;
        }

        events
    }

    /// Q6 reflection: fuel → move-apply → acquire → refuel, then the
    /// stay-step count (Q27). `now` is the tick being reflected.
    fn reflect(&self, state: &mut MatchState, now: u32, events: &mut Vec<StepEvent>) {
        // 1+2. Fuel is charged and the position updates at the move's due
        // reflection (Q6: 燃料の消費 → 移動の反映).
        apply_due_moves(state, now, events);

        // 3. Acquire — Patrol agents on stocked spots pick up. `phase_acquire`
        //    itself skips Supply (issue #234), so looping every agent here
        //    keeps the call site simple and kind-agnostic.
        let agents_for_acquire = state.agents.clone();
        for agent in &agents_for_acquire {
            phase_acquire(agent, state, events);
        }

        // 4. Refuel — same-cell supply contact (after the acquire: Q6).
        let agent_positions: Vec<_> = state.agents.iter().map(|a| a.pos).collect();
        for pos in agent_positions {
            refill_fuel_if_supply_present(&mut state.agents, pos, events);
        }

        // 5. Stay-steps for the road-traffic average (Q27: post-move
        //    positions; every reflection counts, step 0 does not).
        record_stay_steps(state);
    }

    /// Q6 action phase at tick `now`: enqueue each agent's next action and
    /// start it if the agent is free (the previous action has completed).
    fn accept(
        &self,
        state: &mut MatchState,
        decisions: &AgentActions,
        now: u32,
        day_limit: u32,
        events: &mut Vec<StepEvent>,
    ) {
        let agent_ids = state.setting.agent_ids();
        let resolved = phase_wait(&decisions.actions, &agent_ids);
        for (id, action) in resolved {
            state.action_queues.entry(id).or_default().push_back(action);
        }
        // Start free agents' next actions, in agent-id order.
        for id in agent_ids {
            let free = state.in_flight.get(&id).is_none_or(|pending| pending.due_step() <= now);
            if !free {
                continue;
            }
            // Retire a completed in-flight record before starting the next.
            state.in_flight.remove(&id);
            let Some(action) = state.action_queues.get_mut(&id).and_then(|q| q.pop_front()) else {
                continue;
            };
            let Some(agent) = state.agents.iter().find(|a| a.id == id) else {
                continue;
            };
            match try_reserve(agent, action, &state.setting.map, now, day_limit) {
                Ok(pending) => {
                    state.in_flight.insert(id, pending);
                }
                Err(ReserveReject::Truncated) => {
                    // The move could not land inside the day: drop the
                    // rest of this agent's answer (the same truncate the
                    // wire normalizer applies before submission).
                    state.action_queues.remove(&id);
                }
                Err(ReserveReject::Illegal) => {
                    events.push(StepEvent::IllegalMove {
                        agent_id: id,
                        reason: "illegal move order (Q6: whole response rejected)".to_owned(),
                    });
                    state.illegal_action_count = state.illegal_action_count.saturating_add(1);
                    state.day_response_invalid = true;
                    // Demote to a 1-tick wait so the timeline advances.
                    state.in_flight.insert(id, PendingAction::Wait { until_step: now + 1 });
                }
            }
        }
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
        let (illegal, fuel_short, rejected) =
            (state.illegal_action_count, state.fuel_short_count, state.rejected_response_count);
        Ok(RunResult {
            final_state: state,
            variety,
            daily_variety_sum,
            total_balls,
            last_acquisition_at: last_acq,
            illegal_action_count: illegal,
            fuel_short_count: fuel_short,
            rejected_response_count: rejected,
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
        let (illegal, fuel_short, rejected) =
            (state.illegal_action_count, state.fuel_short_count, state.rejected_response_count);
        Ok(RunResult {
            final_state: state.clone(),
            variety,
            daily_variety_sum,
            total_balls,
            last_acquisition_at: last_acq,
            illegal_action_count: illegal,
            fuel_short_count: fuel_short,
            rejected_response_count: rejected,
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
        for _ in 0..total_days {
            if state.is_finished() {
                break;
            }
            let ctx = SolverContext::from_state(state);
            let day_plan = solver.decide(&ctx);
            // Slice the per-day plan into per-step blocks and feed each to
            // [`Self::step`]. An under-/over-sized plan is treated as
            // "all-wait" so a misbehaving solver cannot crash the match.
            //
            // Read the step count *inside* the day loop: `daySteps` is a
            // per-day vector, so day 1 may allow a different number of steps
            // from day 0. Hoisting this read above the loop was a real bug —
            // `daySteps = [16, 24]` ran day 1 for only 16 steps.
            let agent_count = ctx.agent_ids.len();
            let agent_ids_owned = ctx.agent_ids.clone();
            let per_step = ctx.day_steps as usize;
            // A day plan whose length is not exactly one block per step is
            // step-mismatched (Q6: 「規定ステップ数を超過や不足する」) — the
            // official server rejects the whole answer. Flag it, then pad /
            // truncate so the match still completes.
            if day_plan.actions.len() != agent_count.saturating_mul(per_step) {
                state.day_response_invalid = true;
            }
            for s in 0..per_step {
                if state.is_finished() {
                    break;
                }
                let start = s * agent_count;
                let end = start + agent_count;
                let block = if day_plan.actions.len() >= end {
                    day_plan.actions[start..end].to_vec()
                } else {
                    state.day_response_invalid = true;
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
            day_steps_by_day: Vec::new(),
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
        // Direction::Left from (7,7) -> (6,7), departing 平地 (2 steps).
        // Q6: the move is reserved at the tick-0 action phase and lands (and
        // pays its fuel) at the reflection of step 2.
        let step_actions = AgentActions::new(vec![
            Action::step(0, Direction::Left),
            Action::wait(1),
            Action::wait(2),
        ]);
        sim.step(&mut state, &step_actions);
        assert_eq!(state.agents[0].pos, HexCoord::new(7, 7), "reserved, not yet landed");
        assert_eq!(state.agents[0].fuel, 5 * 3, "fuel is charged at the landing reflection");
        assert_eq!(state.day_step, 1);
        // Tick 1: nothing due.
        sim.step(&mut state, &AgentActions::wait_all(&[0, 1, 2], 1));
        assert_eq!(state.agents[0].pos, HexCoord::new(7, 7));
        // Tick 2's reflection lands the move and charges the fuel.
        sim.step(&mut state, &AgentActions::wait_all(&[0, 1, 2], 1));
        assert_eq!(state.agents[0].pos, HexCoord::new(6, 7));
        assert_eq!(state.agents[0].fuel, 5 * 3 - 1);
    }

    /// Issue #243 — the `oracle_refill` counterfactual, both halves:
    ///
    /// 1. **Default is byte-identical** — `SimConfig::default()` carries the
    ///    flag off, and an explicit `oracle_refill: false` config drives the
    ///    exact same fuel trajectory (fuel carries across the day boundary
    ///    unchanged, the official rule).
    /// 2. **`true` refills at the day boundary** — the same steps with the
    ///    flag on end day 1 with a full tank.
    #[test]
    fn oracle_refill_defaults_off_and_refills_patrol_tanks_at_the_day_boundary() {
        // Drive one identical mini-match and report agent 0's fuel at the
        // start of day 1 (after the boundary fired).
        let fuel_at_day1_start = |config: SimConfig| -> u32 {
            let map = one_spot_map();
            // Patrol cap = day_steps(2) x multiplier(2) = 4.
            let setting =
                setting_with(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 2, 2)], 2, 2);
            let mut state = MatchState::new(setting).unwrap();
            let sim = MatchSim::new(config);
            let left = AgentActions::new(vec![
                Action::step(0, Direction::Left),
                Action::wait(1),
                Action::wait(2),
            ]);
            let wait = AgentActions::wait_all(&[0, 1, 2], 1);

            // Day 0 step 0: step onto the spot (plain, cost 1) -> 4 - 1 = 3.
            sim.step(&mut state, &left);
            // Day 0 step 1: wait out the rest of the day -> rollover to day 1.
            sim.step(&mut state, &wait);
            assert_eq!(state.day, 1, "the match must have crossed a day boundary");
            // Day 1 step 0: spawn + (only if enabled) oracle refill, then wait.
            sim.step(&mut state, &wait);
            state.agents[0].fuel
        };

        // Half 1 — the default and an explicit off are the same trajectory.
        assert!(!SimConfig::default().oracle_refill, "the counterfactual must default to off");
        let off = fuel_at_day1_start(SimConfig::default());
        let explicit_off =
            fuel_at_day1_start(SimConfig { oracle_refill: false, ..SimConfig::default() });
        assert_eq!(off, explicit_off, "default must be byte-identical to an explicit `false`");
        assert_eq!(off, 3, "official rule: the day boundary does NOT refill a Patrol tank");

        // Half 2 — the counterfactual refills to cap at the same boundary.
        let on = fuel_at_day1_start(SimConfig { oracle_refill: true, ..SimConfig::default() });
        assert_eq!(on, 4, "oracle_refill=true must restore the tank to cap at the day boundary");
    }

    #[test]
    fn six_phase_ordering_at_day_boundary() {
        // Use a 1-day map with day_steps=2; the second step rolls over the day.
        // Agent starts at (1,0) — one cell to the right of the spot at (0,0).
        // Q6: the 平地 move is reserved at step 0 and lands at the final
        // reflection (step 2), which acquires from the spot before the day
        // boundary runs the traffic update.
        let map = one_spot_map();
        let setting = setting_with(map, vec![Agent::patrol(0, HexCoord::new(1, 0), 5, 3)], 2, 1);
        let mut state = MatchState::new(setting).unwrap();
        let sim = MatchSim::default();
        // Step 0 (action phase only): reserve Left (1,0) -> (0,0).
        let step_a = AgentActions::new(vec![
            Action::step(0, Direction::Left),
            Action::wait(1),
            Action::wait(2),
        ]);
        let events = sim.step(&mut state, &step_a);
        assert!(
            !events.iter().any(|e| matches!(e, StepEvent::Picked { .. })),
            "no acquisition before the move lands (step 0 is action-phase-only)"
        );
        assert_eq!(state.agents[0].inventory.get(&1).copied().unwrap_or(0), 0);
        assert_eq!(state.day_step, 1);
        assert_eq!(state.day, 0);

        // Step 1: reflection at 1 (nothing due) + action; then the final
        // reflection at step 2 lands the move, acquires, and the day rolls
        // over (traffic update + day advance).
        let wait = AgentActions::wait_all(&[0, 1, 2], 1);
        let events = sim.step(&mut state, &wait);
        assert!(
            events.iter().any(|e| matches!(e, StepEvent::Picked { series: 1, .. })),
            "the arrival step acquires from the spot"
        );
        assert_eq!(state.agents[0].inventory.get(&1).copied().unwrap_or(0), 1);
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

    /// Plain 8x8 map with no spots, so nothing can interfere with the step
    /// counting.
    fn empty_plain_map() -> MapGrid {
        let size = MapSize::new(8, 8).unwrap();
        let cells = (0..8)
            .flat_map(|row| {
                (0..8).map(move |col| crate::model::Cell {
                    coord: HexCoord::new(col, row),
                    terrain: Terrain::Plain,
                    spot: None,
                })
            })
            .collect();
        MapGrid::new(size, cells).unwrap()
    }

    /// Regression test for a real bug: `MatchSim::run_inner` read
    /// `state.day_steps()` **once, outside** the day loop, so every day ran
    /// for day 0's step count. With `daySteps = [16, 24]` day 1 executed
    /// only 16 of its 24 steps, and the solver's plan was truncated with it.
    ///
    /// The assertion is on observed agent movement rather than on the
    /// context: day 1 deliberately places its moves at step indices 16..=21,
    /// which only exist if that day really runs 24 steps.
    #[test]
    fn run_honours_per_day_step_counts() {
        let setting = MatchSetting {
            match_id: MatchId::new("per-day-steps").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map: empty_plain_map(),
            agents: vec![
                Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 100),
                Agent::supply(1, HexCoord::new(7, 7)),
                Agent::supply(2, HexCoord::new(7, 6)),
            ],
            day_steps: 16,
            num_days: 2,
            day_steps_by_day: vec![16, 24],
        };

        let recorder = RecordDaySteps { seen: std::sync::Mutex::new(Vec::new()) };

        let sim = MatchSim::default();
        let run = sim.run(setting, &recorder).unwrap();

        // The solver was told the per-day budget for each day it planned.
        assert_eq!(
            *recorder.seen.lock().unwrap(),
            vec![16, 24],
            "day 0 plans 16 steps, day 1 plans 24"
        );

        // Day 1 ran all 24 steps: twelve Right moves were scheduled at
        // indices 12..=23 and six of them (every other tick — a 平地 move
        // takes 2 steps, so the next order starts when the previous lands)
        // completed. Had day 1 been cut to 16 steps, only two moves would
        // have landed and the agent would sit at (2, 0).
        let agent = run.final_state.agents.iter().find(|a| a.id == 0).unwrap();
        assert_eq!(agent.pos, HexCoord::new(6, 0), "day 1 must run its full 24 steps");
        assert_eq!(agent.fuel, 100 - 6, "exactly six moves were applied");

        assert!(run.final_state.is_finished());
        assert_eq!(run.final_state.day, 2);
        assert_eq!(
            run.illegal_action_count, 0,
            "every scheduled move was legal, so this test is about step counts alone"
        );
    }

    /// Records the `day_steps` each day was planned with, and emits a plan
    /// whose meaningful actions sit *after* step 16.
    struct RecordDaySteps {
        seen: std::sync::Mutex<Vec<u32>>,
    }

    impl Solver for RecordDaySteps {
        fn name(&self) -> &'static str {
            "record-day-steps"
        }

        fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
            self.seen.lock().unwrap().push(ctx.day_steps);
            let mut actions =
                Vec::with_capacity(ctx.agent_ids.len().saturating_mul(ctx.day_steps as usize));
            for step in 0..ctx.day_steps {
                for id in &ctx.agent_ids {
                    // Day 1 only: wait for the first 12 steps, then walk
                    // agent 0 right for steps 12..=23. A 平地 move takes 2
                    // ticks, so every other order actually starts — six
                    // moves land within the 24-step day. The other two
                    // agents wait throughout so they never walk off the map.
                    let walk = ctx.day == 1 && *id == 0 && (12..=23).contains(&step);
                    actions.push(if walk {
                        Action::step(*id, Direction::Right)
                    } else {
                        Action::wait(*id)
                    });
                }
            }
            AgentActions::new(actions)
        }
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
        let wait = AgentActions::wait_all(&[0, 1, 2], 1);
        // Day 0 tick 0: reserve the move onto the spot (平地, lands step 2).
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
            1,
            "the stock is untouched until the move lands"
        );
        // Day 0 tick 1: still in flight.
        sim.step(&mut state, &wait);
        // Day 0 tick 2: the move lands and acquires — stock drops.
        sim.step(&mut state, &wait);
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
            0,
            "the landing step acquires the only ball"
        );
        // Day 1 tick 0: spawn fires at the day boundary's next entry and
        // refills stock to max=1. The agent is still on the spot but tick 0
        // is action-phase-only (Q7) — no re-pickup there.
        sim.step(&mut state, &wait);
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
            1,
            "spawn refilled the spot at the start of day 1"
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
