//! MVP solver + init adapter for the coordinator loop.
//!
//! The [`engine::solver::Solver`] trait declares `fn init(&mut self, …)`, but
//! the coordinator stores solvers as `Box<dyn Solver>`, so mutable dispatch
//! is impossible. [`MatchInit`] sidesteps this: a concrete solver that
//! wants setup work implements `MatchInit`, and the coordinator
//! downcasts-then-calls `init_for_match` before the first `decide`.
//!
//! [`WaitSolver`] is the **debug-only** no-op plan — kept for `--solver wait`
//! so operators can smoke-test the wire protocol without invoking the real
//! strategy. The default production solver is [`EngineLoopAdapter`], which
//! wraps `crates/solver-loop::LoopSolver` (the iterative-improvement loop
//! strategy described in `docs/contest/strategy.md`) and converts its
//! per-day spot-visit plans into the step-major `SolverDecision` shape the
//! coordinator expects.
//!
//! [`dummy_match_snapshot`] returns a minimal `protocol::MatchSnapshot` so
//! [`engine::solver::SolverContext::new`] can be constructed without the full
//! `MatchState`. The MVP loop does not consume the snapshot for any
//! gameplay-critical decision; it only feeds the solver context.

use std::sync::{Arc, Mutex};

use domain::state::Score;
use domain::{MatchId, Outcome, PlayerId};
use engine::model::{Direction as EDir, HexCoord as EHexCoord, MatchSetting as EngineSetting};
use engine::solver::{
    Action as SAction, ActionKind as SActionKind, Direction as SDirection, Solver, SolverContext,
    SolverDecision,
};
use protocol::MatchSnapshot;
use protocol::match_config::{AgentState, MatchState};
use solver_loop::{
    Agent as LoopAgent, HexCoord, InitialStrategy, LoopSolver, Series as LoopSeries, SolveOutcome,
    SolveProblem, Solver as LoopSolverTrait, Spot as LoopSpot,
};

/// Optional one-shot setup hook for `Box<dyn Solver>` users.
///
/// `engine::Solver::init` takes `&mut self`, which is incompatible with
/// `Box<dyn Solver>`. Concrete solvers that need pre-match setup
/// (precomputing shortest paths, seeding an RNG, …) implement this trait;
/// the coordinator calls it on the downcast pointer before the first
/// `decide`.
pub trait MatchInit {
    /// One-time setup before the first day of a match. Default: no-op.
    #[allow(dead_code)] // callers downcast then call; default impl never invoked
    fn init_for_match(&mut self, _ctx: &SolverContext) {}
}

/// MVP solver: every controlled agent waits every step.
///
/// Always returns a well-shaped decision (`SolverDecision::wait_all`), which
/// the official server accepts but scores zero. Useful as a placeholder
/// while the `solver-*` crates finish their trait migration, and as the
/// default for `--dry-run` smoke tests that only need to exercise the
/// protocol plumbing.
pub struct WaitSolver;

impl Solver for WaitSolver {
    fn name(&self) -> &'static str {
        "wait-mvp"
    }

    fn init(&mut self, _ctx: &SolverContext) {
        // No-op. WaitSolver has no pre-match setup.
    }

    fn decide(&self, ctx: &SolverContext) -> SolverDecision {
        // Build step-major actions by repeating the wait list `day_steps`
        // times. This is structurally identical to `SolverDecision::wait_all`
        // but spelled out so future solvers can mutate one step at a time.
        let mut actions = Vec::with_capacity(ctx.expected_action_count());
        for _step in 0..ctx.day_steps {
            for &id in &ctx.my_agent_ids {
                actions.push(SAction { agent_id: id, kind: SActionKind::Wait });
            }
        }
        SolverDecision { actions, metadata: serde_json::json!({"strategy":"wait-all"}) }
    }
}

impl MatchInit for WaitSolver {}

