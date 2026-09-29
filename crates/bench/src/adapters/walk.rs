//! Shared helpers for turning a solver's *intent* (target spots / tours) into
//! concrete [`engine::model::AgentActions`] on the live map.
//!
//! Every bundled candidate adapter needs the same two primitives:
//!
//! - [`step_towards`] — one legal step from a cell toward a target cell.
//! - [`plan_walk`] — a whole day's step-major plan, walking each agent along
//!   its own list of targets.
//!
//! Keeping them here means a new candidate only has to decide *where* its
//! agents should go; the hex geometry and the engine's legality rules stay in
//! one place. The engine remains the final authority on what a move does:
//! [`engine::MatchSim`] still downgrades anything illegal to a `Wait`, so an
//! imperfect plan degrades instead of aborting the match.
//!
//! # Who gets a target (`assign=`)
//!
//! The same question, one level up: [`AssignMode`] decides *which* agents
//! receive a target list at all. `assign=agents` (default) is everyone, the
//! pre-#235 behaviour; `assign=patrol` withholds the tour from
//! [`AgentKind::Supply`] agents so they wait instead of walking a spot they
//! could never collect from (Issue #235); `assign=escort` hands each Supply
//! the projected day path of its paired Patrol so the two meet and refuel
//! mid-day (Issue #248).
//!
//! # Walk modes (`walk=`)
//!
//! [`plan_walk`] is the `greedy` mode — the only one that existed before
//! Issue #215. The `astar` mode walks the **same** target list but in list
//! order with A*-planned legs; it lives in [`super::astar_walk`] because it
//! carries Issue #194's coordinate-conversion machinery. Candidates select
//! one or the other through [`plan_walk_with`] after parsing their `walk`
//! parameter with [`parse_walk`]. The assignment half of a tour candidate is
//! otherwise untouched by the choice — except that [`AssignMode::Escort`]
//! projects the paired Patrol's path with the *same* walker (Issue #248), so
//! the chase tracks the path the Patrol will actually walk.

use engine::model::{
    Action, ActionKind, AgentActions, AgentKind, Direction, HexCoord, MapGrid, SolverContext,
};

use crate::candidate::{CandidateError, CandidateSpec};

/// Axial hex distance between two cells, matching the pointy-top convention
/// used across the solver crates: `(|dq| + |dq + dr| + |dr|) / 2`.
#[must_use]
pub fn hex_distance(a: HexCoord, b: HexCoord) -> u32 {
    let dq = i64::from(a.col - b.col).abs();
    let dr = i64::from(a.row - b.row).abs();
    let ds = (i64::from(a.col - b.col) + i64::from(a.row - b.row)).abs();
    u32::try_from((dq + dr + ds) / 2).unwrap_or(u32::MAX)
}

/// One legal step that gets closer to `to`, or `None` when there is nothing
/// useful to do: already there, `to` off the map, or no legal neighbour is
/// strictly closer.
///
/// This is a *greedy* single step, not a path plan — [`plan_walk`] calls it
/// once per step and lets the agent advance across steps. Terrain legality is
/// consulted for the immediate cell only; [`engine::MatchSim`] remains the
/// final authority and downgrades anything illegal to a `Wait`.
///
/// Direction candidates are scanned in [`Direction::ALL`] (wire-code order)
/// and the strictly-closest wins, with first-wins on ties. That makes the
/// choice deterministic — two runs of the same candidate on the same case pick
/// the same step.
#[must_use]
pub fn step_towards(from: HexCoord, to: HexCoord, map: &MapGrid) -> Option<Direction> {
    if from == to {
        return None;
    }
    map.get(to)?;
    let current = hex_distance(from, to);
    let mut best: Option<(Direction, u32)> = None;
    for dir in Direction::ALL {
        let next = dir.step(from);
        if !map.get(next).is_some_and(|c| c.terrain.traversable()) {
            continue;
        }
        let d = hex_distance(next, to);
        if d >= current {
            continue;
        }
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((dir, d)),
        }
    }
    best.map(|(dir, _)| dir)
}

