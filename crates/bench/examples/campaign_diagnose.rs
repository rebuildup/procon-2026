//! Replay a candidate with canonical `MatchSim` and emit daily loss diagnostics.
//!
//! Usage: `campaign_diagnose CASE_SOURCE CANDIDATE KINDS`
//!
//! `CASE_SOURCE` is one of
//!   - `fixture:<name>` — a bundled fixture (e.g. `fixture:small-8x8`),
//!   - `generated:<sel>` — one generated case (e.g. `generated:open:seed=1:days=3`),
//!   - a filesystem path to a `CaseSpec` JSON file (e.g. `crates/bench/tests/fixtures/arena-701.json`).
//!
//! Output is JSON Lines on stdout: one `kind:"day"` record per simulated day
//! (the day's acquired series set, its repeats, and per-agent movement / fuel
//! evidence), then one `kind:"case_summary"` record carrying the official
//! score tiers plus a classified `stop_reasons` list. Together these are the
//! Issue #312 「日別取得種類集合と停止原因」 diagnostic axis: `runs.jsonl`
//! holds the rankable score, this stream explains *why* it landed there.
//!
//! `stop_reasons` taxonomy (Issue #311 「未取得系列・車両停止・燃料不足・
//! 重複・ステップ切詰めなどの原因を区別」):
//!   - `complete` — every map series was acquired;
//!   - `series_unreached` — map series left unacquired (未取得系列);
//!   - `fuel_exhausted` — illegal fuel-short actions, or an agent parked at
//!     zero fuel (燃料不足);
//!   - `vehicle_stalled` — an agent took zero moves on a day that still had
//!     unreached series (車両停止);
//!   - `series_repeats` — a day acquired only series already taken earlier
//!     (重複);
//!   - `plan_normalized` — the pre-submission normalize rewrote the
//!     candidate's plan (ステップ切詰め / illegal tail). For a
//!     `budget=official` candidate this must stay 0: a non-zero count is the
//!     departure-cell charging vs normalize inconsistency check that Issue
//!     #312's acceptance list asks for (see #255 / #302 / #304).
use bench::{CandidateRegistry, CaseSpec};
use engine::{
    MatchSim,
    model::{ActionKind, AgentActions, MatchState, Solver, SolverContext},
};
use serde_json::json;
use std::collections::BTreeSet;

/// Per-day facts folded into the final stop-reason classification.
#[derive(Debug, Default)]
struct DayEvidence {
    daily_acquired_series: Vec<Vec<u32>>,
    normalized_action_changes_total: usize,
    repeat_only_days: Vec<u32>,
    zero_acquisition_days: Vec<u32>,
    stalled_agent_days: Vec<serde_json::Value>,
    fuel_exhausted_agent_days: Vec<serde_json::Value>,
    agent_moves: Vec<u32>,
    agent_waits: Vec<u32>,
    agent_fuel_start: Vec<u32>,
    agent_fuel_end: Vec<u32>,
    agent_balls: Vec<u32>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected CASE_SOURCE CANDIDATE KINDS".into());
    }
    let case = load_case(&args[1])?;
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
    let mut acc = DayEvidence::default();
    let case_id = case.case_id.clone();
    let candidate_id = candidate.id();
    while !state.is_finished() {
        run_day(&sim, solver.as_ref(), &mut state, &all_series, &case_id, &candidate_id, &mut acc);
    }
    print_summary(&case_id, &candidate_id, &kinds, &state, &all_series, &acc);
    Ok(())
}

