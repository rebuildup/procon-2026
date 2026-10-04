//! Translate between `engine::SolverDecision` and the wire `ActionPlanBody`.
//!
//! `ActionPlanBody` is the official `POST /` body: an **outer** list with one
//! inner list per controlled agent, and each **inner** list is a run-length
//! encoded day sequence:
//!
//! - `-N` (N > 0)  -> wait `N` steps,
//! - `0..=5`       -> move one cell in the matching hex direction
//!   (`0` upper-left, `1` upper-right, `2` right, `3` lower-right,
//!   `4` lower-left, `5` left).
//!
//! The element sum of each agent's inner list (`-N` counts as `N`, a move
//! as 1) **must** equal `ctx.day_steps[day]` — that is the *engine clock*
//! this adapter encodes, validated on both the encode and decode paths.
//!
//! The **official** server rule is different: it sums terrain-weighted move
//! costs (平地 2 / 山 3 / 道路 1-2-4, Q&A Q11) plus waits and requires that
//! total to equal `daySteps` (Issue #226 probe evidence lives in
//! `engine::wire_plan`). Candidate output is therefore normalized to the
//! weighted budget at the submission boundary — see
//! `crate::coordinator_loop::run_coordinator_loop` → `engine::wire_plan` —
//! *after* this adapter has encoded it.
//!
//! `engine::SolverDecision` is **step-major**: the first `agent_count` entries
//! cover step 0 (one `Action` per controlled agent, in
//! `SolverContext::my_agent_ids` order), then step 1, and so on. So the
//! encode/decode routines chunk the flat list into per-agent, per-step
//! sequences without ever allocating per-step vectors.

use engine::solver::{
    Action as SAction, ActionKind as SActionKind, Direction as SDirection, SolverContext,
    SolverDecision,
};
use protocol::match_config::ActionPlanBody;

/// Errors produced by the decision <-> action adapter.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// The decision shape does not match `ctx.expected_action_count()`.
    #[error("decision has {actual} actions, expected {expected}")]
    Shape { actual: usize, expected: usize },

    /// A per-agent inner sequence does not consume exactly
    /// `ctx.day_steps` steps.
    #[error("agent {agent_index}: step sum {computed}, expected {expected}")]
    StepSum { agent_index: usize, computed: i64, expected: i64 },

    /// A wire action code (decoded from `POST /`) is outside the documented
    /// `0..=5` / `-N` (N > 0) range.
    #[error("agent {agent_index}: action {value} is not a legal wire code")]
    BadWireAction { agent_index: usize, value: i32 },

    /// The action referenced an `agent_id` that is not in
    /// `ctx.my_agent_ids` at the expected position.
    #[error("agent index {0}: unexpected agent id {1}")]
    AgentMismatch(usize, u8),
}

/// Encode a [`SolverDecision`] as the official [`ActionPlanBody`].
///
/// `SolverDecision` is **step-major** (`expected_action_count()` actions,
/// with the first `agent_count` covering step 0 in agent-id order, etc.).
/// The wire `ActionPlanBody` is **agent-major** (one inner list per
/// controlled agent, in `my_agent_ids` order). This routine transposes
/// the step-major layout into the agent-major wire form and run-length
/// encodes each agent's sequence:
///
/// - contiguous `Wait` runs collapse into a single `-N`,
/// - `Move(dir)` becomes `dir as i32` (`0..=5`).
///
/// # Errors
///
/// Returns [`AdapterError::Shape`] if `decision.actions.len() != ctx.my_agent_ids.len() * ctx.day_steps`,
/// and [`AdapterError::AgentMismatch`] if the per-step agent id does not
/// match `ctx.my_agent_ids` at the expected slot.
pub fn decision_to_actions(
    d: &SolverDecision,
    ctx: &SolverContext,
) -> Result<ActionPlanBody, AdapterError> {
    let agent_count = ctx.my_agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    let expected = agent_count * day_steps;
    if d.actions.len() != expected {
        return Err(AdapterError::Shape { actual: d.actions.len(), expected });
    }

    // Pre-validate the step-major layout: at every step, the per-agent
    // action id must match `my_agent_ids` at the same slot. Cheap because
    // we touch each action exactly once.
    for step in 0..day_steps {
        for (agent_index, expected_id) in ctx.my_agent_ids.iter().enumerate() {
            let actual_id = d.actions[step * agent_count + agent_index].agent_id;
            if actual_id != *expected_id {
                return Err(AdapterError::AgentMismatch(agent_index, actual_id));
            }
        }
    }

    // Walk each agent's actions in order (stride = agent_count) and
    // run-length encode them.
    let mut out: ActionPlanBody = Vec::with_capacity(agent_count);
    for (agent_index, _) in ctx.my_agent_ids.iter().enumerate() {
        let mut inner: Vec<i32> = Vec::new();
        let mut computed: i64 = 0;
        for step in 0..day_steps {
            let action = &d.actions[step * agent_count + agent_index];
            match action.kind {
                SActionKind::Wait => {
                    // Coalesce consecutive waits.
                    if let Some(last) = inner.last_mut() {
                        if *last < 0 {
                            *last -= 1;
                            computed += 1;
                            continue;
                        }
                    }
                    inner.push(-1);
                    computed += 1;
                }
                SActionKind::Move(dir) => {
                    inner.push(dir as i32);
                    computed += 1;
                }
            }
        }
        if computed != i64::from(ctx.day_steps) {
            return Err(AdapterError::StepSum {
                agent_index,
                computed,
                expected: i64::from(ctx.day_steps),
            });
        }
        out.push(inner);
    }
    Ok(out)
}

