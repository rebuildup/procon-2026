//! Vehicle composition comparison (Issue #342, team research).
//!
//! Match 1869 conditions: 32x32, 8 days, 7 agents, fuel_cap=395, day1=232 steps
//!
//! Team autoSupply suggests 6P/1S for these conditions.
//! Individual side currently uses 4P/3S.
//!
//! Tests: 4P3S vs 5P2S vs 6P1S on same board.
use bench::generator_32::{generate_large_case, LargeCaseParams, TerrainMix, SeriesPattern};
use bench::CandidateRegistry;
use engine::MatchSim;

fn evaluate(kinds_str: &str, cid: &str) -> (u32, u32, u32, u32, u32) {
    let params = LargeCaseParams {
        seed: 1869,
        n_series: 27,
        n_agents: 7,
        n_days: 8,
        day_steps_base: 200, // close to day1=232
        fuel_cap: 395,
        terrain: TerrainMix::BALANCED,
        pattern: SeriesPattern::Uniform,
    };
    let case = generate_large_case(&params).expect("generate");
    let pattern = bench::kinds::parse_kinds(kinds_str).unwrap();
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a").unwrap();
    let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).unwrap();
    let state = engine::model::MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin().build(cid).unwrap().build_solver().unwrap();
    let result = MatchSim::default().run(state.setting.clone(), solver.as_ref()).unwrap();
    (result.variety, result.daily_variety_sum, result.total_balls, result.fuel_short_count, result.illegal_action_count)
}

#[test]
fn vehicle_composition_comparison() {
    let compositions = [
        ("P,S,P,S,P,S,P", "4P3S (現行)"),
        ("P,P,P,P,S,P,S", "5P2S"),
        ("P,P,P,P,P,P,S", "6P1S (チーム推奨)"),
    ];
    let candidates = [
        "coverage:budget=official:park=far",
        "anytime:budget_ms=5000:seed=42:budget=official",
    ];

    println!("\n=== Vehicle Composition (Match 1869 conditions) ===");
    println!("32x32, 8 days, 7 agents, fuel=395, day1≈232 steps\n");
    println!("{:>20} {:>35} {:>4} {:>4} {:>4} {:>4} {:>4}",
        "composition", "candidate", "V", "D", "B", "fuel-", "illegal");

    for (kinds, label) in &compositions {
        for cid in &candidates {
            let (v, d, b, fs, il) = evaluate(kinds, cid);
            let short_cid = if cid.contains("park=far") && !cid.contains("anytime") { "park=far" } else { "anytime:5s" };
            println!("{:>20} {:>35} {:>4} {:>4} {:>4} {:>4} {:>4}",
                label, short_cid, v, d, b, fs, il);
        }
    }
}

#[test]
fn composition_across_fuel_levels() {
    let compositions = [
        ("P,S,P,S,P,S,P", "4P3S"),
        ("P,P,P,P,S,P,S", "5P2S"),
        ("P,P,P,P,P,P,S", "6P1S"),
    ];
    let cid = "anytime:budget_ms=5000:seed=42:budget=official";

    println!("\n=== Composition × Fuel (32x32, 7 agents, anytime:5s) ===");
    println!("{:>20} {:>6} {:>4} {:>4} {:>4}", "fuel_cap", "comp", "V", "D", "B");

    for fuel_cap in [100, 200, 300, 395] {
        for (kinds, label) in &compositions {
            let params = LargeCaseParams {
                seed: 42,
                n_series: 27,
                n_agents: 7,
                n_days: 5,
                day_steps_base: 83,
                fuel_cap,
                terrain: TerrainMix::BALANCED,
                pattern: SeriesPattern::Uniform,
            };
            let case = generate_large_case(&params).expect("generate");
            let pattern = bench::kinds::parse_kinds(kinds).unwrap();
            let kinds_resolved = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
            let team = domain::PlayerId::new("player-a").unwrap();
            let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds_resolved)).unwrap();
            let state = engine::model::MatchState::new(setting).unwrap();
            let solver = CandidateRegistry::builtin().build(cid).unwrap().build_solver().unwrap();
            let r = MatchSim::default().run(state.setting.clone(), solver.as_ref()).unwrap();
            println!("{fuel_cap:>20} {:>6} {:>4} {:>4} {:>4}", label, r.variety, r.daily_variety_sum, r.total_balls);
        }
    }
}