/// Plan and simulate one day: accumulate [`DayEvidence`], print the `day` line.
#[allow(clippy::too_many_arguments)]
fn run_day(
    sim: &MatchSim,
    solver: &dyn Solver,
    state: &mut MatchState,
    all_series: &BTreeSet<u32>,
    case_id: &str,
    candidate_id: &str,
    acc: &mut DayEvidence,
) {
    let day = state.day;
    let before = state.agents.clone();
    let ctx = SolverContext::from_state(state);
    let plan = solver.decide(&ctx);
    // Pre-submission normalize: `budget=official` candidates are already
    // truncated inside `decide`, so a non-zero count here means the plan the
    // simulator is about to run is *not* what the server would have accepted
    // (Issue #312 acceptance: departure-cell charging vs normalize
    // consistency).
    let normalized = bench::adapters::walk::BudgetMode::Official.apply(&ctx, plan.clone());
    let changed = plan.actions.iter().zip(&normalized.actions).filter(|(a, b)| a != b).count();
    acc.normalized_action_changes_total += changed;
    let n = ctx.agent_ids.len();
    if acc.agent_moves.is_empty() {
        acc.agent_moves = vec![0; n];
        acc.agent_waits = vec![0; n];
        acc.agent_fuel_start = before.iter().map(|a| a.fuel).collect();
        acc.agent_fuel_end = vec![0; n];
        acc.agent_balls = vec![0; n];
    }
    let mut moves = vec![0u32; n];
    let mut waits = vec![0u32; n];
    for chunk in plan.actions.chunks(n) {
        for (i, a) in chunk.iter().enumerate() {
            match a.kind {
                ActionKind::Move(_) => moves[i] += 1,
                ActionKind::Wait => waits[i] += 1,
            }
        }
        sim.step(state, &AgentActions::new(chunk.to_vec()));
    }
    for i in 0..n {
        acc.agent_moves[i] += moves[i];
        acc.agent_waits[i] += waits[i];
        acc.agent_fuel_end[i] = state.agents[i].fuel;
        acc.agent_balls[i] = state.agents[i].total_inventory();
    }

    let acquired = state.series_acquired_by_day.get(&day).cloned().unwrap_or_default();
    let mut taken_before: BTreeSet<u32> = BTreeSet::new();
    for d in 0..day {
        if let Some(prev) = state.series_acquired_by_day.get(&d) {
            taken_before.extend(prev.iter().copied());
        }
    }
    let repeats: Vec<u32> = acquired.intersection(&taken_before).copied().collect();
    // Per-day `missing_series` is "not taken on *this* day" (the tier-2
    // view); `unreached_series` is "never taken as of the end of this day"
    // (the tier-1 view).
    let missing: Vec<u32> = all_series.difference(&acquired).copied().collect();
    let unreached: Vec<u32> = all_series.difference(&state.series_taken).copied().collect();
    let mut day_flags: Vec<&str> = Vec::new();
    if changed > 0 {
        day_flags.push("plan_normalized");
    }
    if acquired.is_empty() {
        acc.zero_acquisition_days.push(day);
        day_flags.push("zero_acquisition");
    } else if acquired.iter().all(|s| taken_before.contains(s)) {
        acc.repeat_only_days.push(day);
        day_flags.push("repeat_only");
    }
    for (i, a) in state.agents.iter().enumerate() {
        if moves[i] == 0 && !unreached.is_empty() {
            acc.stalled_agent_days.push(json!({"day": day, "agent": i, "waits": waits[i]}));
            if day_flags.last() != Some(&"vehicle_stalled") {
                day_flags.push("vehicle_stalled");
            }
        }
        if a.fuel == 0 {
            acc.fuel_exhausted_agent_days.push(json!({"day": day, "agent": i}));
            if day_flags.last() != Some(&"fuel_exhausted") {
                day_flags.push("fuel_exhausted");
            }
        }
    }
    let agents: Vec<_> = state.agents.iter().enumerate().map(|(i,a)| json!({
        "id":a.id,"kind":format!("{:?}",a.kind),"balls":a.total_inventory()-before[i].total_inventory(),
        "moves":moves[i],"wait_elements":waits[i],"fuel_start":before[i].fuel,"fuel_end":a.fuel,
        "start":[before[i].pos.col,before[i].pos.row],"end":[a.pos.col,a.pos.row]
    })).collect();
    acc.daily_acquired_series.push(acquired.iter().copied().collect());
    println!(
        "{}",
        json!({"kind":"day","source_commit_sha":bench::source_commit_sha(),"case_id":case_id,
        "candidate_id":candidate_id,"day":day,
        "acquired_series":acquired,"repeat_series":repeats,
        "missing_series":missing,"unreached_series":unreached,
        "agents":agents,"day_flags":day_flags,
        "normalized_action_changes":changed,
        "illegal_action_count_cumulative":state.illegal_action_count,
        "fuel_short_count_cumulative":state.fuel_short_count})
    );
}

