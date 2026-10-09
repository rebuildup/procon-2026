//! Diagnose patrol fuel stranding (Issue #339, #342).
//!
//! Match 1810: 32x32, 7 agents (P,S,P,S,P,S,P), 5 days, daySteps=[72,75,95,78,93]
//! Match 1811: 24x24, 5 agents (P,S,P,S,P), 6 days, daySteps=[66,133,64,102,64,167]
//!
//! Tests fuel-aware supply pairing vs default pairing.
use bench::generator_32::{generate_large_case, LargeCaseParams, TerrainMix, SeriesPattern};
use bench::CandidateRegistry;
use engine::MatchSim;


fn diagnose_fuel(case_id: &str, kinds_str: &str, params: LargeCaseParams) {
    let case = generate_large_case(&params).expect("generate");
    let pattern = bench::kinds::parse_kinds(kinds_str).unwrap();
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a").unwrap();
    let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).unwrap();

    println!("\n=== {case_id} ===");
    
    for cid in [
        "coverage:budget=official:park=far",
        "coverage:budget=official:park=far:fuelaware=true",
    ] {
        let state = engine::model::MatchState::new(setting.clone()).unwrap();
        let solver = CandidateRegistry::builtin().build(cid).unwrap().build_solver().unwrap();
        let sim = MatchSim::default();
        let result = sim.run(state.setting.clone(), solver.as_ref()).unwrap();
        
        // Count stranding: patrols with fuel=0 at end of any day
        let daily = result.daily_variety_sum;
        let min_fuel = result.fuel_short_count; // proxy for fuel issues
        
        println!(
            "{cid}: V={} D={} B={} daily={:?} fuel_short={}",
            result.variety, result.daily_variety_sum, result.total_balls, daily, min_fuel
        );
    }
}

#[test]
fn fuel_stranding_diagnosis() {
    // Match 1810 conditions
    diagnose_fuel(
        "Match 1810 (32x32, 7 agents, 5 days)",
        "P,S,P,S,P,S,P",
        LargeCaseParams {
            seed: 1810,
            n_series: 27,
            n_agents: 7,
            n_days: 5,
            day_steps_base: 83,
            fuel_cap: 145,
            terrain: TerrainMix::BALANCED,
            pattern: SeriesPattern::Uniform,
        },
    );

    // Match 1811 conditions
    diagnose_fuel(
        "Match 1811 (24x24, 5 agents, 6 days)",
        "P,S,P,S,P",
        LargeCaseParams {
            seed: 1811,
            n_series: 20,
            n_agents: 5,
            n_days: 6,
            day_steps_base: 99,
            fuel_cap: 145,
            terrain: TerrainMix::BALANCED,
            pattern: SeriesPattern::Uniform,
        },
    );

    // Fuel-tight case (stress test)
    diagnose_fuel(
        "Fuel-tight (32x32, 7 agents, 5 days, cap=60)",
        "P,S,P,S,P,S,P",
        LargeCaseParams {
            seed: 999,
            n_series: 27,
            n_agents: 7,
            n_days: 5,
            day_steps_base: 83,
            fuel_cap: 60,
            terrain: TerrainMix::BALANCED,
            pattern: SeriesPattern::Uniform,
        },
    );
}
