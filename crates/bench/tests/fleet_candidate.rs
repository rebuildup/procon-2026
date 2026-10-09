//! Fleet candidate (Issue #371): registry validation, reproducibility and
//! official-rule cleanliness on the realistic 701 board.
use bench::CandidateRegistry;
use bench::adapters::walk::BudgetMode;
use bench::case::CaseSpec;
use bench::kinds::{parse_kinds, resolve_kinds};
use domain::PlayerId;
use engine::MatchSim;
use engine::model::{AgentActions, MatchState, Solver, SolverContext};

fn solver(id: &str) -> Box<dyn Solver> {
    CandidateRegistry::builtin().build(id).and_then(|c| c.build_solver()).expect("id builds")
}

fn state_701(pattern: &str) -> (CaseSpec, MatchState) {
    let case: CaseSpec = serde_json::from_str(include_str!("fixtures/arena-701.json")).unwrap();
    let kinds = resolve_kinds(Some(&parse_kinds(pattern).unwrap()), case.setting.agents.len());
    let setting = case
        .to_engine_setting_with_kinds(&PlayerId::new("player-a").unwrap(), Some(&kinds))
        .unwrap();
    let state = MatchState::new(setting).unwrap();
    (case, state)
}

/// Play 701 to the end: `(daily variety sum, balls)`. Every day's plan must
/// already be a fixed point of the official normalization and replay clean.
fn play_701(id: &str, pattern: &str) -> (usize, u32) {
    let (case, mut state) = state_701(pattern);
    let solver = solver(id);
    let sim = MatchSim::new(case.sim_config().unwrap());
    let agents = state.agents.len();
    while !state.is_finished() {
        let ctx = SolverContext::from_state(&state);
        let plan = solver.decide(&ctx);
        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "day {}", state.day);
        assert_eq!(
            BudgetMode::Official.apply(&ctx, plan.clone()),
            plan,
            "normalization changed day {} {pattern}",
            state.day
        );
        for chunk in plan.actions.chunks(agents) {
            sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
        }
    }
    assert_eq!(state.illegal_action_count, 0, "{id} {pattern}");
    assert_eq!(state.fuel_short_count, 0, "{id} {pattern}");
    assert_eq!(state.rejected_response_count, 0, "{id} {pattern}");
    let daily = state.series_acquired_by_day.values().map(std::collections::BTreeSet::len).sum();
    let balls = state.agents.iter().map(engine::Agent::total_inventory).sum();
    (daily, balls)
}

#[test]
fn fleet_requires_official_budget_and_rejects_bad_values() {
    let registry = CandidateRegistry::builtin();
    let build = |id: &str| registry.build(id).and_then(|c| c.build_solver());
    assert!(build("fleet:budget=official").is_ok());
    assert!(build("fleet:budget=official:iters=100:seed=7").is_ok());
    assert!(build("fleet").is_err());
    assert!(build("fleet:budget=element").is_err());
    assert!(build("fleet:budget=official:iters=many").is_err());
    assert!(build("fleet:budget=official:budget_ms=60000").is_err());
    assert!(build("fleet:budget=official:park=far").is_err());
}

#[test]
fn fleet_iters_mode_is_reproducible() {
    let (_, state) = state_701("S,P,P,P,P");
    let ctx = SolverContext::from_state(&state);
    let id = "fleet:budget=official:iters=300:seed=42";
    assert_eq!(solver(id).decide(&ctx), solver(id).decide(&ctx));
}

/// One Supply serving four Patrols must keep all of them fuelled: every
/// series every day on 701 (20 series x 10 days), far more balls than the
/// paired-escort `coverage` reaches with its best composition (429).
#[test]
fn fleet_one_supply_covers_701_every_day() {
    let (daily, balls) = play_701("fleet:budget=official:iters=1500:seed=42", "S,P,P,P,P");
    assert!(daily >= 198, "daily variety sum {daily}");
    assert!(balls >= 470, "balls {balls}");
}

/// Compositions the planner cannot improve on must still return a legal day:
/// no Supply at all, and the historical alternation.
#[test]
fn fleet_stays_legal_without_supply_and_with_alternation() {
    play_701("fleet:budget=official:iters=200:seed=1", "P,P,P,P,P");
    play_701("fleet:budget=official:iters=200:seed=1", "P,S");
}

/// A team with no Patrol has nothing to plan; the answer is still a
/// well-shaped day.
#[test]
fn fleet_all_supply_returns_a_well_shaped_day() {
    let (_, state) = state_701("S");
    let ctx = SolverContext::from_state(&state);
    let plan = solver("fleet:budget=official:iters=50").decide(&ctx);
    assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
}