/// Decode a wire [`ActionPlanBody`] back into a [`SolverDecision`].
///
/// Inverse of [`decision_to_actions`]. The outer list must have exactly
/// `ctx.my_agent_ids.len()` entries and each inner list's step sum must
/// equal `ctx.day_steps`.
///
/// # Errors
///
/// Returns [`AdapterError::Shape`], [`AdapterError::StepSum`],
/// [`AdapterError::AgentMismatch`], or [`AdapterError::BadWireAction`] when
/// the wire payload does not match the documented contract.
#[allow(dead_code)] // inverse direction is wired only by tests in this MVP
pub fn actions_to_decision(
    body: &ActionPlanBody,
    ctx: &SolverContext,
) -> Result<SolverDecision, AdapterError> {
    let agent_count = ctx.my_agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    if body.len() != agent_count {
        return Err(AdapterError::Shape { actual: body.len(), expected: agent_count });
    }

    // Decode agent-major -> step-major. For each step `s` (0..day_steps) push
    // the per-agent action for that step in `my_agent_ids` order.
    //
    // The inner list of each agent is a run-length sequence; expand waits
    // into one Wait per consumed step.
    let mut actions: Vec<SAction> = Vec::with_capacity(agent_count * day_steps);
    // Build a per-agent expanded sequence first so step-major indexing
    // becomes a straightforward zip.
    let mut per_agent: Vec<Vec<SAction>> = Vec::with_capacity(agent_count);
    for (agent_index, (agent_id, inner)) in ctx.my_agent_ids.iter().zip(body.iter()).enumerate() {
        let mut flat: Vec<SAction> = Vec::with_capacity(day_steps);
        let mut step_sum: i64 = 0;
        for token in inner {
            match *token {
                v if v < 0 => {
                    // Negate in 64-bit: negating the `i32` first overflows on
                    // `i32::MIN` in debug builds, and the `wait_count < 0`
                    // guard below could never catch it. Fail fast on an
                    // overshoot too, so a token like -2_000_000_000 cannot
                    // push ~2e9 waits before the `step_sum` check runs.
                    let wait_count = i64::from(v).abs();
                    let projected = step_sum + wait_count;
                    if projected > i64::from(ctx.day_steps) {
                        return Err(AdapterError::StepSum {
                            agent_index,
                            computed: projected,
                            expected: i64::from(ctx.day_steps),
                        });
                    }
                    let count = usize::try_from(wait_count).unwrap_or(0);
                    for _ in 0..count {
                        flat.push(SAction::wait(*agent_id));
                    }
                    step_sum = projected;
                }
                v @ 0..=5 => {
                    let dir =
                        wire_dir(v).ok_or(AdapterError::BadWireAction { agent_index, value: v })?;
                    flat.push(SAction::step(*agent_id, dir));
                    step_sum += 1;
                }
                v => {
                    return Err(AdapterError::BadWireAction { agent_index, value: v });
                }
            }
        }
        if step_sum != i64::from(ctx.day_steps) {
            return Err(AdapterError::StepSum {
                agent_index,
                computed: step_sum,
                expected: i64::from(ctx.day_steps),
            });
        }
        per_agent.push(flat);
    }
    // Interleave per-agent flat sequences into step-major output.
    for step in 0..day_steps {
        for (agent_index, agent_flat) in per_agent.iter().enumerate() {
            actions.push(agent_flat[step]);
            let _ = agent_index;
        }
    }
    Ok(SolverDecision::new(actions))
}

