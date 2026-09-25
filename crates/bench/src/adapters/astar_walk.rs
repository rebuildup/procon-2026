//! `walk=astar` — walk a tour-shaped candidate's target list **in order**
//! with A*-planned legs (Issue #215).
//!
//! # Why this exists
//!
//! The tour candidates (`loop`, `multiloop`) solve *where* each agent should
//! go and then hand the visit order to [`super::walk::plan_walk`], which
//! picks its target by **step index** (`step % targets.len()`) and takes one
//! greedy [`super::walk::step_towards`] step per turn. Two consequences were
//! measured as defect classes:
//!
//! * **oscillation** — an `(assignment × start geometry)` pair that never
//!   lets an agent *end* a step on any spot, so the day scores `(0, 0, 0)`
//!   (the Issue #207 root cause behind the Issue #213 baseline zeros);
//! * **path quality** — the greedy step is terrain-blind, so the same
//!   assignment routed by A* wins 47/25/1 against it (Issue #194).
//!
//! [`plan_walk_astar`] keeps the candidate's assignment byte-identical and
//! replaces only the walk: each agent visits **its own target list in list
//! order**, one A* leg per target, and waits once the list is exhausted.
//! Re-reading `step % len` is gone by construction, so an agent can never
//! alternate between two targets.
//!
//! # Day contract
//!
//! Same as [`super::walk::plan_walk`]: `decide` is called once at day start
//! with the full `ctx.day_steps` budget and must return a step-major,
//! `agent_count × day_steps` action list. Everything below replays one day
//! deterministically from the live start positions and fuel levels.
//!
//! # Leg semantics
//!
//! ```text
//! for each step of the day:
//!     while the agent already stands on targets[leg]: leg += 1   (arrival)
//!     list exhausted -> Wait for the rest of the day (the agent parks)
//!     current leg still has planned steps left -> follow them verbatim
//!     otherwise -> A* plan the leg from (pos, fuel, remaining steps)
//!                  on success: follow the new path
//!                  on failure: fall back to one `step_towards` step
//!                               (and Wait if that makes no progress)
//! ```
//!
//! * **Arrival ends a leg**: the step that lands on `targets[leg]` is the
//!   step the engine's acquire phase runs on, and the next step plans
//!   `targets[leg + 1]` — no dwell, no cycling back.
//! * **Fuel**: a leg is planned with the fuel the agent will actually have
//!   at the leg's start and [`CostPolicy::default()`] (the `astar`
//!   candidate's `policy=step`: weighted step cost minimisation with fuel as
//!   a hard constraint). Every planned step is re-checked against
//!   [`engine::rules::fuel_cost`] before it is emitted, and so is every
//!   fallback step, so this walk cannot emit an unaffordable move at all.
//! * **Failure fallback** (documented safety, mirroring
//!   [`super::astar_solver`]'s degrade-to-greedy): if A* returns no path for
//!   a leg — step budget exceeded, unreachable, out of fuel, endpoints off
//!   the walkable grid, or the engine replay check rejecting the conversion —
//!   that leg walks one greedy [`super::walk::step_towards`] step at a time
//!   instead. `step_towards` never moves away from the goal, so even the
//!   fallback cannot oscillate; when it returns `None` (no strictly-closer
//!   legal neighbour) the agent simply waits.
//!
//! # Coordinate conversion — the load-bearing risk (shared with `astar_solver`)
//!
//! `engine::model::Direction::step` and `solver_astar::hex::Direction::step`
//! document the same even-r layout but implement opposite row-parity
//! branches for the four diagonal directions, so the two neighbour graphs are
//! column mirror images. [`to_astar_map`] therefore plans on a
//! **column-mirrored copy** of the map and [`to_engine_dir`] mirrors the step
//! names back at the boundary — the machinery Issue #194 shipped, extracted
//! here so [`super::astar_solver`] and this walk share exactly one
//! implementation. Mirroring is an involution on `[0, width)`, so bounds,
//! terrain and fuel costs are preserved. Every planned leg is re-played
//! against the engine's own move rules ([`plan_is_executable`]) before it is
//! emitted; a conversion regression degrades to the documented fallback
//! instead of illegal actions.

use std::collections::VecDeque;

