//! `astar` candidate — terrain / traffic cost aware routing from
//! `crates/solver-astar` (Issue #194).
//!
//! A routing-level mechanism, deliberately distinct from every assignment /
//! tour candidate shipped so far:
//!
//! - `greedy` and `loop` decide *where* each agent should go, then walk with
//!   [`super::walk::step_towards`] — a nearest-unit, terrain-blind greedy
//!   step recomputed every step.
//! - `astar` picks the same target spots as `greedy` (see below) but lets
//!   `solver-astar`'s single-goal A* plan the actual *path* under terrain /
//!   traffic weighted step cost and a hard fuel constraint, then follows that
//!   planned path step by step.
//!
//! # Day contract
//!
//! Same as [`super::greedy_solver::GreedyCandidate`]: assignment and path are
//! solved **once at day start** from the live [`engine::model::MatchState`]
//! and expanded into one day of step-major actions. Because both candidates
//! share the assignment, any scoring difference between `astar` and `greedy`
//! isolates *routing* quality — exactly the hypothesis in Issue #194.
//!
//! # Goal selection (v1)
//!
//! The target spot per agent comes from `solver-greedy`'s variety-aware
//! nearest assignment (unvisited series preferred) — the same code path
//! `greedy` runs. A* only answers *"how do I get there?"*.
//!
//! # Path following (v1)
//!
//! The A* [`solver_astar::Path`] is expanded into that agent's step-major
//! actions: planned steps are followed verbatim, then the agent waits once it
//! arrives (single target per day). When A* returns no path (goal over the
//! step budget, unreachable under fuel, pond-blocked) **or** the planned path
//! fails the engine-side replay check, the agent falls back to
//! [`super::walk::step_towards`] — i.e. exactly `greedy`'s behaviour — so
//! `astar` degrades to `greedy` instead of to nothing.
//!
//! Known limitation (future `multi_agent` child candidate): paths are planned
//! **per agent with no reservation** of the other agents. The engine lets
//! agents share cells, so overlaps resolve harmlessly; the cost is that the
//! planner cannot yet reason about traffic interactions between our own
//! agents.
//!
//! # Coordinate conversion — the load-bearing risk (shared with `walk=astar`)
//!
//! `engine::model::Direction::step` and `solver_astar::hex::Direction::step`
//! both *document* the contest's even-r layout (Q&A Q1) but implement
//! **opposite row-parity branches** for the four diagonal directions:
//!
//! ```text
//! upper neighbours of (col, row) on an even row
//!   engine      → { col, col + 1 }
//!   solver-astar → { col - 1, col }
//! ```
//!
//! The two neighbour graphs are exact column mirror images of each other, so
//! the planner works on a **column-mirrored copy** of the map
//! (`(col, row) → (width - 1 - col, row)`) and mirrors coordinates and step
//! directions at the boundary. Mirroring is an involution on `[0, width)`, so
//! bounds, terrain and fuel costs are preserved, and A* stays optimal on the
//! engine's grid (a reflection is an isometry of the hex lattice).
//!
//! That machinery — the mirror table, the map conversion, the leg planner and
//! the engine-side replay check — lives in [`super::astar_walk`] and is
//! shared verbatim with the `walk=astar` day walk (Issue #215), so there is
//! exactly one implementation of the conversion in this crate. The
//! correspondence is pinned exhaustively by
//! `mirrored_astar_steps_match_engine_steps`, and every planned path is
//! re-played against the engine's own move rules (bounds / traversability /
//! [`engine::rules::fuel_cost`]) before it is emitted — a conversion
//! regression degrades to the greedy fallback instead of emitting illegal
//! actions.

use engine::model::{
    Action, ActionKind, Agent, AgentActions, HexCoord, MapGrid, Solver, SolverContext,
};
use solver_astar::CostPolicy;

use super::astar_walk::{plan_leg_path, to_astar_map};
use super::greedy_solver::GreedyCandidate;
use super::walk::step_towards;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};

/// Parameter schema, in canonical id order.
pub const PARAMS: &[ParamSpec] = &[ParamSpec {
    key: "policy",
    required: false,
    doc: "A* cost policy: step (default) minimises terrain/traffic weighted \
          step cost with fuel as a hard constraint; conservative scales the \
          step cost up as remaining fuel drops; strict refuses moves while \
          remaining fuel <= 1",
}];

