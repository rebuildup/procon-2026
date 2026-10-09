//! Reproduce the day-8 trajectory comparison from Issue #340.
//!
//! Run with:
//! `cargo test --release --locked -p bench --test day8_trace -- --ignored --nocapture`

use bench::{CandidateRegistry, CaseSpec};
use engine::{
    MatchSim,
    model::{AgentActions, MatchState, Solver, SolverContext},
};

fn apply_day(
    state: &MatchState,
    plan: &AgentActions,
    sim: MatchSim,
    agent_count: usize,
) -> MatchState {
    let mut after = state.clone();
    for actions in plan.actions.chunks(agent_count) {
        sim.step(&mut after, &AgentActions::new(actions.to_vec()));
    }
    after
}

fn daily_count(state: &MatchState, day: u32) -> usize {
    state.series_acquired_by_day.get(&day).map_or(0, std::collections::BTreeSet::len)
}

fn describe_agents(state: &MatchState) -> String {
    state
        .agents
        .iter()
        .map(|agent| {
            format!(
                "{}@{},{}:fuel={}:balls={}",
                agent.id,
                agent.pos.row,
                agent.pos.col,
                agent.fuel,
                agent.total_inventory()
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[test]
#[ignore = "manual trajectory diagnostic; prints day-by-day state evidence"]
fn fixture_701_day8_regression_has_path_dependent_start_positions() {
    let case: CaseSpec =
        serde_json::from_str(include_str!("../tests/fixtures/arena-701.json")).unwrap();
    let team = domain::PlayerId::new("player-a").unwrap();
    let setting = case.to_engine_setting(&team).unwrap();
    let kinds = setting.agent_ids().len();
    let sim = MatchSim::new(case.sim_config().unwrap());
    let registry = CandidateRegistry::builtin();
    let baseline: Box<dyn Solver> =
        registry.build("coverage:budget=official:park=far").unwrap().build_solver().unwrap();
    let search: Box<dyn Solver> = registry
        .build("search:trials=32:seed=42:budget=official:park=far")
        .unwrap()
        .build_solver()
        .unwrap();
    let mut baseline_state = MatchState::new(setting.clone()).unwrap();
    let mut search_state = MatchState::new(setting).unwrap();

    while !baseline_state.is_finished() && !search_state.is_finished() {
        let day = baseline_state.day;
        let baseline_ctx =
            SolverContext::from_state_with_config(&baseline_state, case.sim_config().unwrap());
        let search_ctx =
            SolverContext::from_state_with_config(&search_state, case.sim_config().unwrap());
        let baseline_plan = baseline.decide(&baseline_ctx);
        let search_plan = search.decide(&search_ctx);
        let shadow_plan = baseline.decide(&search_ctx);

        let baseline_after = apply_day(&baseline_state, &baseline_plan, sim, kinds);
        let search_after = apply_day(&search_state, &search_plan, sim, kinds);
        let shadow_after = apply_day(&search_state, &shadow_plan, sim, kinds);
        let old_baseline = daily_count(&baseline_state, day);
        let old_search = daily_count(&search_state, day);
        let changed_actions =
            baseline_plan.actions.iter().zip(&search_plan.actions).filter(|(a, b)| a != b).count();

        println!(
            "day={} baseline_daily={} search_daily={} shadow_baseline_on_search_state={} \
             changed_actions={} baseline_start=[{}] search_start=[{}] \
             baseline_end=[{}] search_end=[{}]",
            day + 1,
            daily_count(&baseline_after, day) - old_baseline,
            daily_count(&search_after, day) - old_search,
            daily_count(&shadow_after, day) - old_search,
            changed_actions,
            describe_agents(&baseline_state),
            describe_agents(&search_state),
            describe_agents(&baseline_after),
            describe_agents(&search_after),
        );
        baseline_state = baseline_after;
        search_state = search_after;
    }
    println!(
        "final baseline=({},{},{}) search=({},{},{}) daily_baseline={:?} daily_search={:?}",
        baseline_state.series_taken.len(),
        baseline_state.daily_varieties().iter().sum::<u32>(),
        baseline_state.agents.iter().map(engine::model::Agent::total_inventory).sum::<u32>(),
        search_state.series_taken.len(),
        search_state.daily_varieties().iter().sum::<u32>(),
        search_state.agents.iter().map(engine::model::Agent::total_inventory).sum::<u32>(),
        baseline_state.daily_varieties(),
        search_state.daily_varieties(),
    );
}
