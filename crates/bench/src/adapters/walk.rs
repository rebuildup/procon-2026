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
//! could never collect from (Issue #235).
//!
//! # Walk modes (`walk=`)
//!
//! [`plan_walk`] is the `greedy` mode — the only one that existed before
//! Issue #215. The `astar` mode walks the **same** target list but in list
//! order with A*-planned legs; it lives in [`super::astar_walk`] because it
//! carries Issue #194's coordinate-conversion machinery. Candidates select
//! one or the other through [`plan_walk_with`] after parsing their `walk`
//! parameter with [`parse_walk`], so the assignment half of a tour candidate
//! is untouched by the choice.

use engine::model::{Action, AgentActions, AgentKind, Direction, HexCoord, MapGrid, SolverContext};

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
    /// Escort / rendezvous (following a Patrol to refuel it) is deliberately
    /// *not* what this mode does — waiting in place is the minimal "stop
    /// wasting the tour" switch the Issue asks for.
    Patrol,
}

impl AssignMode {
    /// Drop the target lists of Supply agents, in `ctx.agent_ids` order.
    ///
    /// Reads the kind through `ctx.state.agents[].kind`, which is the same
    /// visibility path every candidate has (Issue #235 Deliverable 2). When
    /// `self` is [`Self::Agents`] the vector is returned untouched, so the
    /// default path never re-allocates or re-orders anything.
    #[must_use]
    pub fn apply(
        self,
        ctx: &SolverContext<'_>,
        mut targets: Vec<Vec<HexCoord>>,
    ) -> Vec<Vec<HexCoord>> {
        if self == Self::Agents {
            return targets;
        }
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
/// `agents` / `patrol`.
pub(crate) fn parse_assign(spec: &CandidateSpec) -> Result<AssignMode, CandidateError> {
    match spec.param("assign") {
        None | Some("agents") => Ok(AssignMode::Agents),
        Some("patrol") => Ok(AssignMode::Patrol),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "assign".to_owned(),
            value: other.to_owned(),
            reason: "expected agents | patrol".to_owned(),
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

    /// Issue #235 — `assign` is a schema-gated value: omitted and `agents`
    /// select today's all-agents assignment, `patrol` withholds the tour from
    /// Supply agents, anything else is an `InvalidParameter` at parse time.
    #[test]
    fn assign_param_parses_agents_and_patrol_and_rejects_anything_else() {
        let parse = |id: &str| parse_assign(&CandidateSpec::parse(id).expect("id parses"));
        assert_eq!(parse("loop"), Ok(AssignMode::Agents));
        assert_eq!(parse("loop:assign=agents"), Ok(AssignMode::Agents));
        assert_eq!(parse("loop:assign=patrol"), Ok(AssignMode::Patrol));
        assert_eq!(
            parse("multiloop:kmeans=4:seed=42:walk=astar:assign=patrol"),
            Ok(AssignMode::Patrol)
        );
        assert_eq!(parse("loop"), Ok(AssignMode::default()), "the default is agents");
        for id in ["loop:assign=Patrol", "loop:assign=escort", "loop:assign=1", "loop:assign=tour"]
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

        let untouched = AssignMode::Agents.apply(&ctx, targets.clone());
        assert_eq!(untouched, targets, "assign=agents must be a no-op");

        let filtered = AssignMode::Patrol.apply(&ctx, targets);
        assert_eq!(
            filtered,
            vec![vec![HexCoord::new(2, 0)], Vec::new(), Vec::new()],
            "the patrol keeps its tour; both supply agents wait"
        );
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
