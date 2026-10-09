//! Seeded reconstruction changes representatives and vehicle assignment.
//! Candidate retention replays actual actions with `MatchSim`, never a tour proxy.
//!
//! Two stopping rules (Issue #311, the answer-time gap): `trials` bounds the
//! deterministic search and `time_budget_ms` additionally caps wall clock so
//! live can spend the official answer window instead of answering in
//! milliseconds. With a time budget the trials fan out over the machine's
//! cores and the best-so-far keeps strict-improvement retention in trial
//! order, so equal-scoring plans resolve to the earliest trial.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::coverage_solver::{CoverageCandidate, mix};
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{AgentActions, Solver, SolverContext};
pub struct SearchFactory;
type SearchScore = (usize, usize, u32);
type ScoredTrial = (u64, SearchScore, AgentActions);
const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "trials",
        required: false,
        doc: "1..1048576 deterministic reconstructions (default 8)",
    },
    ParamSpec { key: "seed", required: false, doc: "reconstruction seed (default 42)" },
    ParamSpec {
        key: "time_budget_ms",
        required: false,
        doc: "wall-clock cap per day plan: 0 = off (default, deterministic) | \
              1..59000 stops trials early and runs them over the machine's cores \
              (live: 20000 leaves the rest of the 60s day window to the wire)",
    },
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
    ParamSpec { key: "park", required: false, doc: "none (default) | far" },
    ParamSpec {
        key: "lookahead",
        required: false,
        doc: "none (default) | next (one official day ahead; requires time_budget_ms=0)",
    },
];
struct SearchCandidate {
    trials: u32,
    seed: u64,
    time_budget_ms: u32,
    park_risk: bool,
    lookahead_next: bool,
}
fn replay_score(ctx: &SolverContext<'_>, plan: &AgentActions) -> SearchScore {
    let mut state = ctx.state.clone();
    let sim = engine::MatchSim::new(ctx.sim_config);
    for step in plan.actions.chunks(ctx.agent_ids.len()) {
        sim.step(&mut state, &AgentActions::new(step.to_vec()));
    }
    (
        state.series_acquired_by_day.get(&ctx.day).map_or(0, std::collections::BTreeSet::len),
        0,
        state.agents.iter().map(engine::model::Agent::total_inventory).sum(),
    )
}
fn official_prefix_score(state: &engine::model::MatchState) -> SearchScore {
    (
        state.series_taken.len(),
        state.series_acquired_by_day.values().map(std::collections::BTreeSet::len).sum(),
        state.agents.iter().map(engine::model::Agent::total_inventory).sum(),
    )
}
fn replay_next_day_score(
    ctx: &SolverContext<'_>,
    plan: &AgentActions,
    park_risk: bool,
) -> SearchScore {
    let sim = engine::MatchSim::new(ctx.sim_config);
    let mut state = ctx.state.clone();
    for step in plan.actions.chunks(ctx.agent_ids.len()) {
        sim.step(&mut state, &AgentActions::new(step.to_vec()));
    }
    if !state.is_finished() {
        let next_ctx = SolverContext::from_state_with_config(&state, ctx.sim_config);
        let next_plan = CoverageCandidate { park_risk, variant: 0, fuel_aware: false }.decide(&next_ctx);
        for step in next_plan.actions.chunks(next_ctx.agent_ids.len()) {
            sim.step(&mut state, &AgentActions::new(step.to_vec()));
        }
    }
    official_prefix_score(&state)
}
impl SearchCandidate {
    fn score(&self, ctx: &SolverContext<'_>, plan: &AgentActions) -> SearchScore {
        if self.lookahead_next {
            replay_next_day_score(ctx, plan, self.park_risk)
        } else {
            replay_score(ctx, plan)
        }
    }
}
impl Solver for SearchCandidate {
    fn name(&self) -> &'static str {
        "search"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let started = Instant::now();
        let mut best =
            CoverageCandidate { park_risk: self.park_risk, variant: 0, fuel_aware: false }
                .decide(ctx);
        let mut score = self.score(ctx, &best);
        // The wall clock is bounded from both sides (Issue #311, live
        // match 1762's revision=-1): the explicit `time_budget_ms` and the
        // per-day answer window the coordinator hands us as `remaining_ms`.
        // Whichever is shorter wins, with a 3s margin for adapter +
        // normalize + POST — a plan that answers after `endsAt` is
        // discarded server-side no matter how good it is.
        let live_window = (ctx.remaining_ms != u32::MAX)
            .then(|| Duration::from_millis(u64::from(ctx.remaining_ms.saturating_sub(3_000))));
        let search_budget = (self.time_budget_ms > 0)
            .then(|| Duration::from_millis(u64::from(self.time_budget_ms)));
        let deadline = match (live_window, search_budget) {
            (Some(a), Some(b)) => Some(started + a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(started + a),
            (None, None) => None,
        };
        if self.time_budget_ms == 0 {
            for trial in 1..self.trials {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    break;
                }
                let plan = CoverageCandidate {
                    park_risk: self.park_risk,
                    variant: mix(self.seed.wrapping_add(u64::from(trial))),
                    fuel_aware: false,
                }
                .decide(ctx);
                let candidate_score = self.score(ctx, &plan);
                if candidate_score > score {
                    best = plan;
                    score = candidate_score;
                }
            }
            return best;
        }
        let deadline = deadline.expect("explicit search time budget");

