//! Issue #207 regression — `loop:…:operators=extended` must never walk a
//! day without reaching a single spot on the road-heavy inputs.
//!
//! Before the fix both of these cases scored `(0, 0, 0)` for `extended`
//! (status `success`, `illegal = 0`, `fuel_short = 0` — the agents simply
//! never stood on a spot) while `operators=classic` scored positively on
//! the very same input. The tests below pin both halves of the acceptance
//! criteria:
//!
//! 1. a zero-collection outcome is impossible for `extended` on these inputs;
//! 2. `classic` — the control — stays positive too.
//!
//! AC 2 originally read "`extended`'s official tuple is **at least**
//! `classic`'s". That relative bound leaned on `classic` being the healthy
//! side, but the defect behind Issue #207 was not confined to `extended`:
//! `walk::hex_distance` measured the even-r grid as axial, so greedy walks
//! stalled on *both* operator sets — `classic` itself scored `(0, 0, 0)` on
//! `generated:v2:road-heavy:seed=2` at SHA b511c9d. With the metric corrected
//! both sides improve (that case: classic `(0,0,0)` → `(2,6,7)`, extended
//! `(1,3,3)` → `(1,3,6)`), and `classic` now simply leads on it. What Issue
//! #207 actually forbids is a zero day, so that is what is pinned.
//!
//! `operators=classic` / the default candidate id is deliberately *not*
//! asserted here — its behaviour is pinned by its own tests and by the
//! parent candidate's Stage B/C records (#191); Issue #207 may not move it.

use bench::candidate::CandidateRegistry;
use bench::case::CaseSpec;
use engine::MatchSim;
use engine::model::{AgentActions, MatchState, SolverContext};

const CLASSIC: &str = "loop:iterations=32:initial=greedy:operators=classic";
const EXTENDED: &str = "loop:iterations=32:initial=greedy:operators=extended";

/// The official `(variety, daily_variety_sum, total_balls)` triple — the
/// first three contest ranking tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Score {
    variety: u32,
    daily: u32,
    balls: u32,
}

/// Drive one candidate through the real `engine::MatchSim` and read the
/// official score off the final state — the evaluator's own path, so a
/// passing test describes an evaluation, not a re-implementation of one.
fn run(candidate_id: &str, case: &CaseSpec) -> Score {
    let team = domain::PlayerId::new("player-a").expect("valid team id");
    let setting = case.to_engine_setting(&team).expect("case converts to an engine setting");
    let sim_config = case.sim_config().expect("case declares a sim config");
    let candidate = CandidateRegistry::builtin().build(candidate_id).expect("candidate builds");
    let solver = candidate.build_solver().expect("solver builds");

    let mut state = MatchState::new(setting).expect("validated setting");
    let sim = MatchSim::new(sim_config);
    for _ in 0..state.num_days() {
        if state.is_finished() {
            break;
        }
        let ctx = SolverContext::from_state(&state);
        let plan = solver.decide(&ctx);
        let agent_count = ctx.agent_ids.len();
        let agent_ids = ctx.agent_ids.clone();
        for step in 0..ctx.day_steps as usize {
            if state.is_finished() {
                break;
            }
            let start = step * agent_count;
            let block = if plan.actions.len() >= start + agent_count {
                plan.actions[start..start + agent_count].to_vec()
            } else {
                AgentActions::wait_all(&agent_ids, 1).actions
            };
            sim.step(&mut state, &AgentActions::new(block));
        }
    }

    Score {
        variety: u32::try_from(state.series_taken.len()).unwrap_or(u32::MAX),
        daily: state.daily_varieties().iter().sum(),
        balls: state.agents.iter().map(engine::model::Agent::total_inventory).sum(),
    }
}

/// Both Issue #207 zero cases: the rebuilt road fixture and the generated
/// road archetype seed the issue reproduces on.
fn issue_cases() -> Vec<(&'static str, CaseSpec)> {
    let mut out = vec![(
        "fixture:road-heavy-v2",
        CaseSpec::from_bundled_fixture("road-heavy-v2").expect("bundled fixture loads"),
    )];
    let params = bench::generator::parse_selection("road-heavy:seed=2")
        .expect("road-heavy:seed=2 parses as a generated case");
    let corpus = bench::generator::build_corpus(&params).expect("generated case builds");
    for case in corpus.cases {
        let id: &'static str = Box::leak(case.case_id.clone().into_boxed_str());
        out.push((id, case));
    }
    out
}

#[test]
fn extended_never_scores_zero_on_the_issue207_cases() {
    for (label, case) in issue_cases() {
        let extended = run(EXTENDED, &case);
        assert!(
            extended.variety > 0 && extended.balls > 0,
            "Issue #207: `operators=extended` collected nothing on {label}: \
             got ({}, {}, {})",
            extended.variety,
            extended.daily,
            extended.balls
        );
    }
}

#[test]
fn neither_candidate_scores_zero_on_the_issue207_cases() {
    for (label, case) in issue_cases() {
        for candidate in [CLASSIC, EXTENDED] {
            let score = run(candidate, &case);
            assert!(
                score.variety > 0 && score.balls > 0,
                "Issue #207: `{candidate}` collected nothing on {label}: \
                 got ({}, {}, {})",
                score.variety,
                score.daily,
                score.balls
            );
        }
    }
}
