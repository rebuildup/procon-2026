//! Trace per-step target selection of the coverage candidate.
use bench::{CandidateRegistry, CaseSpec};
use engine::{
    MatchSim,
    model::{ActionKind, AgentActions, MatchState, SolverContext},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected CASE_JSON CANDIDATE KINDS".into());
    }
    let case: CaseSpec = if let Some(name) = args[1].strip_prefix("fixture:") {
        CaseSpec::from_bundled_fixture(name)?
    } else {
        serde_json::from_str(&std::fs::read_to_string(&args[1])?)?
    };
    let pattern = bench::kinds::parse_kinds(&args[3])?;
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a")?;
    let mut state = MatchState::new(case.to_engine_setting_with_kinds(&team, Some(&kinds))?)?;
    let candidate = CandidateRegistry::builtin().build(&args[2])?;
    let solver = candidate.build_solver()?;
    let sim = MatchSim::new(case.sim_config()?);
    let map = state.setting.map.clone();
    // print spot layout
    let mut spots = Vec::new();
    for c in map.cells() {
        if let Some(spot) = &c.spot {
            spots.push(format!("s{}@({},{})", spot.series, c.coord.col, c.coord.row));
        }
    }
    println!("spots: {}", spots.join(" "));
    while !state.is_finished() {
        let day = state.day;
        let ctx = SolverContext::from_state(&state);
        let plan = solver.decide(&ctx);
        let n = ctx.agent_ids.len();
        for (step, chunk) in plan.actions.chunks(n).enumerate() {
            let before: Vec<String> = state
                .agents
                .iter()
                .map(|a| {
                    format!("[{},{},f{},b{}]", a.pos.col, a.pos.row, a.fuel, a.total_inventory())
                })
                .collect();
            let acts: Vec<String> = chunk
                .iter()
                .map(|a| match a.kind {
                    ActionKind::Wait => "W".into(),
                    ActionKind::Move(d) => format!("M{d:?}"),
                })
                .collect();
            sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
            let acq: Vec<u32> = state
                .series_acquired_by_day
                .get(&day)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            println!("d{day}s{step:02} {} -> {:?} acq={:?}", before.join(" "), acts, acq);
        }
    }
    let acq_all: std::collections::BTreeSet<u32> =
        state.series_acquired_by_day.values().flatten().copied().collect();
    println!("final acquired series: {acq_all:?}");
    Ok(())
}