/// Convert a wire direction code (`0..=5`) to the engine [`SDirection`].
///
/// Returns `None` if the code is outside the documented range. The cast is
/// total because [`SDirection`] variants are declared in wire-code order
/// (see `engine::solver::Direction`).
#[allow(dead_code)] // used only by the inverse-direction decoder in tests
fn wire_dir(v: i32) -> Option<SDirection> {
    match v {
        0 => Some(SDirection::UpperLeft),
        1 => Some(SDirection::UpperRight),
        2 => Some(SDirection::Right),
        3 => Some(SDirection::LowerRight),
        4 => Some(SDirection::LowerLeft),
        5 => Some(SDirection::Left),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::state::{GameState, Score};
    use domain::{MatchId, PlayerId};
    use protocol::MatchSnapshot;

    fn ctx(agent_ids: Vec<u8>, day_steps: u32) -> SolverContext {
        let match_id = MatchId::new("match-test").unwrap();
        let team = PlayerId::new("player-a").unwrap();
        let snapshot = MatchSnapshot {
            match_id: match_id.clone(),
            state: GameState { match_id, turn: 0, scores: vec![(team.clone(), Score(0))] },
            outcome: domain::Outcome::Pending,
            server_time_unix_ms: 0,
        };
        SolverContext::new(snapshot, team, agent_ids, 0, day_steps, 60_000)
    }

    #[test]
    fn wait_all_round_trips() {
        let ctx = ctx(vec![0, 1, 2], 3);
        let decision = SolverDecision::wait_all(&ctx);
        let body = decision_to_actions(&decision, &ctx).expect("encode");
        // Three agents, each runs the entire day waiting -> single -3 token.
        assert_eq!(body, vec![vec![-3], vec![-3], vec![-3]]);
        let back = actions_to_decision(&body, &ctx).expect("decode");
        assert_eq!(back, decision);
    }

    #[test]
    fn direction_sequence_round_trips() {
        // Agent 0: move right, wait 2. Agent 1: wait 1, move upper-left.
        // Step-major layout: each step lists agent 0 then agent 1.
        let ctx = ctx(vec![0, 1], 3);
        let actions = vec![
            SAction::step(0, SDirection::Right),     // step 0: agent 0
            SAction::wait(1),                        // step 0: agent 1
            SAction::wait(0),                        // step 1: agent 0
            SAction::step(1, SDirection::UpperLeft), // step 1: agent 1
            SAction::wait(0),                        // step 2: agent 0
            SAction::wait(1),                        // step 2: agent 1
        ];
        let decision = SolverDecision::new(actions);
        let body = decision_to_actions(&decision, &ctx).expect("encode");
        assert_eq!(body, vec![vec![2, -2], vec![-1, 0, -1]]);
        let back = actions_to_decision(&body, &ctx).expect("decode");
        assert_eq!(back, decision);
    }

    #[test]
    fn wrong_shape_is_rejected() {
        let ctx = ctx(vec![0, 1], 3);
        let too_short = SolverDecision::new(vec![SAction::wait(0); 3]);
        assert!(matches!(decision_to_actions(&too_short, &ctx), Err(AdapterError::Shape { .. })));
    }

    #[test]
    fn step_sum_off_by_one_is_rejected() {
        // 3 day_steps but the inner sequence sums to 4 (-2 move + wait1 move + wait1).
        let ctx = ctx(vec![0, 1], 3);
        let body: ActionPlanBody = vec![vec![2, -2, 1], vec![-3]]; // agent 0 sums to 4, not 3
        assert!(matches!(
            actions_to_decision(&body, &ctx),
            Err(AdapterError::StepSum { agent_index: 0, computed: 4, expected: 3 })
        ));
    }

    #[test]
    fn bad_wire_action_is_rejected() {
        let ctx = ctx(vec![0], 3);
        let body: ActionPlanBody = vec![vec![2, 9, 0]];
        assert!(matches!(
            actions_to_decision(&body, &ctx),
            Err(AdapterError::BadWireAction { agent_index: 0, value: 9 })
        ));
    }

    #[test]
    fn i32_min_wait_token_is_rejected_without_panicking() {
        // `i64::from(-v)` used to negate the i32 first: debug builds panicked
        // with "attempt to negate with overflow" before the guard could run.
        let ctx = ctx(vec![0], 3);
        let body: ActionPlanBody = vec![vec![i32::MIN]];
        assert!(matches!(
            actions_to_decision(&body, &ctx),
            Err(AdapterError::StepSum { agent_index: 0, computed: 2_147_483_648, expected: 3 })
        ));
    }

    #[test]
    fn oversized_wait_token_is_rejected_before_expansion() {
        // A hostile token must fail on the budget check, not after pushing
        // ~2e9 waits into `flat`.
        let ctx = ctx(vec![0], 3);
        let body: ActionPlanBody = vec![vec![-2_000_000_000]];
        assert!(matches!(
            actions_to_decision(&body, &ctx),
            Err(AdapterError::StepSum { agent_index: 0, computed: 2_000_000_000, expected: 3 })
        ));
    }
}
