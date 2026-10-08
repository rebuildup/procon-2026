//! Independent #316 hypothesis: shortest next harvest over all spots, using
//! #313's joint fuel replay but removing the series-first detours.
use super::coverage_solver::CoverageCandidate;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{AgentActions, Solver, SolverContext};
pub struct HarvestFactory;
const PARAMS: &[ParamSpec] =
    &[ParamSpec { key: "budget", required: true, doc: "official (required)" }];
struct HarvestCandidate;
impl Solver for HarvestCandidate {
    fn name(&self) -> &'static str {
        "harvest"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        CoverageCandidate { variety_first: false }.decide(ctx)
    }
}
impl CandidateFactory for HarvestFactory {
    fn algorithm(&self) -> &'static str {
        "harvest"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "All-spot nearest harvest with joint supply replay"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("harvest requires budget=official".into()));
        }
        Ok(Box::new(HarvestCandidate))
    }
}
