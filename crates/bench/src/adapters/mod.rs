//! Candidate adapters: turn concrete solver crates into
//! [`engine::model::Solver`]s the canonical evaluator can drive.
//!
//! Each adapter answers exactly one question — *"given this day's
//! [`engine::model::SolverContext`], what should my agents do?"* — and returns
//! [`engine::model::AgentActions`]. Nothing here computes a score. The score
//! comes from [`engine::MatchSim`] alone.
//!
//! # Registering a new candidate
//!
//! Implement [`crate::candidate::CandidateFactory`] and register it on a
//! [`crate::candidate::CandidateRegistry`]. One crate may expose several
//! algorithms; one algorithm may expose many parameter sets. Neither needs a
//! new crate.

pub mod astar_solver;
pub mod context;
pub mod greedy_solver;
pub mod loop_solver;
pub mod multiloop_solver;
pub mod nearest_solver;
pub mod walk;

pub use astar_solver::AstarFactory;
pub use greedy_solver::GreedyFactory;
pub use loop_solver::LoopFactory;
pub use multiloop_solver::MultiLoopFactory;
pub use nearest_solver::NearestFactory;

use engine::model::{AgentActions, Solver, SolverContext};

use crate::candidate::{CandidateError, CandidateSpec, ParamSpec};

/// Baseline candidate: every agent waits every step.
///
/// Scores zero on every tier, which makes it a stable anchor for
/// win / draw / lose reporting — and it is also the cheapest smoke test that
/// the whole evaluation path is wired (a broken path cannot even produce a
/// zero record).
pub struct WaitSolver;

impl Solver for WaitSolver {
    fn name(&self) -> &'static str {
        "wait"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
    }
}

/// Factory for [`WaitSolver`]. Takes no parameters.
pub struct WaitFactory;

impl crate::candidate::CandidateFactory for WaitFactory {
    fn algorithm(&self) -> &'static str {
        "wait"
    }

    fn params(&self) -> &'static [ParamSpec] {
        &[]
    }

    fn describe(&self) -> &'static str {
        "Baseline: every agent waits all day. Scores zero on every tier."
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        if let Some((key, _)) = spec.params.first() {
            return Err(CandidateError::UnknownParameter {
                algorithm: spec.algorithm.clone(),
                key: key.clone(),
            });
        }
        Ok(Box::new(WaitSolver))
    }
}

/// Helper shared by the algorithm factories: reject any parameter the schema
/// does not list.
pub(crate) fn reject_unknown_params(
    spec: &CandidateSpec,
    schema: &'static [ParamSpec],
) -> Result<(), CandidateError> {
    for (key, _) in &spec.params {
        if !schema.iter().any(|p| p.key == key.as_str()) {
            return Err(CandidateError::UnknownParameter {
                algorithm: spec.algorithm.clone(),
                key: key.clone(),
            });
        }
    }
    Ok(())
}