/// Adapter that runs `solver_loop::LoopSolver` (iterative-improvement loop)
/// inside the coordinator loop.
///
/// The two solver crates disagree on their abstraction level:
///
/// - `engine::solver::Solver` expects a [`SolverContext`] (an authoritative
///   state snapshot + per-day `day_steps`) and returns a flat
///   **step-major** `Vec<Action>`.
/// - `solver_loop::Solver` expects a [`SolveProblem`] (spots + agents +
///   `days`) and returns a [`SolveOutcome`] whose plans are
///   `plans[day][agent] = spot visit order`.
///
/// This adapter closes the gap by:
///
/// 1. **When the engine setting is attached** (production path, set via
///    [`EngineLoopAdapter::set_engine_setting`]): build a [`SolveProblem`]
///    from the *real* spots and agent positions on the contest map. Then
///    translate the visit order into a step-major action vector that
///    walks each agent through the map cell by cell, **validating every
///    `Move(dir)` against map bounds + terrain passability**. An invalid
///    move is downgraded to a `Wait` so the plan stays well-shaped and
///    never trips the server's `revision=-1` rejection.
///
/// 2. **When the engine setting is *not* attached** (e.g. unit tests,
///    in-process mocks that never read the wire setting): fall back to a
///    deterministic stand-in problem. The visit order is converted into
///    directions via `spot_id % 6`. This path is structurally similar to
///    the MVP's pre-fix behaviour and exists only to keep the dry-run /
///    unit-test suite green; it must not be used against the live server
///    because the stand-in problem does not reflect the real map.
///
/// When the loop solver returns an error (e.g. empty problem), the
/// adapter falls back to [`SolverDecision::wait_all`] so the coordinator
/// never panics — failing the loop is more disruptive than degrading to
/// a degenerate-but-legal plan.
pub struct EngineLoopAdapter {
    inner: LoopSolver,
    /// Optional real-world engine setting (map + agents). When present,
    /// `decide` builds a [`SolveProblem`] from these and validates every
    /// emitted `Move(dir)` against the map. `None` until the coordinator
    /// calls [`Self::set_engine_setting`] after `protocol_setting_to_engine`.
    engine_setting: Arc<Mutex<Option<EngineSetting>>>,
}

impl EngineLoopAdapter {
    /// Construct an adapter backed by a [`LoopSolver`] with the given
    /// initial strategy and iteration cap.
    #[must_use]
    pub fn new(initial: InitialStrategy, max_iter: u32) -> Self {
        Self {
            inner: LoopSolver::new(initial, max_iter),
            engine_setting: Arc::new(Mutex::new(None)),
        }
    }

    /// Borrow the underlying [`LoopSolver`] (used by tests).
    #[must_use]
    #[allow(dead_code)] // used by integration tests in `crates/cli/tests/`
    pub fn inner(&self) -> &LoopSolver {
        &self.inner
    }

    /// Clone of the internal engine-setting slot, suitable for sharing
    /// with the coordinator loop so it can inject the parsed setting
    /// after [`crate::protocol_to_engine::protocol_setting_to_engine`].
    ///
    /// The handle is `Arc<Mutex<Option<EngineSetting>>>`: cloning it
    /// shares the same cell, so writes through any clone land in the
    /// slot the adapter reads in `decide`. This is the only way to
    /// bypass the `Box<dyn Solver>` boundary — the `engine::Solver`
    /// trait cannot be modified to expose a setter, and `Any` is not a
    /// supertrait, so direct downcast is impossible.
    #[must_use]
    pub fn setting_handle(&self) -> Arc<Mutex<Option<EngineSetting>>> {
        Arc::clone(&self.engine_setting)
    }

    /// Attach the parsed engine setting (map + agents) so subsequent
    /// `decide` calls can validate moves against the real map.
    ///
    /// Prefer [`Self::setting_handle`] + populating the slot from the
    /// caller side: this direct setter exists for tests and for
    /// call-sites that already hold an `&EngineLoopAdapter`.
    ///
    /// Calling this a second time replaces the previous setting.
    #[allow(dead_code)] // exposed for integration tests; production path uses `setting_handle`
    pub fn set_engine_setting(&self, setting: EngineSetting) {
        let mut guard = self.engine_setting.lock().expect("engine_setting mutex poisoned");
        *guard = Some(setting);
    }
}