use engine::model::{
    Action, ActionKind, Agent, AgentActions, Direction, HexCoord, MapGrid, SolverContext, Terrain,
    TrafficState,
};
use engine::rules::fuel_cost as engine_fuel_cost;
use solver_astar::{
    Action as AstarAction, AstarSolver, CellKind, CostPolicy, Direction as AstarDirection,
    HexCoord as AstarCoord, HexMap, SolveContext, Solver as AstarSolve,
    TrafficState as AstarTraffic,
};

use super::walk::step_towards;

/// Column mirror between the engine's grid and the planning grid.
///
/// `(col, row) → (width - 1 - col, row)`. An involution: applying it twice
/// is the identity, and it maps `[0, width) × [0, height)` onto itself.
pub(crate) const fn mirror_coord(coord: HexCoord, width: i32) -> HexCoord {
    HexCoord::new(width - 1 - coord.col, coord.row)
}

/// Engine direction for one mirrored A* step.
///
/// Left-right mirrored name table (`East ↔ West`,
/// `NorthEast ↔ UpperLeft`, `NorthWest ↔ UpperRight`,
/// `SouthEast ↔ LowerLeft`, `SouthWest ↔ LowerRight`) so that stepping with
/// the returned direction on the engine grid lands on the mirrored image of
/// the A* step's destination. Pinned exhaustively by
/// `astar_solver::mirrored_astar_steps_match_engine_steps`.
pub(crate) const fn to_engine_dir(dir: AstarDirection) -> Direction {
    match dir {
        AstarDirection::East => Direction::Left,
        AstarDirection::West => Direction::Right,
        AstarDirection::NorthEast => Direction::UpperLeft,
        AstarDirection::NorthWest => Direction::UpperRight,
        AstarDirection::SouthEast => Direction::LowerLeft,
        AstarDirection::SouthWest => Direction::LowerRight,
    }
}

/// Map an engine terrain onto the A* crate's cell kind.
pub(crate) const fn to_cell_kind(terrain: Terrain) -> CellKind {
    match terrain {
        Terrain::Plain => CellKind::Plain,
        Terrain::Mountain => CellKind::Mountain,
        Terrain::Pond => CellKind::Pond,
        Terrain::Road(state) => CellKind::Road { traffic: to_traffic(state) },
    }
}

const fn to_traffic(state: TrafficState) -> AstarTraffic {
    match state {
        TrafficState::Smooth => AstarTraffic::Smooth,
        TrafficState::Busy => AstarTraffic::Busy,
        TrafficState::Jammed => AstarTraffic::Jammed,
    }
}

/// Convert the live engine map into the A* crate's map on the mirrored
/// grid: A* cell `(col, row)` holds the terrain of engine cell
/// `(width - 1 - col, row)`.
///
/// Returns `None` only if the sizes somehow violate the A* crate's
/// `8..=32` contract (the engine validates the same range, so this cannot
/// happen for a constructed [`MapGrid`]).
pub(crate) fn to_astar_map(map: &MapGrid) -> Option<HexMap> {
    let width = u32::from(map.size.width);
    let height = u32::from(map.size.height);
    let width_usize = usize::try_from(width).ok()?;
    let cell_count = width_usize.checked_mul(usize::try_from(height).ok()?)?;
    let mut cells = vec![CellKind::Plain; cell_count];
    for cell in map.cells() {
        let mirrored = mirror_coord(cell.coord, i32::from(map.size.width));
        let col = usize::try_from(mirrored.col).ok()?;
        let row = usize::try_from(mirrored.row).ok()?;
        *cells.get_mut(row.checked_mul(width_usize)?.checked_add(col)?)? =
            to_cell_kind(cell.terrain);
    }
    HexMap::new(width, height, cells).ok()
}

/// Replay a planned action list against the engine's move rules, starting
/// from `start` with `fuel` remaining: every move must stay in bounds, land
/// on traversable terrain, and be affordable at the fuel level the agent
/// will actually have at that point.
///
/// This is the tripwire for the coordinate-conversion risk: it uses
/// [`engine::model::Direction::step`] and [`engine::rules::fuel_cost`],
/// i.e. the same functions [`engine::MatchSim`] resolves moves with.
pub(crate) fn plan_is_executable(
    start: HexCoord,
    fuel: u32,
    plan: &[ActionKind],
    map: &MapGrid,
) -> bool {
    let mut pos = start;
    let mut fuel = fuel;
    for kind in plan {
        let ActionKind::Move(dir) = *kind else {
            continue;
        };
        let next = dir.step(pos);
        let Some(cell) = map.get(next) else {
            return false;
        };
        if !cell.terrain.traversable() {
            return false;
        }
        let Some(remaining) = fuel.checked_sub(engine_fuel_cost(cell.terrain)) else {
            return false;
        };
        fuel = remaining;
        pos = next;
    }
    true
}