/// Build one day's step-major plan: for every step, each controlled agent
/// takes one legal step toward the next entry of its target list.
///
/// `targets` is indexed by position in `ctx.agent_ids` — so `targets[i]`
/// belongs to `ctx.agent_ids[i]`. An empty target list makes that agent wait
/// the whole day. Targets cycle: `step % targets.len()`, so a short list is
/// repeated rather than running out.
///
/// The result is always well-shaped (`agent_count * day_steps` actions, one
/// per controlled agent per step, in `ctx.agent_ids` order), so
/// [`engine::MatchSim`] never has to reject it as malformed.
#[must_use]
pub fn plan_walk(ctx: &SolverContext<'_>, targets: &[Vec<HexCoord>]) -> AgentActions {
    let agent_count = ctx.agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    let mut actions = Vec::with_capacity(agent_count.saturating_mul(day_steps));

    // Live positions, indexed like `targets`.
    let mut positions: Vec<HexCoord> = ctx
        .agent_ids
        .iter()
        .map(|id| {
            ctx.state.agents.iter().find(|a| a.id == *id).map_or(HexCoord::new(0, 0), |a| a.pos)
        })
        .collect();

    for step in 0..day_steps {
        for (i, &agent_id) in ctx.agent_ids.iter().enumerate() {
            let Some(list) = targets.get(i).filter(|l| !l.is_empty()) else {
                actions.push(Action::wait(agent_id));
                continue;
            };
            let target = list[step % list.len()];
            match step_towards(positions[i], target, &ctx.state.setting.map) {
                Some(dir) => {
                    positions[i] = dir.step(positions[i]);
                    actions.push(Action::step(agent_id, dir));
                }
                None => actions.push(Action::wait(agent_id)),
            }
        }
    }
    AgentActions::new(actions)
}

/// How a candidate walks the per-agent target list it was given — the value
/// of the `walk=` parameter shared by the tour-shaped candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WalkMode {
    /// `greedy` (the default): [`plan_walk`] byte for byte — target chosen by
    /// `step % targets.len()`, one terrain-blind [`step_towards`] step.
    #[default]
    Greedy,
    /// `astar`: visit the list **in order**, one A* leg per target, then
    /// wait — Issue #215's anti-oscillation walk (see [`super::astar_walk`]).
    Astar,
}

/// Parse the `walk` parameter into a [`WalkMode`].
///
/// Omitted means [`WalkMode::Greedy`]: every pre-existing candidate id keeps
/// its exact behaviour, and `walk=greedy` is the explicit spelling of the
/// same default (Issue #215 appends the parameter last so existing canonical
/// ids keep their shape).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any value other than
/// `greedy` / `astar`.
pub(crate) fn parse_walk(spec: &CandidateSpec) -> Result<WalkMode, CandidateError> {
    match spec.param("walk") {
        None | Some("greedy") => Ok(WalkMode::Greedy),
        Some("astar") => Ok(WalkMode::Astar),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "walk".to_owned(),
            value: other.to_owned(),
            reason: "expected greedy | astar".to_owned(),
        }),
    }
}

/// Build one day's plan with the selected walk mode.
///
/// `WalkMode::Greedy` is exactly [`plan_walk`] (the dispatcher adds nothing
/// to it — pinned by `plan_walk_with_greedy_is_exactly_plan_walk`);
/// `WalkMode::Astar` is [`super::astar_walk::plan_walk_astar`]. Either way
/// the result is a well-shaped step-major plan.
#[must_use]
pub fn plan_walk_with(
    ctx: &SolverContext<'_>,
    targets: &[Vec<HexCoord>],
    mode: WalkMode,
) -> AgentActions {
    match mode {
        WalkMode::Greedy => plan_walk(ctx, targets),
        WalkMode::Astar => super::astar_walk::plan_walk_astar(ctx, targets),
    }
}