/// Classify why the match landed where it did and print the `case_summary` line.
fn print_summary(
    case_id: &str,
    candidate_id: &str,
    kinds: &[engine::model::AgentKind],
    state: &MatchState,
    all_series: &BTreeSet<u32>,
    acc: &DayEvidence,
) {
    let missing_series: Vec<u32> = all_series.difference(&state.series_taken).copied().collect();
    let complete = missing_series.is_empty();
    let fuel_exhausted = state.fuel_short_count > 0
        || !acc.fuel_exhausted_agent_days.is_empty()
        || state.agents.iter().any(|a| a.fuel == 0);
    let mut stop_reasons: Vec<&str> = Vec::new();
    if complete {
        stop_reasons.push("complete");
    } else {
        stop_reasons.push("series_unreached");
        if fuel_exhausted {
            stop_reasons.push("fuel_exhausted");
        }
        if !acc.stalled_agent_days.is_empty() {
            stop_reasons.push("vehicle_stalled");
        }
        if !acc.repeat_only_days.is_empty() {
            stop_reasons.push("series_repeats");
        }
        if acc.normalized_action_changes_total > 0 {
            stop_reasons.push("plan_normalized");
        }
    }
    println!(
        "{}",
        json!({"kind":"case_summary","source_commit_sha":bench::source_commit_sha(),
        "case_id":case_id,"candidate_id":candidate_id,
        "kinds":kinds.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
        "num_days":state.num_days(),
        "variety":state.series_taken.len(),
        "daily_variety_sum":state.daily_varieties().iter().sum::<u32>(),
        "total_balls":acc.agent_balls.iter().sum::<u32>(),
        "daily_acquired_series":acc.daily_acquired_series,
        "missing_series":missing_series,
        "stop_reasons":stop_reasons,
        "evidence":{
            "last_acquisition_at":state.series_first_taken_at.values().copied().max(),
            "illegal_action_count":state.illegal_action_count,
            "fuel_short_count":state.fuel_short_count,
            "normalized_action_changes_total":acc.normalized_action_changes_total,
            "zero_acquisition_days":acc.zero_acquisition_days,
            "repeat_only_days":acc.repeat_only_days,
            "stalled_agent_days":acc.stalled_agent_days,
            "fuel_exhausted_agent_days":acc.fuel_exhausted_agent_days,
            "per_agent_moves":acc.agent_moves,
            "per_agent_waits":acc.agent_waits,
            "per_agent_fuel_start":acc.agent_fuel_start,
            "per_agent_fuel_end":acc.agent_fuel_end,
            "per_agent_balls":acc.agent_balls
        }})
    );
}

/// Resolve a `CASE_SOURCE` token to a validated [`CaseSpec`].
fn load_case(source: &str) -> Result<CaseSpec, Box<dyn std::error::Error>> {
    if let Some(name) = source.strip_prefix("fixture:") {
        return Ok(CaseSpec::from_bundled_fixture(name)?);
    }
    if let Some(selection) = source.strip_prefix("generated:") {
        let params = bench::generator::parse_selection(selection)?;
        let corpus = bench::generator::build_corpus(&params)?;
        return corpus
            .cases
            .into_iter()
            .next()
            .ok_or_else(|| format!("generated selection {selection:?} produced no case").into());
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(source)?)?)
}