/// Plan one **leg** of an ordered tour: an A* path from `start` (engine
/// coordinates, `fuel` units left) to `goal` costing at most `max_steps`,
/// converted back to engine directions and replay-validated.
///
/// `width` is the engine map's width — the mirror parameter shared by
/// [`mirror_coord`] and [`to_astar_map`].
///
/// Returns the leg's move sequence **without padding** — the caller consumes
/// exactly as many steps as the day has left. `None` means "no usable path"
/// and covers every failure the `astar` adapter classifies: the endpoints
/// being rejected by [`SolveContext::new`] (off-map / pond),
/// [`AstarSolver`] finding no path within the step budget or the fuel budget
/// (`Unreachable` / `ExceedsStepBudget` / `OutOfFuel`), an empty path, or
/// the engine replay check rejecting the converted steps.
pub(crate) fn plan_leg_path(
    astar_map: &HexMap,
    engine_map: &MapGrid,
    start: HexCoord,
    fuel: u32,
    goal: HexCoord,
    max_steps: u32,
    policy: CostPolicy,
) -> Option<Vec<ActionKind>> {
    let width = i32::from(engine_map.size.width);
    let mirrored_start = mirror_coord(start, width);
    let mirrored_goal = mirror_coord(goal, width);
    let solve_ctx = SolveContext::new(
        astar_map.clone(),
        AstarCoord::new(mirrored_start.col, mirrored_start.row),
        AstarCoord::new(mirrored_goal.col, mirrored_goal.row),
        fuel,
        max_steps,
    )?;
    let solver = AstarSolver::with_policy(policy);
    let result = solver.solve(&solve_ctx);
    let path = result.path?;
    if path.steps.is_empty() {
        // `start == goal` degenerate: nothing to walk, and the arrival rule
        // is the caller's job — reported as "no usable path".
        return None;
    }

    let max_steps = usize::try_from(max_steps).unwrap_or(0);
    let mut plan: Vec<ActionKind> = path
        .steps
        .iter()
        .map(|step| match step {
            AstarAction::Move(dir) => ActionKind::Move(to_engine_dir(*dir)),
            AstarAction::Wait => ActionKind::Wait,
        })
        .collect();
    // Defensive: A* only emits paths whose weighted cost fits `max_steps`
    // (and every step weighs ≥ 1), but never trust that on the way out.
    plan.truncate(max_steps);

    if !plan_is_executable(start, fuel, &plan, engine_map) {
        tracing::debug!(
            start = ?start,
            goal = ?goal,
            "astar leg failed the engine replay check; falling back to a greedy step"
        );
        return None;
    }
    Some(plan)
}