/// Which controlled agents a tour candidate hands target spots to (the
/// `assign=` parameter, Issue #235).
///
/// Every tour candidate built before #235 gave **all** of its agents a tour,
/// including the Supply tankers — which cannot collect 玉 at all (Issue
/// #234), so the tour a Supply agent walks is pure waste and, worse, it is
/// planned from an agent that could instead be following a Patrol to refuel
/// it. `assign` makes that choice explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssignMode {
    /// `assign=agents` (default): every controlled agent gets a target list —
    /// the pre-#235 behaviour, byte for byte, so existing canonical ids keep
    /// both their shape and their meaning.
    #[default]
    Agents,
    /// `assign=patrol`: [`AgentKind::Supply`] agents get an empty target list
    /// and therefore wait the whole day ([`plan_walk`]'s documented empty-list
    /// behaviour). Patrol agents are untouched.
    ///
    /// Waiting in place is the minimal "stop wasting the tour" switch (Issue
    /// #235); [`Self::Escort`] is the following variant that puts the Supply
    /// back on the road.
    Patrol,
    /// `assign=escort` (Issue #248): each [`AgentKind::Supply`] gets the
    /// **projected day path of its paired Patrol** as its target list, so the
    /// two walk the same cells and the engine's same-cell refill rule tops the
    /// Patrol up mid-day.
    ///
    /// Pairing is deterministic: the Patrol whose day-start cell is nearest to
    /// the Supply ([`hex_distance`]), ties broken by the smaller position in
    /// `ctx.agent_ids`. With no Patrol in the team every Supply gets an empty
    /// list and waits — escort has nobody to follow. Patrol target lists are
    /// never modified.
    ///
    /// The projection is [`day_paths`] over `walk`, i.e. exactly the path the
    /// paired Patrol will walk in the adapter's own `plan_walk_with` call
    /// (both walkers plan every agent independently, so phase 1 reproduces the
    /// Patrol's phase-2 actions). A Supply aiming at the Patrol's step-`s`
    /// position closes by one cell whenever the Patrol does not step directly
    /// away, so the two share a cell as soon as the Patrol turns or waits —
    /// and a Supply that starts with an empty Patrol tour parks on the
    /// Patrol's start cell instead.
    Escort,
}

impl AssignMode {
    /// Resolve the Supply agents' target lists for this mode, in
    /// `ctx.agent_ids` order.
    ///
    /// Reads the kind through `ctx.state.agents[].kind`, which is the same
    /// visibility path every candidate has (Issue #235 Deliverable 2). When
    /// `self` is [`Self::Agents`] the vector is returned untouched, so the
    /// default path never re-allocates or re-orders anything.
    ///
    /// `walk` selects the walker used to project the paired Patrol's path for
    /// [`Self::Escort`]; the other modes ignore it (the caller still walks the
    /// returned lists with the same `walk`, so the two stay consistent).
    #[must_use]
    pub fn apply(
        self,
        ctx: &SolverContext<'_>,
        mut targets: Vec<Vec<HexCoord>>,
        walk: WalkMode,
    ) -> Vec<Vec<HexCoord>> {
        match self {
            Self::Agents => targets,
            Self::Patrol => {
                for (index, id) in ctx.agent_ids.iter().enumerate() {
                    let is_supply =
                        ctx.state.agents.iter().any(|a| a.id == *id && a.kind == AgentKind::Supply);
                    if is_supply {
                        if let Some(list) = targets.get_mut(index) {
                            list.clear();
                        }
                    }
                }
                targets
            }
            Self::Escort => {
                let paths = day_paths(ctx, &targets, walk);
                let patrols: Vec<usize> = ctx
                    .agent_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| kind_of(ctx, **id) == Some(AgentKind::Patrol))
                    .map(|(index, _)| index)
                    .collect();
                for (index, id) in ctx.agent_ids.iter().enumerate() {
                    if kind_of(ctx, *id) != Some(AgentKind::Supply) {
                        continue;
                    }
                    let Some(list) = targets.get_mut(index) else {
                        continue;
                    };
                    let Some(supply_pos) = pos_of(ctx, *id) else {
                        continue;
                    };
                    let Some(paired) = patrols.iter().copied().min_by_key(|&patrol| {
                        let patrol_pos =
                            ctx.agent_ids.get(patrol).and_then(|pid| pos_of(ctx, *pid));
                        (patrol_pos.map_or(u32::MAX, |p| hex_distance(supply_pos, p)), patrol)
                    }) else {
                        // No Patrol to escort: wait, exactly like `assign=patrol`.
                        list.clear();
                        continue;
                    };
                    *list = paths.get(paired).cloned().unwrap_or_default();
                }
                targets
            }
        }
    }
}