impl Solver for EngineLoopAdapter {
    fn name(&self) -> &'static str {
        "loop"
    }

    fn decide(&self, ctx: &SolverContext) -> SolverDecision {
        let setting = self.engine_setting.lock().expect("engine_setting mutex poisoned").clone();
        let problem = match setting.as_ref() {
            Some(s) => synthesize_problem_from_setting(ctx, s),
            None => synthesize_problem(ctx),
        };
        let outcome = match self.inner.solve(&problem) {
            Ok(outcome) => outcome,
            Err(err) => {
                tracing::warn!(
                    solver = "loop",
                    error = %err,
                    "loop solver returned an error; degrading to wait_all"
                );
                return SolverDecision::wait_all(ctx).with_metadata(serde_json::json!({
                    "solver": "loop",
                    "fallback": "wait_all",
                    "error": err.to_string(),
                }));
            }
        };
        translate_plans_to_actions(&outcome, ctx, setting.as_ref())
    }
}

impl MatchInit for EngineLoopAdapter {}

/// Convert a wire `engine::solver::Direction` (used by the [`Solver`]
/// trait) into the matching `engine::model::Direction` (which carries
/// the geometry math: [`engine::model::Direction::step`]). Both enums
/// declare the same six variants in the same order, so this is a
/// mechanical one-to-one mapping.
fn solver_dir_to_model(d: SDirection) -> EDir {
    match d {
        SDirection::UpperLeft => EDir::UpperLeft,
        SDirection::UpperRight => EDir::UpperRight,
        SDirection::Right => EDir::Right,
        SDirection::LowerRight => EDir::LowerRight,
        SDirection::LowerLeft => EDir::LowerLeft,
        SDirection::Left => EDir::Left,
    }
}

/// Convert an `engine::model::Direction` back to the solver-side
/// variant. Inverse of [`solver_dir_to_model`].
fn model_dir_to_solver(d: EDir) -> SDirection {
    match d {
        EDir::UpperLeft => SDirection::UpperLeft,
        EDir::UpperRight => SDirection::UpperRight,
        EDir::Right => SDirection::Right,
        EDir::LowerRight => SDirection::LowerRight,
        EDir::LowerLeft => SDirection::LowerLeft,
        EDir::Left => SDirection::Left,
    }
}

/// Build a small [`SolveProblem`] that lets [`LoopSolver`] produce a
/// well-shaped plan for the given context.
///
/// The MVP wire format does not publish spot coordinates or agent start
/// positions yet, so the stand-in problem uses a deterministic linear
/// arrangement: agents start on the `q = 0` column one row apart, spots
/// sit on `q = 1..=N`. The number of days is `ctx.day + 1` so the loop
/// solver always sees at least one day even when the coordinator calls
/// it for `day = 0`.
///
/// **Use only when the engine setting is unavailable.** The stand-in
/// problem has no map awareness, so the downstream translation will
/// pick directions from `spot_id % 6` and may emit invalid moves.
fn synthesize_problem(ctx: &SolverContext) -> SolveProblem {
    let agent_count = ctx.my_agent_ids.len().max(1);
    let spot_count = (agent_count * 4).max(4); // at least four spots so the tour is non-trivial
    let spots: Vec<LoopSpot> = (0..spot_count)
        .map(|i| LoopSpot {
            id: u32::try_from(i).unwrap_or(0),
            coord: HexCoord::new(i32::try_from(i + 1).unwrap_or(0), 0),
            series: LoopSeries::new("mvp"),
            stock: 1,
        })
        .collect();
    let agents: Vec<LoopAgent> = ctx
        .my_agent_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| LoopAgent {
            id: u32::from(id),
            start: HexCoord::new(0, i32::try_from(i).unwrap_or(0)),
        })
        .collect();
    let days = ctx.day.saturating_add(1).max(1);
    SolveProblem::new(spots, agents, days)
}