/// The `policy=` parameter values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyKind {
    /// `step` — pure weighted step cost; fuel only bounds feasibility.
    Step,
    /// `conservative` — [`CostPolicy::conservative`]: step cost grows as
    /// remaining fuel drops.
    Conservative,
    /// `strict` — [`CostPolicy::strict`]: never spend the last fuel unit.
    Strict,
}

impl PolicyKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "step" => Some(Self::Step),
            "conservative" => Some(Self::Conservative),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }

    /// Materialise the policy for one agent, using its fuel cap as the
    /// planner's `max_fuel` reference (the A* crate needs the cap to scale
    /// remaining fuel, and it is only known from the live state).
    fn to_cost_policy(self, agent: &Agent) -> CostPolicy {
        match self {
            Self::Step => CostPolicy::default(),
            Self::Conservative => CostPolicy::conservative(agent.fuel_cap),
            Self::Strict => CostPolicy::strict(agent.fuel_cap),
        }
    }
}

/// Parse and validate the `policy=` parameter.
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for an unknown policy name.
fn parse_policy(spec: &CandidateSpec) -> Result<PolicyKind, CandidateError> {
    let bad = |reason: &str| CandidateError::InvalidParameter {
        key: "policy".to_owned(),
        value: spec.param("policy").unwrap_or_default().to_owned(),
        reason: reason.to_owned(),
    };
    match spec.param("policy") {
        None => Ok(PolicyKind::Step),
        Some(value) => {
            PolicyKind::parse(value).ok_or_else(|| bad("expected step | conservative | strict"))
        }
    }
}

/// Greedy fallback for one agent: the same per-step `step_towards` walk
/// [`super::walk::plan_walk`] performs, so a failed A* plan degrades to
/// exactly `greedy`'s behaviour for that agent.
fn walk_actions(
    map: &MapGrid,
    agent: Option<&Agent>,
    goal: Option<HexCoord>,
    day_steps: usize,
) -> Vec<ActionKind> {
    let (Some(agent), Some(goal)) = (agent, goal) else {
        return vec![ActionKind::Wait; day_steps];
    };
    let mut pos = agent.pos;
    let mut plan = Vec::with_capacity(day_steps);
    for _ in 0..day_steps {
        match step_towards(pos, goal, map) {
            Some(dir) => {
                pos = dir.step(pos);
                plan.push(ActionKind::Move(dir));
            }
            None => plan.push(ActionKind::Wait),
        }
    }
    plan
}

/// `astar` candidate: greedy's assignment + A*'s path.
pub struct AstarCandidate {
    policy: PolicyKind,
}

impl AstarCandidate {
    /// Build from an already-validated spec.
    fn from_spec(spec: &CandidateSpec) -> Result<Self, CandidateError> {
        Ok(Self { policy: parse_policy(spec)? })
    }
}

impl Solver for AstarCandidate {
    fn name(&self) -> &'static str {
        "astar"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        // Goal selection: greedy's variety-aware nearest assignment, so the
        // comparison against `greedy` isolates routing quality.
        let targets = GreedyCandidate::new().targets(ctx);
        let map = &ctx.state.setting.map;
        let day_steps = usize::try_from(ctx.day_steps).unwrap_or(0);
        let astar_map = to_astar_map(map);

        let mut per_agent: Vec<Vec<ActionKind>> = Vec::with_capacity(ctx.agent_ids.len());
        for (i, id) in ctx.agent_ids.iter().enumerate() {
            let agent = ctx.state.agents.iter().find(|a| a.id == *id);
            let goal = targets.get(i).and_then(|list| list.first().copied());
            let planned = match (agent, goal, astar_map.as_ref()) {
                (Some(agent), Some(goal), Some(astar_map)) => plan_leg_path(
                    astar_map,
                    map,
                    agent.pos,
                    agent.fuel,
                    goal,
                    ctx.day_steps,
                    self.policy.to_cost_policy(agent),
                )
                .map(|mut plan| {
                    // One day of actions: `plan_leg_path` never returns more
                    // than `day_steps`, so this pads the tail with waits.
                    plan.resize(day_steps, ActionKind::Wait);
                    plan
                }),
                _ => None,
            };
            per_agent.push(planned.unwrap_or_else(|| walk_actions(map, agent, goal, day_steps)));
        }

        // Step-major interleave, matching `plan_walk`'s layout: for each
        // step, one action per controlled agent in `ctx.agent_ids` order.
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
}

/// Factory for [`AstarCandidate`].
pub struct AstarFactory;

