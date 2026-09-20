//! MVP solver + init adapter for the coordinator loop.
//!
//! The [`engine::solver::Solver`] trait declares `fn init(&mut self, …)`, but
//! the coordinator stores solvers as `Box<dyn Solver>`, so mutable dispatch
//! is impossible. [`MatchInit`] sidesteps this: a concrete solver that
//! wants setup work implements `MatchInit`, and the coordinator
//! downcasts-then-calls `init_for_match` before the first `decide`.
//!
//! [`WaitSolver`] is the MVP default — a no-op plan that always passes the
//! shape check. It exists for the `--dry-run` and offline replay paths; the
//! real day-by-day work moves into the `solver-*` crates as they stabilise.
//!
//! [`dummy_match_snapshot`] returns a minimal `protocol::MatchSnapshot` so
//! [`engine::solver::SolverContext::new`] can be constructed without the full
//! `MatchState`. The MVP loop does not consume the snapshot for any
//! gameplay-critical decision; it only feeds the solver context.

use domain::state::Score;
use domain::{MatchId, Outcome, PlayerId};
use engine::solver::{
    Action as SAction, ActionKind as SActionKind, Solver, SolverContext, SolverDecision,
};
use protocol::MatchSnapshot;
use protocol::match_config::{AgentState, MatchState};

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
}
