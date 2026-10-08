//! Seeded reconstruction changes representatives and vehicle assignment.
//! Candidate retention replays actual actions with `MatchSim`, never a tour proxy.
use super::coverage_solver::{CoverageCandidate, mix};
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{AgentActions, Solver, SolverContext};
pub struct SearchFactory;
const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "trials",
        required: false,
        doc: "1..64 deterministic reconstructions (default 8)",
    },
    ParamSpec { key: "seed", required: false, doc: "reconstruction seed (default 42)" },
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
];
struct SearchCandidate {
    trials: u32,
    seed: u64,
}
// This is a one-day retention criterion, not a claim about the full match.
fn replay_score(ctx: &SolverContext<'_>, plan: &AgentActions) -> (usize, u32) {
    let mut state = ctx.state.clone();
    let sim = engine::MatchSim::default();
    for step in plan.actions.chunks(ctx.agent_ids.len()) {
        sim.step(&mut state, &AgentActions::new(step.to_vec()));
    }
    (
        state.series_acquired_by_day.get(&ctx.day).map_or(0, std::collections::BTreeSet::len),
        state.agents.iter().map(engine::model::Agent::total_inventory).sum(),
    )
}
impl Solver for SearchCandidate {
    fn name(&self) -> &'static str {
        "search"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let mut best = CoverageCandidate { variant: 0 }.decide(ctx);
        let mut score = replay_score(ctx, &best);
        for trial in 1..self.trials {
            let plan = CoverageCandidate { variant: mix(self.seed.wrapping_add(u64::from(trial))) }
                .decide(ctx);
            let candidate_score = replay_score(ctx, &plan);
            if candidate_score > score {
                best = plan;
                score = candidate_score;
            }
        }
        best
    }
}
impl CandidateFactory for SearchFactory {
    fn algorithm(&self) -> &'static str {
        "search"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "Bounded seeded daily reconstruction with canonical action replay"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        let trials = spec.param_u32("trials")?.unwrap_or(8);
        if !(1..=64).contains(&trials) || spec.param("budget") != Some("official") {
            return Err(CandidateError::Build(
                "search requires trials=1..64 and budget=official".into(),
            ));
        }
        Ok(Box::new(SearchCandidate { trials, seed: spec.param_u64("seed")?.unwrap_or(42) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_changes_when_actions_reach_a_spot() {
        let case: crate::CaseSpec =
            serde_json::from_str(include_str!("../../tests/fixtures/arena-701.json")).unwrap();
        let setting = case.to_engine_setting(&domain::PlayerId::new("player-a").unwrap()).unwrap();
        let state = engine::model::MatchState::new(setting).unwrap();
        let ctx = SolverContext::from_state(&state);
        let wait = AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps);
        let routed = CoverageCandidate { variant: 0 }.decide(&ctx);
        assert!(replay_score(&ctx, &routed) > replay_score(&ctx, &wait));
    }
}