/// Plan one whole day for one agent: visit `targets` **in order**, one A*
/// leg per target, then wait.
///
/// `agent == None` or an empty `targets` list yields an all-wait day (the
/// same contract [`super::walk::plan_walk`] documents). The returned vector
/// always has exactly `day_steps` entries and never contains a move the
/// engine would reject.
fn plan_agent_day(
    engine_map: &MapGrid,
    astar_map: Option<&HexMap>,
    agent: Option<&Agent>,
    targets: &[HexCoord],
    day_steps: usize,
) -> Vec<ActionKind> {
    let Some(agent) = agent else {
        return vec![ActionKind::Wait; day_steps];
    };

    let mut plan: Vec<ActionKind> = Vec::with_capacity(day_steps);
    let mut pos = agent.pos;
    let mut fuel = agent.fuel;
    // Index of the target this agent is currently walking to. Monotonically
    // non-decreasing — that is the no-oscillation invariant.
    let mut leg: usize = 0;
    // Remaining planned steps of the current leg's A* path.
    let mut leg_path: VecDeque<ActionKind> = VecDeque::new();
    // Set once A* has failed for the current leg: that leg degrades to
    // `step_towards` for the rest of the day (see the module docs), and is
    // reset on arrival so a later leg gets a fresh plan.
    let mut leg_fallback = false;

    for step in 0..day_steps {
        // Arrival (also covers an agent that starts on its first target, and
        // consecutive duplicate targets): the leg is done, plan the next one
        // from here — inside this very step, so no step is wasted.
        while leg < targets.len() && pos == targets[leg] {
            leg += 1;
            leg_path.clear();
            leg_fallback = false;
        }

        let kind = loop {
            if leg >= targets.len() {
                break ActionKind::Wait;
            }
            if let Some(next) = leg_path.pop_front() {
                break next;
            }
            let goal = targets[leg];
            if !leg_fallback {
                let remaining = u32::try_from(day_steps - step).unwrap_or(u32::MAX);
                let planned = astar_map.and_then(|map| {
                    plan_leg_path(
                        map,
                        engine_map,
                        pos,
                        fuel,
                        goal,
                        remaining,
                        CostPolicy::default(),
                    )
                });
                match planned {
                    Some(path) if !path.is_empty() => {
                        leg_path.extend(path);
                        continue; // re-enter and take the first planned step
                    }
                    _ => {
                        tracing::debug!(
                            agent = agent.id,
                            goal = ?goal,
                            remaining,
                            "walk=astar leg has no path; degrading it to step_towards"
                        );
                        leg_fallback = true;
                    }
                }
            }
            // Documented fallback: one greedy step toward this very target.
            // `step_towards` returns `None` when no strictly-closer legal
            // neighbour exists, which becomes a Wait — no progress, no
            // oscillation.
            break step_towards(pos, goal, engine_map).map_or(ActionKind::Wait, ActionKind::Move);
        };

        // Apply the chosen action to the tracked (pos, fuel). Anything the
        // engine would downgrade — out of bounds, impassable, unaffordable —
        // is emitted as a Wait instead, so this walk can never produce an
        // `illegal` / `fuel_short` event.
        match kind {
            ActionKind::Move(dir) => {
                let next = dir.step(pos);
                let cost = engine_map
                    .get(next)
                    .filter(|cell| cell.terrain.traversable())
                    .map(|cell| engine_fuel_cost(cell.terrain));
                match cost {
                    Some(cost) if fuel >= cost => {
                        fuel -= cost;
                        pos = next;
                        plan.push(ActionKind::Move(dir));
                    }
                    _ => {
                        // The planned path (or the greedy fallback) disagrees
                        // with the engine: drop the rest of the leg's plan and
                        // re-plan from here next step.
                        leg_path.clear();
                        leg_fallback = true;
                        plan.push(ActionKind::Wait);
                    }
                }
            }
            ActionKind::Wait => plan.push(ActionKind::Wait),
        }
    }
    plan
}