        // Wall-clock mode: fan the trials out over the machine's cores and
        // stop launching new work at the deadline (Issue #311 — millisecond
        // answers leave the whole answer window unused).
        let threads = std::thread::available_parallelism().map_or(1, usize::from).clamp(1, 16);
        let next = AtomicU64::new(1);
        let stop = AtomicBool::new(false);
        let winners: Mutex<Vec<ScoredTrial>> = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    let mut local: Option<ScoredTrial> = None;
                    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                        let trial = next.fetch_add(1, Ordering::Relaxed);
                        if trial >= u64::from(self.trials) {
                            stop.store(true, Ordering::Relaxed);
                            break;
                        }
                        let plan = CoverageCandidate {
                            park_risk: self.park_risk,
                            variant: mix(self.seed.wrapping_add(trial)),
                            fuel_aware: false,
                        }
                        .decide(ctx);
                        let candidate_score = self.score(ctx, &plan);
                        if local.as_ref().is_none_or(|(_, s, _)| candidate_score > *s) {
                            local = Some((trial, candidate_score, plan));
                        }
                    }
                    if let Some(entry) = local {
                        winners.lock().expect("winner lock").push(entry);
                    }
                });
            }
        });
        let mut winners = winners.into_inner().expect("winner lock");
        // Trial order makes ties resolve like the deterministic loop above:
        // the earliest strict improvement wins.
        winners.sort_by_key(|(trial, _, _)| *trial);
        for (_, candidate_score, plan) in winners {
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
        let time_budget_ms = spec.param_u32("time_budget_ms")?.unwrap_or(0);
        let park_risk = match spec.param("park") {
            None | Some("none") => false,
            Some("far") => true,
            Some(other) => {
                return Err(CandidateError::Build(format!("unknown park mode {other:?}")));
            }
        };
        let lookahead_next = match spec.param("lookahead") {
            None | Some("none") => false,
            Some("next") => true,
            Some(other) => {
                return Err(CandidateError::Build(format!("unknown lookahead mode {other:?}")));
            }
        };
        if !(1..=1_048_576).contains(&trials)
            || time_budget_ms > 59_000
            || spec.param("budget") != Some("official")
        {
            return Err(CandidateError::Build(
                "search requires trials=1..1048576, time_budget_ms<=59000 and budget=official"
                    .into(),
            ));
        }
        if lookahead_next && time_budget_ms > 0 {
            return Err(CandidateError::Build(
                "search lookahead=next requires time_budget_ms=0".into(),
            ));
        }
        Ok(Box::new(SearchCandidate {
            trials,
            seed: spec.param_u64("seed")?.unwrap_or(42),
            time_budget_ms,
            park_risk,
            lookahead_next,
        }))
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
        let routed = CoverageCandidate { park_risk: false, variant: 0, fuel_aware: false }.decide(&ctx);
        assert!(replay_score(&ctx, &routed) > replay_score(&ctx, &wait));
    }

    /// Issue #311: the wall-clock mode keeps the strict-improvement retention
    /// (never below the pure coverage plan) and stays inside the day
    /// contract. The budget is small on purpose — this pins behaviour, not
    /// performance.
    #[test]
    fn time_budget_search_is_no_worse_than_the_pure_coverage_plan() {
        let case: crate::CaseSpec =
            serde_json::from_str(include_str!("../../tests/fixtures/arena-701.json")).unwrap();
        let setting = case.to_engine_setting(&domain::PlayerId::new("player-a").unwrap()).unwrap();
        let state = engine::model::MatchState::new(setting).unwrap();
        let ctx = SolverContext::from_state(&state);
        let coverage_score = replay_score(
            &ctx,
            &CoverageCandidate { park_risk: false, variant: 0, fuel_aware: false }.decide(&ctx),
        );
        let searched = SearchCandidate {
            trials: 1_048_576,
            seed: 42,
            time_budget_ms: 150,
            park_risk: false,
            lookahead_next: false,
        }
        .decide(&ctx);
        assert!(searched.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");
        assert!(
            replay_score(&ctx, &searched) >= coverage_score,
            "retention keeps at least the base"
        );
    }

    #[test]
    fn next_day_lookahead_keeps_at_least_the_base_horizon_score() {
        let case: crate::CaseSpec =
            serde_json::from_str(include_str!("../../tests/fixtures/arena-701.json")).unwrap();
        let config = case.sim_config().unwrap();
        let setting = case.to_engine_setting(&domain::PlayerId::new("player-a").unwrap()).unwrap();
        let state = engine::model::MatchState::new(setting).unwrap();
        let ctx = SolverContext::from_state_with_config(&state, config);
        let baseline = CoverageCandidate { park_risk: true, variant: 0, fuel_aware: false }.decide(&ctx);
        let baseline_score = replay_next_day_score(&ctx, &baseline, true);
        let candidate = SearchCandidate {
            trials: 4,
            seed: 42,
            time_budget_ms: 0,
            park_risk: true,
            lookahead_next: true,
        };
        let selected = candidate.decide(&ctx);
        assert!(selected.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
        assert!(replay_next_day_score(&ctx, &selected, true) >= baseline_score);
    }
}