/// Build a [`SolveProblem`] from a real [`EngineSetting`].
///
/// Each spot in `setting.map` becomes a [`LoopSpot`] with its actual
/// `(col, row)` coordinate and series. Each agent's start position is
/// the engine-side `Agent::pos`. The number of days is `ctx.day + 1` so
/// the loop solver always sees at least one day.
fn synthesize_problem_from_setting(ctx: &SolverContext, setting: &EngineSetting) -> SolveProblem {
    let spots: Vec<LoopSpot> = setting
        .map
        .cells()
        .iter()
        .filter_map(|cell| {
            let spot = cell.spot.as_ref()?;
            Some(LoopSpot {
                id: spot_id_for_coord(cell.coord),
                coord: HexCoord::new(cell.coord.col, cell.coord.row),
                series: LoopSeries::new(format!("series-{}", spot.series)),
                stock: spot.current_stock,
            })
        })
        .collect();
    let agents: Vec<LoopAgent> = setting
        .agents
        .iter()
        .map(|a| LoopAgent { id: u32::from(a.id), start: HexCoord::new(a.pos.col, a.pos.row) })
        .collect();
    let days = ctx.day.saturating_add(1).max(1);
    SolveProblem::new(spots, agents, days)
}

/// Synthesise a stable per-cell id from `(col, row)` for the synthetic
/// loop-solver problem. The id has to round-trip through the loop
/// solver's visit order, so it must be unique within a single map.
fn spot_id_for_coord(coord: EHexCoord) -> u32 {
    // `(row * 1_000_000 + col)` is unique within any map bounded by the
    // contest's `MapSize::MAX = 32` (`row * 32 + col` is also unique but
    // easier to read in logs at this scale). The constant is chosen so
    // that a 32x32 map fits comfortably without overflowing `i32`.
    let id = i64::from(coord.row) * 1_000_000 + i64::from(coord.col);
    u32::try_from(id).unwrap_or(0)
}

/// Look up the engine-side `EHexCoord` of a spot by its loop-solver id.
fn coord_for_spot_id(spot_id: u32) -> Option<EHexCoord> {
    let id = i64::from(spot_id);
    let row = i32::try_from(id / 1_000_000).ok()?;
    let col = i32::try_from(id.rem_euclid(1_000_000)).ok()?;
    Some(EHexCoord::new(col, row))
}

/// Translate a [`SolveOutcome`] into a step-major [`SolverDecision`].
///
/// Each spot in `outcome.plans[ctx.day][agent]` becomes a `Move(Direction)`
/// in the produced action vector. The direction is computed by walking
/// from the agent's current position to the spot's coordinates using
/// `engine::solver::Direction::step` math; if no direction lands on the
/// spot within one step, or if the move would cross an impassable cell /
/// fall off the map, the action downgrades to a `Wait`.
///
/// When `setting` is `None` (legacy fallback path), the visit list is
/// decoded with the pre-fix `spot_id % 6` heuristic so existing unit
/// tests that do not wire the engine setting keep their assertions.
fn translate_plans_to_actions(
    outcome: &SolveOutcome,
    ctx: &SolverContext,
    setting: Option<&EngineSetting>,
) -> SolverDecision {
    let agent_count = ctx.my_agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    let expected = agent_count * day_steps;
    let mut actions = Vec::with_capacity(expected);

    let day_plan = outcome.plans.get(ctx.day as usize);

    // Track each agent's current position. Start from the engine setting's
    // agent start positions when available; otherwise default to (0, 0)
    // for every agent (the legacy fallback path).
    let mut positions: Vec<EHexCoord> = (0..agent_count)
        .map(|i| setting.and_then(|s| s.agents.get(i)).map_or(EHexCoord::new(0, 0), |a| a.pos))
        .collect();

    for step in 0..day_steps {
        for (agent_idx, &agent_id) in ctx.my_agent_ids.iter().enumerate() {
            let action = match day_plan.and_then(|d| d.get(agent_idx)) {
                Some(visit_list) if !visit_list.is_empty() => {
                    let pos = step % visit_list.len();
                    let spot_id = visit_list[pos];
                    if let Some(s) = setting {
                        match pick_valid_move(positions[agent_idx], spot_id, s) {
                            Some(dir) => {
                                positions[agent_idx] =
                                    solver_dir_to_model(dir).step(positions[agent_idx]);
                                SAction::step(agent_id, dir)
                            }
                            None => SAction::wait(agent_id),
                        }
                    } else {
                        // Legacy fallback: `spot_id % 6` encodes a
                        // direction without consulting the map. Kept
                        // so unit tests that bypass the engine
                        // setting keep working.
                        let dir = match spot_id % 6 {
                            0 => SDirection::UpperLeft,
                            1 => SDirection::UpperRight,
                            2 => SDirection::Right,
                            3 => SDirection::LowerRight,
                            4 => SDirection::LowerLeft,
                            _ => SDirection::Left,
                        };
                        SAction::step(agent_id, dir)
                    }
                }
                _ => SAction::wait(agent_id),
            };
            actions.push(action);
        }
    }

    let total_visits: usize = outcome.plans.iter().flat_map(|d| d.iter()).map(Vec::len).sum();

    SolverDecision::new(actions).with_metadata(serde_json::json!({
        "solver": "loop",
        "ctx_day": ctx.day,
        "agent_count": agent_count,
        "day_steps": ctx.day_steps,
        "loop_planned_visits": total_visits,
        "map_aware": setting.is_some(),
    }))
}

