//! Independent metric-tour hypothesis: each Patrol repeats a full spot tour
//! each day, entering from its actual start; paths and supply contacts may overlap.
use super::coverage_solver::CoverageCandidate;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{AgentActions, Solver, SolverContext};
pub struct RepeatTourFactory;
const PARAMS: &[ParamSpec] =
    &[ParamSpec { key: "budget", required: true, doc: "official (required)" }];
struct RepeatTourCandidate;
impl Solver for RepeatTourCandidate {
    fn name(&self) -> &'static str {
        "repeat-tour"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        CoverageCandidate { repeat_tour: true }.decide(ctx)
    }
}
impl CandidateFactory for RepeatTourFactory {
    fn algorithm(&self) -> &'static str {
        "repeat-tour"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "Per-Patrol metric nearest-neighbor tours with shared supply replay"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("repeat-tour requires budget=official".into()));
        }
        Ok(Box::new(RepeatTourCandidate))
    }
}
