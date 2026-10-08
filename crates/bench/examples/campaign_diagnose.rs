//! Replay a candidate with canonical MatchSim and emit daily loss diagnostics.
//! Usage: campaign_diagnose CASE_JSON CANDIDATE KINDS
use bench::{CandidateRegistry, CaseSpec};
use engine::{
    MatchSim,
    model::{ActionKind, AgentActions, MatchState, SolverContext},
};
use serde_json::json;
use std::collections::BTreeSet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected CASE_JSON CANDIDATE KINDS".into());
    }
    let case: CaseSpec = serde_json::from_str(&std::fs::read_to_string(&args[1])?)?;
    let pattern = bench::kinds::parse_kinds(&args[3])?;
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a")?;
    let mut state = MatchState::new(case.to_engine_setting_with_kinds(&team, Some(&kinds))?)?;
    let candidate = CandidateRegistry::builtin().build(&args[2])?;
    let solver = candidate.build_solver()?;
    let sim = MatchSim::new(case.sim_config()?);
    let all_series: BTreeSet<_> = state
        .setting
        .map
        .cells()
        .iter()
        .filter_map(|c| c.spot.as_ref().map(|s| s.series))
        .collect();
    while !state.is_finished() {
        let day = state.day;
        let before = state.agents.clone();
        let ctx = SolverContext::from_state(&state);
        let plan = solver.decide(&ctx);
        let normalized = bench::adapters::walk::BudgetMode::Official.apply(&ctx, plan.clone());
        let changed = plan.actions.iter().zip(&normalized.actions).filter(|(a, b)| a != b).count();
        let n = ctx.agent_ids.len();
        let mut moves = vec![0u32; n];
        let mut waits = vec![0u32; n];
        for chunk in plan.actions.chunks(n) {
            for (i, a) in chunk.iter().enumerate() {
                match a.kind {
                    ActionKind::Move(_) => moves[i] += 1,
                    ActionKind::Wait => waits[i] += 1,
                }
            }
            sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
        }
        let acquired = state.series_acquired_by_day.get(&day).cloned().unwrap_or_default();
        let missing: Vec<_> = all_series.difference(&acquired).copied().collect();
        let agents: Vec<_> = state.agents.iter().enumerate().map(|(i,a)| json!({
            "id":a.id,"kind":format!("{:?}",a.kind),"balls":a.total_inventory()-before[i].total_inventory(),
            "moves":moves[i],"wait_elements":waits[i],"fuel_start":before[i].fuel,"fuel_end":a.fuel,
            "start":[before[i].pos.col,before[i].pos.row],"end":[a.pos.col,a.pos.row]
        })).collect();
        println!(
            "{}",
            json!({"source_commit_sha":bench::source_commit_sha(),"case_id":case.case_id,"candidate_id":candidate.id(),"day":day,
            "acquired_series":acquired,"missing_series":missing,"agents":agents,"normalized_action_changes":changed,
            "illegal_action_count_cumulative":state.illegal_action_count,"fuel_short_count_cumulative":state.fuel_short_count})
        );
    }
    Ok(())
}