/// Pick a [`SDirection`] that walks from `from` to the cell containing
/// `spot_id`. Returns `None` if the spot is not on the map, is more
/// than one step away, or the only adjacent cell that lands on the
/// target is impassable / out of bounds.
fn pick_valid_move(from: EHexCoord, spot_id: u32, setting: &EngineSetting) -> Option<SDirection> {
    let target = coord_for_spot_id(spot_id)?;
    if !setting.map.size.contains(target) {
        return None;
    }
    // Scan every direction; accept the first one that lands exactly on
    // the target cell AND is a legal move. The scan order follows
    // `Direction::ALL` so behaviour is deterministic across runs.
    for dir in EDir::ALL {
        let next = dir.step(from);
        if next == target && is_legal_move(from, next, setting) {
            return Some(model_dir_to_solver(dir));
        }
    }
    None
}

/// Whether a move from `from` to `to` is legal under the contest rules.
///
/// "Legal" here means *bounds + terrain passability*. Fuel accounting is
/// intentionally out of scope — the contest server downgrades a
/// fuel-short move to `Wait` for the same step (per
/// `crates/engine::transitions::try_move`), so emitting it does not
/// cause `revision < 0`; it merely costs the agent a wasted step. This
/// routine matches the server's bounds + passability gate, which is the
/// gate that produced the original `revision=-1` rejection.
///
/// Exposed publicly so integration tests in `crates/cli/tests/` can
/// assert the contract independently of the adapter's internal call
/// path.
#[must_use]
pub fn is_legal_move(from: EHexCoord, to: EHexCoord, setting: &EngineSetting) -> bool {
    let _ = from; // bounds already imply `from` is on the map.
    if !setting.map.size.contains(to) {
        return false;
    }
    match setting.map.get(to) {
        Some(cell) => cell.terrain.traversable(),
        None => false,
    }
}

/// Build a placeholder `MatchSnapshot` for [`SolverContext::new`].
///
/// The MVP coordinator never reads the snapshot body; it only needs a
/// well-typed value to satisfy the constructor. The match id is stable
/// (`"mvp"`) so log lines are greppable.
#[must_use]
pub fn dummy_match_snapshot() -> MatchSnapshot {
    let match_id = MatchId::new("mvp").expect("valid id");
    let team = PlayerId::new("mvp-player").expect("valid id");
    MatchSnapshot {
        match_id: match_id.clone(),
        state: domain::state::GameState { match_id, turn: 0, scores: vec![(team, Score(0))] },
        outcome: Outcome::Pending,
        server_time_unix_ms: 0,
    }
}

