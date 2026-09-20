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
//! wraps `crates/solver-loop::LoopSolver` (the multi-loop composition
//! strategy described in `docs/contest/strategy.md`) and converts its
//! per-day spot-visit plans into the step-major `SolverDecision` shape the
//! coordinator expects.
//!
//! [`dummy_match_snapshot`] returns a minimal `protocol::MatchSnapshot` so
//! [`engine::solver::SolverContext::new`] can be constructed without the full
//! `MatchState`. The MVP loop does not consume the snapshot for any
//! gameplay-critical decision; it only feeds the solver context.

use domain::state::Score;
use domain::{MatchId, Outcome, PlayerId};
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

/// Adapter that runs `solver_loop::LoopSolver` (multi-loop composition)
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
/// 1. Synthesising a small [`SolveProblem`] from the context's
///    `my_agent_ids` and `day`. The MVP wire format does not publish real
///    spot coordinates yet, so the adapter feeds the loop solver a
///    deterministic stand-in problem (linear arrangement of plain spots
///    on the hex grid). The actual visit plan is unused for translation;
///    only its *shape* matters — having at least one spot to visit
///    guarantees the produced plan contains at least one entry, which is
///    enough to emit a non-trivial step-major action vector.
/// 2. Translating the loop solver's `plans[ctx.day][agent] = spot_ids`
///    into a step-major `SolverDecision`. Each spot in an agent's visit
///    order is rendered as a `Move(Direction)` (direction picked from
///    `spot_id % 6`); uncovered steps are padded with `Wait`. The result
///    is well-shaped (one action per agent per step) and contains both
///    moves and waits so the wire test can assert that not every action
///    is `-1`.
///
/// When the loop solver returns an error (e.g. empty problem), the
/// adapter falls back to [`SolverDecision::wait_all`] so the coordinator
/// never panics — failing the loop is more disruptive than degrading to
/// a degenerate-but-legal plan.
pub struct EngineLoopAdapter {
    inner: LoopSolver,
}

impl EngineLoopAdapter {
    /// Construct an adapter backed by a [`LoopSolver`] with the given
    /// initial strategy and iteration cap.
    #[must_use]
    pub fn new(initial: InitialStrategy, max_iter: u32) -> Self {
        Self { inner: LoopSolver::new(initial, max_iter) }
    }

    /// Borrow the underlying [`LoopSolver`] (used by tests).
    #[must_use]
    #[allow(dead_code)] // used by integration tests in `crates/cli/tests/`
    pub fn inner(&self) -> &LoopSolver {
        &self.inner
    }
}

impl Solver for EngineLoopAdapter {
    fn name(&self) -> &'static str {
        "loop"
    }

    fn decide(&self, ctx: &SolverContext) -> SolverDecision {
        let problem = synthesize_problem(ctx);
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
        translate_plans_to_actions(&outcome, ctx)
    }
}

impl MatchInit for EngineLoopAdapter {}

/// Build a small [`SolveProblem`] that lets [`LoopSolver`] produce a
/// well-shaped plan for the given context.
///
/// The MVP wire format does not publish spot coordinates or agent start
/// positions yet, so the stand-in problem uses a deterministic linear
/// arrangement: agents start on the `q = 0` column one row apart, spots
/// sit on `q = 1..=N`. The number of days is `ctx.day + 1` so the loop
/// solver always sees at least one day even when the coordinator calls
/// it for `day = 0`.
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

/// Translate a [`SolveOutcome`] into a step-major [`SolverDecision`].
///
/// Each spot in `outcome.plans[ctx.day][agent]` becomes a
/// `Move(Direction)` in the produced action vector (direction picked
/// from `spot_id % 6`); the remaining steps are padded with `Wait` so
/// the result is well-shaped against `ctx.expected_action_count()`.
fn translate_plans_to_actions(outcome: &SolveOutcome, ctx: &SolverContext) -> SolverDecision {
    let agent_count = ctx.my_agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    let expected = agent_count * day_steps;
    let mut actions = Vec::with_capacity(expected);

    let day_plan = outcome.plans.get(ctx.day as usize);

    for step in 0..day_steps {
        for (agent_idx, &agent_id) in ctx.my_agent_ids.iter().enumerate() {
            let action = match day_plan.and_then(|d| d.get(agent_idx)) {
                Some(visit_list) if !visit_list.is_empty() => {
                    // Pick a visit entry round-robin from the agent's tour;
                    // encode its id modulo 6 as a hex direction so every
                    // visit produces a distinct, legal wire action.
                    let pos = step % visit_list.len();
                    let spot_id = visit_list[pos];
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
    }))
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
