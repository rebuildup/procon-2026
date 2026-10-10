//! Diagnose patrol fuel stranding (Issue #339, #342).
//!
//! Shape-only stand-ins for live Match 1810 / 1811. `generate_large_case`
//! hardcodes a 32x32 board, so **every case here is 32x32** — including the
//! 1811 stand-in (the live 1811 map was 24x24). These values must NOT be
//! cited as 24x24 evidence. daySteps are generator-produced around
//! `day_steps_base`, not the live match clocks.
//!
//! - Match 1810 shape: 32x32, 7 agents (P,S,P,S,P,S,P), 5 days
//! - Match 1811 shape: 32x32, 5 agents (P,S,P,S,P), 6 days (live was 24x24)
//! - Fuel-tight: 32x32, 7 agents, 5 days, tight but official-valid fuel cap
//!
//! Official constraint: `fuelLimits >= daySteps[0]`. The earlier cap=60
//! stress case violated that floor and is excluded from evidence.
//!
//! Tests fuel-aware supply pairing vs default pairing.
use bench::CandidateRegistry;
use bench::generator_32::{LargeCaseParams, SeriesPattern, TerrainMix, generate_large_case};
use engine::MatchSim;

fn diagnose_fuel(case_id: &str, kinds_str: &str, params: LargeCaseParams) {
    let case = generate_large_case(&params).expect("generate");
    let day0 = case.setting.day_steps[0];
    assert!(
        i64::from(case.setting.fuel_limits) >= i64::from(day0),
        "official constraint fuelLimits >= daySteps[0] violated: fuel={} day0={day0}",
        case.setting.fuel_limits
    );
    let pattern = bench::kinds::parse_kinds(kinds_str).unwrap();
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a").unwrap();
    let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).unwrap();

    println!("\n=== {case_id} (32x32, daySteps[0]={day0}) ===");

    for cid in
        ["coverage:budget=official:park=far", "coverage:budget=official:park=far:fuelaware=true"]
    {
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
    // Match 1810 shape (32x32).
    diagnose_fuel(
        "Match 1810 shape (32x32, 7 agents, 5 days)",
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

    // Match 1811 shape. Live 1811 was 24x24; this generator is 32x32 only,
    // so the label says 32x32 and the numbers are NOT 24x24 evidence.
    diagnose_fuel(
        "Match 1811 shape (32x32 stand-in for live 24x24, 5 agents, 6 days)",
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

    // Fuel-tight case (stress test). cap=100 sits just above the seed-999
    // daySteps[0] floor and is official-valid (the old cap=60 case was not).
    diagnose_fuel(
        "Fuel-tight (32x32, 7 agents, 5 days, cap=100)",
        "P,S,P,S,P,S,P",
        LargeCaseParams {
            seed: 999,
            n_series: 27,
            n_agents: 7,
            n_days: 5,
            day_steps_base: 83,
            fuel_cap: 100,
            terrain: TerrainMix::BALANCED,
            pattern: SeriesPattern::Uniform,
        },
    );
}