/// Look up a controlled agent's kind by id through `ctx.state`.
fn kind_of(ctx: &SolverContext<'_>, id: u8) -> Option<AgentKind> {
    ctx.state.agents.iter().find(|a| a.id == id).map(|a| a.kind)
}

/// Look up a controlled agent's day-start cell by id through `ctx.state`.
fn pos_of(ctx: &SolverContext<'_>, id: u8) -> Option<HexCoord> {
    ctx.state.agents.iter().find(|a| a.id == id).map(|a| a.pos)
}

/// Every agent's projected day path: `paths[agent][step]` is the cell that
/// agent occupies after step `step` when walked with `walk` over `targets`.
///
/// Phase 1 of [`AssignMode::Escort`] — run the adapter's own walker once over
/// the unmodified target lists and reconstruct each agent's positions from its
/// action sequence (the same `Direction::step` fold [`plan_walk`] uses). Both
/// walkers plan every agent independently from its day-start cell, so these
/// paths equal the actions the adapter will emit for the Patrols in phase 2.
fn day_paths(
    ctx: &SolverContext<'_>,
    targets: &[Vec<HexCoord>],
    walk: WalkMode,
) -> Vec<Vec<HexCoord>> {
    let plan = plan_walk_with(ctx, targets, walk);
    let per_agent = ctx.agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    let mut positions: Vec<HexCoord> =
        ctx.agent_ids.iter().map(|id| pos_of(ctx, *id).unwrap_or(HexCoord::new(0, 0))).collect();
    let mut paths = vec![Vec::with_capacity(day_steps); per_agent];
    for step in 0..day_steps {
        for (index, _) in ctx.agent_ids.iter().enumerate() {
            let action = plan.actions.get(step * per_agent + index);
            if let Some(ActionKind::Move(dir)) = action.map(|a| a.kind) {
                if let Some(pos) = positions.get_mut(index) {
                    *pos = dir.step(*pos);
                }
            }
            if let Some(path) = paths.get_mut(index) {
                if let Some(pos) = positions.get(index) {
                    path.push(*pos);
                }
            }
        }
    }
    paths
}