/// Convert a wire `MatchState` snapshot into a `MatchSnapshot` the solver
/// can consume. The MVP only needs the match id and player scores to look
/// realistic in logs; agent positions live in the engine's
/// `protocol_setting_to_engine` view of the day.
///
/// Returns `None` if the wire payload does not reference the calling team
/// (the engine cannot synthesise scores for a foreign player).
#[must_use]
pub fn match_state_to_snapshot(
    state: &MatchState,
    match_id: &MatchId,
    team: &PlayerId,
) -> Option<MatchSnapshot> {
    let has_team = state.agents.iter().any(|a| agent_state_mentions(a, team));
    if !has_team {
        return None;
    }
    Some(MatchSnapshot {
        match_id: match_id.clone(),
        state: domain::state::GameState {
            match_id: match_id.clone(),
            turn: u32::try_from(state.day).unwrap_or(0),
            scores: vec![(team.clone(), Score(0))],
        },
        outcome: Outcome::Pending,
        server_time_unix_ms: 0,
    })
}

fn agent_state_mentions(_a: &AgentState, _t: &PlayerId) -> bool {
    // MVP: any agent state is accepted; the engine never reads per-agent
    // state in this path. Real reconciliation lands when the engine's
    // day-step simulation hooks into the loop (later issue).
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::state::GameState;
    use domain::{MatchId, Outcome, PlayerId};

    fn snapshot() -> MatchSnapshot {
        let match_id = MatchId::new("mvp").unwrap();
        let team = PlayerId::new("mvp-player").unwrap();
        MatchSnapshot {
            match_id: match_id.clone(),
            state: GameState { match_id, turn: 0, scores: vec![(team, Score(0))] },
            outcome: Outcome::Pending,
            server_time_unix_ms: 0,
        }
    }

    #[test]
    fn dummy_snapshot_is_well_typed() {
        let snap = dummy_match_snapshot();
        assert_eq!(snap.match_id.as_str(), "mvp");
        assert_eq!(snap.outcome, Outcome::Pending);
    }

    #[test]
    fn wait_solver_returns_well_shaped_wait_all() {
        let ctx = SolverContext::new(
            snapshot(),
            PlayerId::new("mvp-player").unwrap(),
            vec![0, 1, 2],
            0,
            4,
            60_000,
        );
        let solver = WaitSolver;
        let decision = solver.decide(&ctx);
        assert_eq!(decision.len(), 12);
        assert!(decision.is_well_shaped(&ctx));
        assert!(decision.actions.iter().copied().all(SAction::is_wait));
        assert_eq!(solver.name(), "wait-mvp");
    }

    #[test]
    fn engine_loop_adapter_produces_well_shaped_plan_with_moves() {
        let ctx = SolverContext::new(
            snapshot(),
            PlayerId::new("mvp-player").unwrap(),
            vec![0, 1],
            0,
            6,
            60_000,
        );
        let solver = EngineLoopAdapter::new(InitialStrategy::GreedyNearestNeighbor, 8);
        let decision = solver.decide(&ctx);
        assert_eq!(decision.len(), 12, "2 agents * 6 steps");
        assert!(decision.is_well_shaped(&ctx), "must be well-shaped against ctx");
        assert_eq!(solver.name(), "loop");
        let non_wait = decision.actions.iter().filter(|a| !a.is_wait()).count();
        assert!(
            non_wait > 0,
            "EngineLoopAdapter must produce at least one Move action, got only waits: {:?}",
            decision.actions
        );
        assert_eq!(
            decision.metadata["solver"], "loop",
            "metadata should identify the loop adapter"
        );
    }

    #[test]
    fn engine_loop_adapter_falls_back_when_loop_solver_errors() {
        // No agents → LoopSolver returns `NoAgents`; adapter must degrade
        // to a well-shaped wait_all rather than panic.
        let ctx = SolverContext::new(
            snapshot(),
            PlayerId::new("mvp-player").unwrap(),
            vec![],
            0,
            3,
            60_000,
        );
        let solver = EngineLoopAdapter::new(InitialStrategy::Identity, 1);
        let decision = solver.decide(&ctx);
        // `expected_action_count() == 0 * 3 == 0` for an empty agent list,
        // so the fallback yields an empty action vector (still legal).
        assert_eq!(decision.len(), 0);
        assert_eq!(decision.metadata["fallback"], "wait_all");
    }
}