/// Build one day's step-major plan for `walk=astar`: every controlled agent
/// walks its own target list **in order**, one A* leg per target, and waits
/// once the list (or the day) is exhausted.
///
/// `targets` is indexed by position in `ctx.agent_ids` — `targets[i]`
/// belongs to `ctx.agent_ids[i]` — exactly like [`super::walk::plan_walk`],
/// so the two walks are drop-in substitutes for one another. An empty (or
/// missing) target list makes that agent wait the whole day.
///
/// The result is always well-shaped (`agent_count * day_steps` actions, one
/// per controlled agent per step, in `ctx.agent_ids` order).
#[must_use]
pub fn plan_walk_astar(ctx: &SolverContext<'_>, targets: &[Vec<HexCoord>]) -> AgentActions {
    let map = &ctx.state.setting.map;
    let day_steps = usize::try_from(ctx.day_steps).unwrap_or(0);
    let astar_map = to_astar_map(map);

    let per_agent: Vec<Vec<ActionKind>> = ctx
        .agent_ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let agent = ctx.state.agents.iter().find(|a| a.id == *id);
            let list = targets.get(i).map_or(&[][..], |list| list.as_slice());
            plan_agent_day(map, astar_map.as_ref(), agent, list, day_steps)
        })
        .collect();

    // Step-major interleave, matching `plan_walk`'s layout: for each step,
    // one action per controlled agent in `ctx.agent_ids` order.
    let mut actions = Vec::with_capacity(ctx.agent_ids.len().saturating_mul(day_steps));
    for step in 0..day_steps {
        for (i, id) in ctx.agent_ids.iter().enumerate() {
            let kind = per_agent
                .get(i)
                .and_then(|plan| plan.get(step))
                .copied()
                .unwrap_or(ActionKind::Wait);
            actions.push(Action { agent_id: *id, kind });
        }
    }
    AgentActions::new(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::MatchSim;
    use engine::model::{Agent, Cell, MapSize, MatchSetting, MatchState, Spot};

    use crate::adapters::walk::plan_walk;
    use domain::{MatchId, PlayerId};

    /// Two idle supply agents: a match team needs `3..=8` agents and no
    /// agent may start on a spot, so every test here runs one "actor" plus
    /// these two bystanders (they get empty target lists and wait).
    const IDLE_1: HexCoord = HexCoord::new(6, 7);
    const IDLE_2: HexCoord = HexCoord::new(7, 7);

    /// An 8×8 plain map carrying `spots`, one patrol "actor" at
    /// `actor_pos`, and the two idle supply agents.
    ///
    /// `actor_pos` must not be a spot cell — the engine rejects that start.
    fn setting(spots: &[HexCoord], actor_pos: HexCoord, day_steps: u32) -> MatchSetting {
        assert!(!spots.contains(&actor_pos), "the actor may not start on a spot");
        assert!(
            !spots.contains(&IDLE_1) && !spots.contains(&IDLE_2),
            "idles may not start on a spot"
        );
        let size = MapSize::new(8, 8).expect("8x8 is in range");
        let mut cells = Vec::with_capacity(64);
        for row in 0..8 {
            for col in 0..8 {
                let coord = HexCoord::new(col, row);
                let series = spots.iter().position(|s| *s == coord);
                let spot = series.map(|idx| Spot {
                    series: u32::try_from(idx + 1).unwrap_or(u32::MAX),
                    pos: coord,
                    max_stock: 8,
                    current_stock: 8,
                });
                cells.push(Cell { coord, terrain: Terrain::Plain, spot });
            }
        }
        MatchSetting {
            match_id: MatchId::new("walk-astar").expect("match id"),
            team: PlayerId::new("player-a").expect("team id"),
            map: MapGrid::new(size, cells).expect("valid map"),
            agents: vec![
                Agent::patrol_with_fuel(0, actor_pos, 16),
                Agent::supply(1, IDLE_1),
                Agent::supply(2, IDLE_2),
            ],
            day_steps,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        }
    }

    /// One agent's actions from a step-major plan (`agent_count` actions per
    /// step, `ctx.agent_ids` order — so slot `s` is every `s`-th action).
    fn agent_kinds(plan: &AgentActions, agent_count: usize, slot: usize) -> Vec<ActionKind> {
        assert!(agent_count > 0, "a team always has agents");
        assert_eq!(plan.actions.len() % agent_count, 0, "the plan is a whole number of steps");
        plan.actions
            .iter()
            .enumerate()
            .filter(|(i, _)| i % agent_count == slot)
            .map(|(_, action)| action.kind)
            .collect()
    }

    /// The positions one agent occupies after each of its steps.
    fn positions(
        plan: &AgentActions,
        start: HexCoord,
        agent_count: usize,
        slot: usize,
    ) -> Vec<HexCoord> {
        let mut pos = start;
        agent_kinds(plan, agent_count, slot)
            .into_iter()
            .map(|kind| {
                if let ActionKind::Move(dir) = kind {
                    pos = dir.step(pos);
                }
                pos
            })
            .collect()
    }

    /// Issue #207's defect class in miniature: `plan_walk` picks its target
    /// by `step % len`, so an agent whose two targets straddle its start
    /// position ping-pongs forever and never ends a step on a spot.
    /// `plan_walk_astar` walks the same list in order and reaches both.
    #[test]
    fn oscillating_input_is_reached_by_walk_astar_and_missed_by_plan_walk() {
        let start = HexCoord::new(2, 0);
        let first = HexCoord::new(4, 0);
        let second = HexCoord::new(0, 0);
        let state = MatchState::new(setting(&[second, first], start, 8)).expect("valid state");
        let ctx = SolverContext::from_state(&state);
        // Listed so the greedy walk alternates *away* from each target on
        // every other step: from the midpoint (2, 0) step 0 heads to (4, 0),
        // step 1 back to (0, 0) — landing on neither.
        let targets = vec![vec![first, second]];

        let greedy = plan_walk(&ctx, &targets);
        let greedy_positions = positions(&greedy, start, 3, 0);
        assert!(
            greedy_positions.iter().all(|p| *p != first && *p != second),
            "the repro must oscillate without touching a spot: {greedy_positions:?}"
        );

        let astar = plan_walk_astar(&ctx, &targets);
        assert!(astar.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");
        let astar_positions = positions(&astar, start, 3, 0);
        // In list order: (4, 0) first, then (0, 0) — and then the agent
        // parks on the last target instead of cycling back.
        let (Some(hit_first), Some(hit_second)) = (
            astar_positions.iter().position(|p| *p == first),
            astar_positions.iter().position(|p| *p == second),
        ) else {
            panic!("walk=astar must reach both targets in order: {astar_positions:?}");
        };
        assert!(hit_first < hit_second, "targets are visited in list order: {astar_positions:?}");
        assert_eq!(
            astar_positions.last(),
            Some(&second),
            "the agent parks on the final target instead of cycling: {astar_positions:?}"
        );
        // Standing *on* the target for a whole step is what makes the engine
        // acquire — a graze on the way through would not count.
        assert!(
            astar_positions.iter().filter(|p| **p == first).count() >= 1,
            "the arrival step stands on the first spot: {astar_positions:?}"
        );
    }

    /// Arrival ends the leg and the *next step* starts the next one: the
    /// exact trace of a two-target walk on plain ground.
    #[test]
    fn arrival_advances_to_the_next_leg_without_losing_a_step() {
        let start = HexCoord::new(0, 0);
        let state = MatchState::new(setting(&[HexCoord::new(2, 0), HexCoord::new(5, 0)], start, 8))
            .expect("valid state");
        let ctx = SolverContext::from_state(&state);
        let targets = vec![vec![HexCoord::new(2, 0), HexCoord::new(5, 0)]];

        let plan = plan_walk_astar(&ctx, &targets);
        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");
        assert_eq!(
            positions(&plan, start, 3, 0),
            vec![
                HexCoord::new(1, 0),
                HexCoord::new(2, 0), // leg 0 arrives …
                HexCoord::new(3, 0), // … and leg 1 starts on the very next step
                HexCoord::new(4, 0),
                HexCoord::new(5, 0), // leg 1 arrives, list exhausted …
                HexCoord::new(5, 0), // … so the agent parks for the rest of the day
                HexCoord::new(5, 0),
                HexCoord::new(5, 0),
            ],
            "ordered legs: straight out, then straight on, then wait"
        );
    }

    /// Budget failure (Issue AC (c)): the leg cannot be planned inside the
    /// remaining day, so it degrades to `step_towards` — and the emitted
    /// action is still in bounds, traversable and affordable, which the
    /// engine itself confirms.
    #[test]
    fn budget_failure_falls_back_to_a_legal_step_towards_step() {
        let start = HexCoord::new(5, 0);
        // One step of budget for a target three cells away.
        let setting = setting(&[HexCoord::new(2, 0)], start, 1);
        let state = MatchState::new(setting.clone()).expect("valid state");
        let ctx = SolverContext::from_state(&state);
        let targets = vec![vec![HexCoord::new(2, 0)]];

        let plan = plan_walk_astar(&ctx, &targets);
        assert_eq!(
            agent_kinds(&plan, 3, 0),
            vec![ActionKind::Move(Direction::Left)],
            "the fallback takes the single greedy step toward (2, 0)"
        );
        assert!(
            plan.actions.iter().skip(1).all(|a| matches!(a.kind, ActionKind::Wait)),
            "the bystanders wait all day"
        );

        // The engine agrees: the plan applies cleanly, with no downgrades.
        let mut run_state = MatchState::new(setting).expect("valid state");
        let sim = MatchSim::default();
        sim.step(&mut run_state, &plan);
        assert_eq!(run_state.illegal_action_count, 0, "no illegal actions");
        assert_eq!(run_state.fuel_short_count, 0, "no fuel shortages");
        assert_eq!(run_state.agents[0].pos, HexCoord::new(4, 0), "the step landed");
    }

    /// Out-of-fuel fallback: A* refuses the leg, `step_towards` still wants
    /// to move, but the move costs more than the tank holds — so the walk
    /// emits a Wait rather than an unaffordable move (the `fuel_short = 0`
    /// invariant) and never retries anything oscillatory.
    #[test]
    fn fuel_failure_falls_back_to_waiting_instead_of_spending_it() {
        let start = HexCoord::new(4, 4);
        let goal = HexCoord::new(1, 4);
        let size = MapSize::new(8, 8).expect("8x8 is in range");
        let mut cells = Vec::with_capacity(64);
        for row in 0..8 {
            for col in 0..8 {
                let coord = HexCoord::new(col, row);
                // The only strictly-closer neighbour of (4, 4) toward (1, 4)
                // is (3, 4): make it a mountain (2 fuel) so the greedy
                // fallback step is unaffordable at 1 fuel.
                let terrain = if (col, row) == (3, 4) { Terrain::Mountain } else { Terrain::Plain };
                let spot = if (col, row) == (0, 0) {
                    Some(Spot { series: 1, pos: coord, max_stock: 4, current_stock: 4 })
                } else {
                    None
                };
                cells.push(Cell { coord, terrain, spot });
            }
        }
        let setting = MatchSetting {
            match_id: MatchId::new("walk-astar-fuel").expect("match id"),
            team: PlayerId::new("player-a").expect("team id"),
            map: MapGrid::new(size, cells).expect("valid map"),
            agents: vec![
                Agent::patrol_with_fuel(0, start, 1),
                Agent::supply(1, IDLE_1),
                Agent::supply(2, IDLE_2),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let state = MatchState::new(setting).expect("valid state");
        let ctx = SolverContext::from_state(&state);
        let plan = plan_walk_astar(&ctx, &[vec![goal]]);

        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");
        assert!(
            agent_kinds(&plan, 3, 0).iter().all(|k| matches!(k, ActionKind::Wait)),
            "an agent that cannot pay for its greedy step waits: {:?}",
            agent_kinds(&plan, 3, 0)
        );
    }

    /// Empty / missing target lists are all-wait, exactly like `plan_walk`.
    #[test]
    fn empty_targets_wait_the_whole_day() {
        let state = MatchState::new(setting(&[HexCoord::new(4, 0)], HexCoord::new(2, 0), 4))
            .expect("valid state");
        let ctx = SolverContext::from_state(&state);
        for targets in [vec![vec![]], vec![Vec::new(), Vec::new()]] {
            let plan = plan_walk_astar(&ctx, &targets);
            assert_eq!(plan.actions.len(), ctx.agent_ids.len() * ctx.day_steps as usize);
            assert!(plan.actions.iter().all(|a| matches!(a.kind, ActionKind::Wait)));
        }
    }

    /// Well-shapedness plus the engine's own legality verdict on a full day
    /// over an obstacle map: the walk routes around the pond onto the spot
    /// and never emits an illegal or unaffordable action.
    #[test]
    fn full_day_plan_is_legal_and_affordable_over_ponds() {
        let start = HexCoord::new(0, 0);
        let goal = HexCoord::new(7, 0);
        let size = MapSize::new(8, 8).expect("8x8 is in range");
        let mut cells = Vec::with_capacity(64);
        for row in 0..8 {
            for col in 0..8 {
                let coord = HexCoord::new(col, row);
                let terrain =
                    if row == 0 && (1..=6).contains(&col) { Terrain::Pond } else { Terrain::Plain };
                let spot = if (col, row) == (7, 0) {
                    Some(Spot { series: 1, pos: coord, max_stock: 4, current_stock: 4 })
                } else {
                    None
                };
                cells.push(Cell { coord, terrain, spot });
            }
        }
        let setting = MatchSetting {
            match_id: MatchId::new("walk-astar-pond").expect("match id"),
            team: PlayerId::new("player-a").expect("team id"),
            map: MapGrid::new(size, cells).expect("valid map"),
            agents: vec![
                Agent::patrol_with_fuel(0, start, 16),
                Agent::supply(1, IDLE_1),
                Agent::supply(2, IDLE_2),
            ],
            day_steps: 12,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let state = MatchState::new(setting).expect("valid state");
        let ctx = SolverContext::from_state(&state);
        let plan = plan_walk_astar(&ctx, &[vec![goal]]);
        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");

        let mut pos = start;
        let mut fuel = state.agents[0].fuel;
        for kind in agent_kinds(&plan, 3, 0) {
            let ActionKind::Move(dir) = kind else { continue };
            let next = dir.step(pos);
            let cell = state.setting.map.get(next).expect("move stays in bounds");
            assert!(cell.terrain.traversable(), "move lands on traversable terrain");
            let cost = engine_fuel_cost(cell.terrain);
            assert!(fuel >= cost, "move is affordable");
            fuel -= cost;
            pos = next;
        }
        assert_eq!(pos, goal, "the agent routes around the pond onto the spot");
    }
}
