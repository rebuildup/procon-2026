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

use engine::model::{Action, AgentActions, Direction, HexCoord, MapGrid, SolverContext};

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
}