/// Parse the `assign` parameter into an [`AssignMode`].
///
/// Omitted means [`AssignMode::Agents`]: every pre-#235 id keeps its exact
/// behaviour, and `assign=agents` is the explicit spelling of the same
/// default (the parameter is appended last in each factory's `PARAMS`, so
/// existing canonical ids keep their shape).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any value other than
/// `agents` / `patrol` / `escort`.
pub(crate) fn parse_assign(spec: &CandidateSpec) -> Result<AssignMode, CandidateError> {
    match spec.param("assign") {
        None | Some("agents") => Ok(AssignMode::Agents),
        Some("patrol") => Ok(AssignMode::Patrol),
        Some("escort") => Ok(AssignMode::Escort),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "assign".to_owned(),
            value: other.to_owned(),
            reason: "expected agents | patrol | escort".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{MatchId, PlayerId};
    use engine::model::{Agent, MapSize, MatchSetting, MatchState, Spot, Terrain};

    fn map_with_spots() -> MapGrid {
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..8 {
            for col in 0..8 {
                let spot = if (col, row) == (2, 0) {
                    Some(Spot {
                        series: 1,
                        pos: HexCoord::new(2, 0),
                        max_stock: 3,
                        current_stock: 3,
                    })
                } else {
                    None
                };
                cells.push(engine::model::Cell {
                    coord: HexCoord::new(col, row),
                    terrain: Terrain::Plain,
                    spot,
                });
            }
        }
        MapGrid::new(size, cells).unwrap()
    }

    fn state() -> MatchState {
        let map = map_with_spots();
        let mut agents = vec![Agent::patrol(0, HexCoord::new(0, 0), 4, 3)];
        while agents.len() < 3 {
            let id = u8::try_from(agents.len()).unwrap_or(u8::MAX);
            agents.push(Agent::supply(id, HexCoord::new(7, 7)));
        }
        MatchState::new(MatchSetting {
            match_id: MatchId::new("walk-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents,
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn step_towards_picks_the_legal_adjacent_direction() {
        let map = map_with_spots();
        // Adjacent target: one step reaches it.
        assert_eq!(
            step_towards(HexCoord::new(0, 0), HexCoord::new(1, 0), &map),
            Some(Direction::Right)
        );
        // Distant target: the step that gets closer, repeated across steps.
        assert_eq!(
            step_towards(HexCoord::new(0, 0), HexCoord::new(2, 0), &map),
            Some(Direction::Right)
        );
        // Backwards: moving away from the target is never chosen.
        assert_eq!(step_towards(HexCoord::new(2, 0), HexCoord::new(2, 0), &map), None);
    }

    #[test]
    fn step_towards_is_none_when_already_there() {
        let map = map_with_spots();
        assert_eq!(step_towards(HexCoord::new(2, 0), HexCoord::new(2, 0), &map), None);
    }

    #[test]
    fn plan_walk_is_well_shaped_and_moves_the_agent() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        // Agent 0 starts at (0, 0) and heads for the spot at (2, 0): two
        // Right steps get it there, then it holds position.
        let targets = vec![vec![HexCoord::new(2, 0)], vec![], vec![]];
        let plan = plan_walk(&ctx, &targets);
        assert_eq!(plan.actions.len(), 3 * 4, "one action per agent per step");
        assert!(
            plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
            "plan must be acceptable to MatchSim"
        );

        let agent0: Vec<_> = plan.actions.iter().step_by(3).map(|a| a.kind).collect();
        assert_eq!(agent0.len(), 4);
        assert!(
            matches!(agent0[0], engine::model::ActionKind::Move(Direction::Right))
                && matches!(agent0[1], engine::model::ActionKind::Move(Direction::Right)),
            "first two steps walk toward the target, got {agent0:?}"
        );
        assert!(
            matches!(agent0[2], engine::model::ActionKind::Wait)
                && matches!(agent0[3], engine::model::ActionKind::Wait),
            "once arrived the agent holds position, got {agent0:?}"
        );
    }

    /// Issue #215 — `walk` is a schema-gated value: omitted and `greedy`
    /// select today's [`plan_walk`], `astar` selects the ordered-tour walk,
    /// anything else is an `InvalidParameter` at parse / build time.
    #[test]
    fn walk_param_parses_greedy_and_astar_and_rejects_anything_else() {
        let parse = |id: &str| parse_walk(&CandidateSpec::parse(id).expect("id parses"));
        assert_eq!(parse("loop"), Ok(WalkMode::Greedy));
        assert_eq!(parse("loop:walk=greedy"), Ok(WalkMode::Greedy));
        assert_eq!(parse("loop:walk=astar"), Ok(WalkMode::Astar));
        assert_eq!(parse("multiloop:kmeans=4:seed=42:walk=astar"), Ok(WalkMode::Astar));
        assert_eq!(parse("loop"), Ok(WalkMode::default()), "the default mode is greedy");
        for id in ["loop:walk=Astar", "loop:walk=Greedy", "loop:walk=smart", "loop:walk=1"] {
            assert!(
                matches!(parse(id), Err(CandidateError::InvalidParameter { key, .. }) if key == "walk"),
                "{id} must be rejected as an invalid walk value"
            );
        }
    }

    /// `assign` is a schema-gated value: omitted and `agents` select the
    /// all-agents assignment, `patrol` withholds the tour from Supply agents
    /// (Issue #235), `escort` walks each Supply along its paired Patrol's
    /// projected path (Issue #248), anything else is an `InvalidParameter`.
    #[test]
    fn assign_param_parses_agents_patrol_escort_and_rejects_anything_else() {
        let parse = |id: &str| parse_assign(&CandidateSpec::parse(id).expect("id parses"));
        assert_eq!(parse("loop"), Ok(AssignMode::Agents));
        assert_eq!(parse("loop:assign=agents"), Ok(AssignMode::Agents));
        assert_eq!(parse("loop:assign=patrol"), Ok(AssignMode::Patrol));
        assert_eq!(parse("loop:assign=escort"), Ok(AssignMode::Escort));
        assert_eq!(
            parse("multiloop:kmeans=4:seed=42:walk=astar:assign=patrol"),
            Ok(AssignMode::Patrol)
        );
        assert_eq!(
            parse("multiloop:kmeans=4:seed=42:walk=astar:assign=escort"),
            Ok(AssignMode::Escort)
        );
        assert_eq!(parse("loop"), Ok(AssignMode::default()), "the default is agents");
        for id in ["loop:assign=Patrol", "loop:assign=Escort", "loop:assign=1", "loop:assign=tour"]
        {
            assert!(
                matches!(parse(id), Err(CandidateError::InvalidParameter { key, .. }) if key == "assign"),
                "{id} must be rejected as an invalid assign value"
            );
        }
    }

    /// Issue #235 AC — `assign=patrol` is the "Supply gets no spot tour"
    /// switch: it reads the kind through `SolverContext` and empties exactly
    /// those agents' target lists. `assign=agents` must not touch anything.
    #[test]
    fn assign_patrol_withholds_tours_from_supply_agents_only() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        // The fixture state is [Patrol, Supply, Supply].
        assert_eq!(ctx.state.agents[0].kind, AgentKind::Patrol);
        assert_eq!(ctx.state.agents[1].kind, AgentKind::Supply);
        assert_eq!(ctx.state.agents[2].kind, AgentKind::Supply);

        let targets =
            vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(3, 3)], vec![HexCoord::new(4, 4)]];

        let untouched = AssignMode::Agents.apply(&ctx, targets.clone(), WalkMode::Greedy);
        assert_eq!(untouched, targets, "assign=agents must be a no-op");

        let filtered = AssignMode::Patrol.apply(&ctx, targets, WalkMode::Greedy);
        assert_eq!(
            filtered,
            vec![vec![HexCoord::new(2, 0)], Vec::new(), Vec::new()],
            "the patrol keeps its tour; both supply agents wait"
        );
    }

    /// Issue #248 AC — pairing: each Supply receives the projected day path
    /// of the Patrol whose day-start cell is nearest to it (ties by smaller
    /// `agent_ids` index), while the Patrols' own lists stay untouched.
    #[test]
    fn assign_escort_pairs_each_supply_with_its_nearest_patrol() {
        let map = map_with_spots();
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("escort-pair").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::patrol(0, HexCoord::new(0, 0), 4, 3),
                Agent::patrol(1, HexCoord::new(0, 7), 4, 3),
                Agent::supply(2, HexCoord::new(1, 0)),
                Agent::supply(3, HexCoord::new(1, 6)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![
            vec![HexCoord::new(2, 0)],
            vec![HexCoord::new(5, 7)],
            vec![HexCoord::new(4, 4)],
            vec![HexCoord::new(3, 3)],
        ];
        // The pairing keys off hex distance: (1, 0) is next to patrol0 and
        // (1, 6) is next to patrol1 — the own-tour targets must not decide it.
        assert_eq!(hex_distance(HexCoord::new(1, 0), HexCoord::new(0, 0)), 1);
        assert_eq!(hex_distance(HexCoord::new(1, 0), HexCoord::new(0, 7)), 7);
        assert_eq!(hex_distance(HexCoord::new(1, 6), HexCoord::new(0, 7)), 1);
        assert_eq!(hex_distance(HexCoord::new(1, 6), HexCoord::new(0, 0)), 7);

        let paths = day_paths(&ctx, &targets, WalkMode::Greedy);
        assert_ne!(paths[0], paths[1], "the two patrols must walk different paths");

        let escorted = AssignMode::Escort.apply(&ctx, targets.clone(), WalkMode::Greedy);
        assert_eq!(escorted[0], targets[0], "patrol0's list is untouched");
        assert_eq!(escorted[1], targets[1], "patrol1's list is untouched");
        assert_eq!(
            escorted[2], paths[0],
            "supply2 pairs with the nearest patrol (patrol0) and walks its path"
        );
        assert_eq!(
            escorted[3], paths[1],
            "supply3 pairs with the nearest patrol (patrol1) and walks its path"
        );
        assert_ne!(escorted[2], targets[2], "the escort replaces the Supply's own tour");
    }

    /// Issue #248 — with no Patrol in the team there is nobody to escort, so
    /// every Supply waits: the same empty-list contract as `assign=patrol`.
    #[test]
    fn assign_escort_without_a_patrol_makes_supplies_wait() {
        let map = map_with_spots();
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("escort-solo").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::supply(0, HexCoord::new(0, 0)),
                Agent::supply(1, HexCoord::new(3, 3)),
                Agent::supply(2, HexCoord::new(7, 7)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(5, 5)], vec![]];
        let escorted = AssignMode::Escort.apply(&ctx, targets, WalkMode::Greedy);
        assert!(
            escorted.iter().all(Vec::is_empty),
            "no Patrol to follow, so every Supply waits, got {escorted:?}"
        );
    }

    /// Issue #248 AC — the engine end of the contract: walking with
    /// `assign=escort` puts the Supply on the paired Patrol's cell mid-day and
    /// the official same-cell refill rule fires (`Picked` with the
    /// `u32::MAX` sentinel, tank back to cap). The `assign=patrol` control
    /// never meets the Patrol, so it never refuels.
    #[test]
    fn assign_escort_meets_the_patrol_and_the_engine_refuels_it_mid_day() {
        let map = map_with_spots();
        let setting = MatchSetting {
            match_id: MatchId::new("escort-refuel").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                // Patrol walks (0,0) -> (1,0) -> (2,0) and parks on the spot.
                Agent::patrol(0, HexCoord::new(0, 0), 4, 3),
                // …and the Supply starts one cell below the first step, so
                // escorting it onto the projected path meets the Patrol at
                // step 0 — while its own tour would wander elsewhere.
                Agent::supply(1, HexCoord::new(1, 1)),
                // A second Supply too far away to arrive within the day: it
                // proves pairing is per-agent, not a global overwrite.
                Agent::supply(2, HexCoord::new(7, 7)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let state = MatchState::new(setting.clone()).unwrap();
        let ctx = SolverContext::from_state(&state);
        let targets =
            vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(4, 4)], vec![HexCoord::new(5, 5)]];

        let is_refuel = |e: &engine::transitions::StepEvent| matches!(e, engine::transitions::StepEvent::Picked { series, .. } if *series == u32::MAX);
        let run = |mode: AssignMode| -> (bool, u32) {
            let plan = plan_walk(&ctx, &mode.apply(&ctx, targets.clone(), WalkMode::Greedy));
            assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
            let mut run_state = MatchState::new(setting.clone()).unwrap();
            let sim = engine::MatchSim::default();
            let mut refuel_seen = false;
            for block in plan.actions.chunks(ctx.agent_ids.len()) {
                let events = sim.step(&mut run_state, &AgentActions::new(block.to_vec()));
                refuel_seen |= events.iter().any(is_refuel);
            }
            let patrol = run_state.agents.iter().find(|a| a.id == 0).expect("patrol exists");
            (refuel_seen, patrol.fuel)
        };

        let (escorted, fuel) = run(AssignMode::Escort);
        assert!(escorted, "escort must produce a same-cell refill event");
        // `Agent::patrol(_, _, 4, 3)` = day_steps × multiplier = 12.
        assert_eq!(fuel, 12, "the patrol ends the day at tank cap after the refill");

        let (waiting, _) = run(AssignMode::Patrol);
        assert!(!waiting, "assign=patrol must never meet the patrol, so no refill");
    }

    /// Issue #215 AC — default equivalence: selecting `greedy` explicitly is
    /// *exactly* [`plan_walk`], action for action, on a fixed input.
    #[test]
    fn plan_walk_with_greedy_is_exactly_plan_walk() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![vec![HexCoord::new(2, 0)], vec![], vec![]];
        assert_eq!(
            plan_walk_with(&ctx, &targets, WalkMode::Greedy),
            plan_walk(&ctx, &targets),
            "walk=greedy must be byte-for-byte the pre-#215 walk"
        );
        // And so is the omitted parameter, resolved through the mode default.
        assert_eq!(plan_walk_with(&ctx, &targets, WalkMode::default()), plan_walk(&ctx, &targets));
    }
}
