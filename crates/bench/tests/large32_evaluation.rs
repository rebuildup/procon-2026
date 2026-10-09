//! 32x32 large-map evaluation suite (Issue #331).
//!
//! Compares step-greedy / park=far / tour candidates on 32x32 fixtures with
//! 27 series across multiple terrain and distribution patterns.
//!
//! Run: `cargo test -p bench --test large32_evaluation --offline --locked -- --nocapture`
use bench::generator_32::{large32_suite, generate_large_case};
use bench::{CandidateRegistry};
use engine::MatchSim;
use engine::model::MatchState;


fn evaluate_candidate(case: &bench::CaseSpec, candidate_id: &str) -> (u32, u32, u32, bool) {
    let pattern = bench::kinds::parse_kinds("P,S,P,S,P,S,P").unwrap();
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a").unwrap();
    let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).unwrap();
    let state = MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin()
        .build(candidate_id)
        .expect("candidate registered")
        .build_solver()
        .unwrap();
    let sim = MatchSim::default();
    let result = sim.run(state.setting.clone(), solver.as_ref()).unwrap();
    (
        result.variety,
        result.daily_variety_sum,
        result.total_balls,
        result.illegal_action_count == 0 && result.fuel_short_count == 0,
    )
}

#[test]
fn large32_suite_evaluation() {
    let suite = large32_suite();
    let candidates = [
        ("greedy", "coverage:budget=official:park=none"),
        ("park=far", "coverage:budget=official:park=far"),
        ("tour", "tour:budget=official"),
    ];

    println!("=== 32x32 Evaluation Suite (Issue #333) ===");
    println!("{:8} {:20} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>5}",
        "case", "pattern", "V_g", "D_g", "B_g", "V_p", "D_p", "B_p", "V_t", "D_t", "B_t", "valid");

    for (i, params) in suite.iter().enumerate() {
        let case = generate_large_case(params).expect("generate");
        let mut results = Vec::new();
        for (_, cid) in &candidates {
            let (v, d, b, valid) = evaluate_candidate(&case, cid);
            results.push((v, d, b, valid));
        }
        let all_valid = results.iter().all(|r| r.3);
        println!(
            "case-{:<3} {:20} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>5}",
            i,
            format!("{:?}", params.pattern),
            results[0].0, results[0].1, results[0].2,
            results[1].0, results[1].1, results[1].2,
            results[2].0, results[2].1, results[2].2,
            if all_valid { "ok" } else { "FAIL" }
        );
    }

    // Summary: how many cases does each candidate win on D?
    let mut wins = [0usize; 3];
    let mut losses = [0usize; 3];
    for (i, params) in suite.iter().enumerate() {
        let case = generate_large_case(params).expect("generate");
        let mut ds = Vec::new();
        for (_, cid) in &candidates {
            let (_, d, _, _) = evaluate_candidate(&case, cid);
            ds.push(d);
        }
        let best = ds.iter().max().unwrap();
        for (j, &d) in ds.iter().enumerate() {
            if d == *best {
                wins[j] += 1;
            } else {
                losses[j] += 1;
            }
        }
    }
    println!("\n=== D-axis wins (out of {} cases) ===", suite.len());
    for (j, (name, _)) in candidates.iter().enumerate() {
        println!("  {:10} wins={} losses={}", name, wins[j], losses[j]);
    }
}