impl CandidateFactory for AstarFactory {
    fn algorithm(&self) -> &'static str {
        "astar"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn describe(&self) -> &'static str {
        "Terrain/traffic cost aware A*: greedy assigns each agent a target spot, \
         solver-astar plans the path under weighted step cost and a hard fuel \
         constraint, and the agent follows the planned steps."
    }

    fn validate_spec(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        parse_policy(spec).map(|_| ())
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        AstarCandidate::from_spec(spec).map(|c| Box::new(c) as Box<dyn Solver>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::astar_walk::{mirror_coord, to_cell_kind, to_engine_dir};
    use crate::candidate::CandidateRegistry;
    use domain::{MatchId, PlayerId};
    use engine::MatchSim;
    use engine::model::{
        Agent, Cell, MapSize, MatchSetting, MatchState, Spot, Terrain, TrafficState,
    };
    use engine::rules::fuel_cost as engine_fuel_cost;
    use solver_astar::{CellKind, Direction as AstarDirection, HexCoord as AstarCoord};

    /// An 8x8 map with one patrol-worthy spot and a solid mountain strip
    /// across the direct route: greedy's `step_towards` walks straight
    /// through it, A* routes around on plain cells.
    fn mountain_strip_setting() -> MatchSetting {
        let size = MapSize::new(8, 8).expect("8x8 is in range");
        let mut cells = Vec::with_capacity(64);
        for row in 0..8 {
            for col in 0..8 {
                let coord = HexCoord::new(col, row);
                let terrain = if row == 3 && (1..=6).contains(&col) {
                    Terrain::Mountain
                } else {
                    Terrain::Plain
                };
                let spot = if (col, row) == (7, 3) {
                    Some(Spot { series: 1, pos: coord, max_stock: 1, current_stock: 1 })
                } else {
                    None
                };
                cells.push(Cell { coord, terrain, spot });
            }
        }
        MatchSetting {
            match_id: MatchId::new("astar-strip").expect("match id"),
            team: PlayerId::new("player-a").expect("team id"),
            map: MapGrid::new(size, cells).expect("valid map"),
            agents: vec![
                Agent::patrol(0, HexCoord::new(0, 3), 16, 3),
                Agent::patrol(1, HexCoord::new(0, 6), 16, 3),
                Agent::patrol(2, HexCoord::new(4, 6), 16, 3),
            ],
            day_steps: 16,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        }
    }

    /// A small map whose terrain is asymmetric under the column mirror, so
    /// placing cells without mirroring fails this test.
    fn asymmetric_map() -> MapGrid {
        let size = MapSize::new(8, 8).expect("8x8 is in range");
        let mut cells = Vec::with_capacity(64);
        for row in 0..8 {
            for col in 0..8 {
                let coord = HexCoord::new(col, row);
                let terrain = match (col, row) {
                    (0, 0) => Terrain::Mountain,
                    (4, 4) => Terrain::Pond,
                    (1, 2) => Terrain::Road(TrafficState::Smooth),
                    (2, 5) => Terrain::Road(TrafficState::Busy),
                    (3, 1) => Terrain::Road(TrafficState::Jammed),
                    _ => Terrain::Plain,
                };
                cells.push(Cell { coord, terrain, spot: None });
            }
        }
        MapGrid::new(size, cells).expect("valid map")
    }

    #[test]
    fn factory_accepts_policy_values_and_rejects_bad_params() {
        let reg = CandidateRegistry::builtin();
        for id in ["astar", "astar:policy=step", "astar:policy=conservative", "astar:policy=strict"]
        {
            assert!(reg.build(id).is_ok(), "{id} must be a valid candidate");
        }
        assert!(matches!(
            reg.build("astar:policy=nope"),
            Err(CandidateError::InvalidParameter { .. })
        ));
        assert!(matches!(reg.build("astar:beam=4"), Err(CandidateError::UnknownParameter { .. })));
    }

    #[test]
    fn astar_is_registered_and_ids_round_trip() {
        let reg = CandidateRegistry::builtin();
        assert!(reg.algorithms().contains(&"astar"), "astar must be registered");
        for id in ["astar", "astar:policy=conservative"] {
            let spec = CandidateSpec::parse(id).expect("parses");
            assert_eq!(spec.id(), id, "candidate id must round-trip");
            assert_eq!(reg.build(id).expect("builds").id(), id);
        }
    }

    #[test]
    fn mirrored_astar_steps_match_engine_steps() {
        let width = 8;
        for row in 0..8 {
            for col in 0..8 {
                let coord = HexCoord::new(col, row);
                // The mirror is an involution on the grid.
                assert_eq!(mirror_coord(mirror_coord(coord, width), width), coord);
                for dir in AstarDirection::ALL {
                    let mirrored = mirror_coord(coord, width);
                    let engine_next = to_engine_dir(dir).step(coord);
                    let astar_next = dir.step(AstarCoord::new(mirrored.col, mirrored.row));
                    assert_eq!(
                        mirror_coord(engine_next, width),
                        HexCoord::new(astar_next.col, astar_next.row),
                        "mirrored engine step must equal the A* step for {dir:?} at {coord:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn map_conversion_mirrors_every_terrain() {
        let engine_map = asymmetric_map();
        let astar_map = to_astar_map(&engine_map).expect("8x8 engine map converts to an A* map");
        for cell in engine_map.cells() {
            let mirrored = mirror_coord(cell.coord, 8);
            let kind = astar_map
                .get(AstarCoord::new(mirrored.col, mirrored.row))
                .expect("mirrored coordinate is in bounds");
            assert_eq!(kind, to_cell_kind(cell.terrain), "terrain at {:?}", cell.coord);
        }
        // Explicitly asymmetric spot checks: the mountain at engine (0, 0)
        // must live at A* (7, 0) — identity placement would fail above.
        assert_eq!(astar_map.get(AstarCoord::new(7, 0)), Some(CellKind::Mountain));
        assert_eq!(astar_map.get(AstarCoord::new(0, 0)), Some(CellKind::Plain));
    }

    #[test]
    fn decide_produces_well_shaped_in_bounds_affordable_actions() {
        let state = MatchState::new(mountain_strip_setting()).expect("valid state");
        let ctx = SolverContext::from_state(&state);
        let candidate = AstarCandidate::from_spec(&CandidateSpec::bare("astar")).expect("builds");
        let plan = candidate.decide(&ctx);
        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");

        let map = &state.setting.map;
        let agent_idx: Vec<usize> = ctx
            .agent_ids
            .iter()
            .map(|id| state.agents.iter().position(|a| a.id == *id).expect("agent exists"))
            .collect();
        let mut pos: Vec<HexCoord> = agent_idx.iter().map(|&i| state.agents[i].pos).collect();
        let mut fuel: Vec<u32> = agent_idx.iter().map(|&i| state.agents[i].fuel).collect();
        let n = ctx.agent_ids.len();
        for (i, action) in plan.actions.iter().enumerate() {
            let slot = i % n;
            assert_eq!(action.agent_id, ctx.agent_ids[slot]);
            if let ActionKind::Move(dir) = action.kind {
                let next = dir.step(pos[slot]);
                let cell = map.get(next).expect("planned move stays in bounds");
                assert!(cell.terrain.traversable(), "planned move lands on traversable terrain");
                let cost = engine_fuel_cost(cell.terrain);
                assert!(fuel[slot] >= cost, "planned move is affordable");
                fuel[slot] -= cost;
                pos[slot] = next;
            }
        }
    }

    #[test]
    fn astar_avoids_mountain_strip_that_greedy_walks_through() {
        let astar = AstarCandidate::from_spec(&CandidateSpec::bare("astar")).expect("builds");
        let greedy = GreedyCandidate::new();
        let sim = MatchSim::default();

        let astar_run = sim.run(mountain_strip_setting(), &astar).expect("astar run");
        let greedy_run = sim.run(mountain_strip_setting(), &greedy).expect("greedy run");

        for (name, run) in [("astar", &astar_run), ("greedy", &greedy_run)] {
            assert_eq!(run.illegal_action_count, 0, "{name}: no illegal actions");
            assert_eq!(run.fuel_short_count, 0, "{name}: no fuel shortages");
            assert_eq!(run.total_balls, 1, "{name}: reaches the spot");
            assert_eq!(
                run.final_state.agents[0].pos,
                HexCoord::new(7, 3),
                "{name}: agent 0 ends on the spot"
            );
        }

        let astar_fuel = astar_run.final_state.agents[0].fuel;
        let greedy_fuel = greedy_run.final_state.agents[0].fuel;
        // Greedy walks the straight line through six mountain cells
        // (6 × 2 + 1 = 13 fuel); A* detours on plain cells (9 fuel).
        assert!(
            astar_fuel > greedy_fuel,
            "A* must spend less fuel than greedy's mountain crossing \
             (astar {astar_fuel} vs greedy {greedy_fuel})"
        );
    }
}
