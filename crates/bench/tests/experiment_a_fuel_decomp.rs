//! Experiment A: fuel-aware pairing condition decomposition (Issue #342).
//!
//! Decompose the effect of fuel-aware pairing across:
//! - vehicle count (5, 7, 9)
//! - Patrol/Supply ratio (P,S,P,S,P / P,P,P,S,P / P,P,P,P,S)
//! - fuel cap (official-range tight, 100, 145, 200)
//! - series count (15, 20, 27)
//! - days (3, 5, 7)
//!
//! All cases are 32x32 (`generate_large_case` has no size knob) — the earlier
//! "map size" axis was really a series-count axis.
//!
//! Official constraint: `fuelLimits >= daySteps[0]`. The earlier cap=60 point
//! violated that floor at seed 42 (`daySteps[0]=75`) and its numbers are not
//! admissible evidence; the sweep now starts at the tightest official-valid
//! cap (= `daySteps[0]`).
//!
//! Records V/D/B for each condition with/without fuelaware.
//! NO case ID branching - use board features for safety conditions.
use bench::CandidateRegistry;
use bench::generator_32::{LargeCaseParams, SeriesPattern, TerrainMix, generate_large_case};
use engine::MatchSim;

fn evaluate(params: &LargeCaseParams, kinds_str: &str, fuel_aware: bool) -> (u32, u32, u32, u32) {
    let case = generate_large_case(params).expect("generate");
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
    let state = engine::model::MatchState::new(setting).unwrap();
    let cid = if fuel_aware {
        "coverage:budget=official:park=far:fuelaware=true"
    } else {
        "coverage:budget=official:park=far"
    };
    let solver = CandidateRegistry::builtin().build(cid).unwrap().build_solver().unwrap();
    let sim = MatchSim::default();
    let result = sim.run(state.setting.clone(), solver.as_ref()).unwrap();
    (result.variety, result.daily_variety_sum, result.total_balls, result.fuel_short_count)
}

#[test]
#[allow(
    clippy::cast_possible_wrap,
    reason = "diagnostic sweep prints signed dD deltas of bounded V/D/B scores"
)]
fn experiment_a_decomposition() {
    println!("=== Experiment A: Fuel-Aware Pairing Decomposition ===");
    println!("Note: MatchSim-based, provisional until #342 parity fix\n");

    // Base: 32x32, 7 agents, 5 days, cap=145
    let base = LargeCaseParams {
        seed: 42,
        n_series: 27,
        n_agents: 7,
        n_days: 5,
        day_steps_base: 83,
        fuel_cap: 145,
        terrain: TerrainMix::BALANCED,
        pattern: SeriesPattern::Uniform,
    };

    let kinds_7 = "P,S,P,S,P,S,P";

    // 1. Vehicle count variation
    println!("--- Vehicle Count (32x32, 5 days, cap=145) ---");
    println!("{:>8} {:>20} {:>20} {:>8}", "agents", "base V/D/B", "fuelaware V/D/B", "dD");
    for n_agents in [5, 7, 8] {
        let params = LargeCaseParams { n_agents, ..base };
        let kinds = match n_agents {
            5 => "P,S,P,S,P",
            7 => "P,S,P,S,P,S,P",
            _ => "P,S,P,S,P,S,P,P",
        };
        let (v1, d1, b1, _) = evaluate(&params, kinds, false);
        let (v2, d2, b2, _) = evaluate(&params, kinds, true);
        let dd = d2 as i32 - d1 as i32;
        println!("{n_agents:>8} {v1:>5}/{d1:>5}/{b1:>5} {v2:>5}/{d2:>5}/{b2:>5} {dd:>+8}");
    }

    // 2. Patrol/Supply ratio
    println!("\n--- Patrol/Supply Ratio (32x32, 7 agents, 5 days, cap=145) ---");
    println!("{:>20} {:>20} {:>20} {:>8}", "kinds", "base V/D/B", "fuelaware V/D/B", "dD");
    for kinds in ["P,S,P,S,P,S,P", "P,P,P,S,P,S,P", "P,P,P,P,S,S,S"] {
        let (v1, d1, b1, _) = evaluate(&base, kinds, false);
        let (v2, d2, b2, _) = evaluate(&base, kinds, true);
        let dd = d2 as i32 - d1 as i32;
        println!("{kinds:>20} {v1:>5}/{d1:>5}/{b1:>5} {v2:>5}/{d2:>5}/{b2:>5} {dd:>+8}");
    }

    // 3. Fuel cap variation (official range only: fuel_cap >= daySteps[0]).
    // The old cap=60 point was below daySteps[0]=75 at seed 42 and is gone.
    let day0 = generate_large_case(&base).expect("generate").setting.day_steps[0];
    let tight_cap = u32::try_from(day0).expect("day0 fits u32");
    println!("\n--- Fuel Cap (32x32, 7 agents, 5 days; daySteps[0]={day0}) ---");
    println!("{:>8} {:>20} {:>20} {:>8}", "cap", "base V/D/B", "fuelaware V/D/B", "dD");
    for fuel_cap in [tight_cap, 100, 145, 200] {
        let params = LargeCaseParams { fuel_cap, ..base };
        let (v1, d1, b1, _) = evaluate(&params, kinds_7, false);
        let (v2, d2, b2, _) = evaluate(&params, kinds_7, true);
        let dd = d2 as i32 - d1 as i32;
        println!("{fuel_cap:>8} {v1:>5}/{d1:>5}/{b1:>5} {v2:>5}/{d2:>5}/{b2:>5} {dd:>+8}");
    }

    // 4. Map size variation (via n_series as proxy)
    println!("\n--- Series Count (32x32, 7 agents, 5 days, cap=145) ---");
    println!("{:>8} {:>20} {:>20} {:>8}", "series", "base V/D/B", "fuelaware V/D/B", "dD");
    for n_series in [15, 20, 27] {
        let params = LargeCaseParams { n_series, ..base };
        let (v1, d1, b1, _) = evaluate(&params, kinds_7, false);
        let (v2, d2, b2, _) = evaluate(&params, kinds_7, true);
        let dd = d2 as i32 - d1 as i32;
        println!("{n_series:>8} {v1:>5}/{d1:>5}/{b1:>5} {v2:>5}/{d2:>5}/{b2:>5} {dd:>+8}");
    }

    // 5. Days variation
    println!("\n--- Days (32x32, 7 agents, cap=145) ---");
    println!("{:>8} {:>20} {:>20} {:>8}", "days", "base V/D/B", "fuelaware V/D/B", "dD");
    for n_days in [3, 5, 7] {
        let params = LargeCaseParams { n_days, ..base };
        let (v1, d1, b1, _) = evaluate(&params, kinds_7, false);
        let (v2, d2, b2, _) = evaluate(&params, kinds_7, true);
        let dd = d2 as i32 - d1 as i32;
        println!("{n_days:>8} {v1:>5}/{d1:>5}/{b1:>5} {v2:>5}/{d2:>5}/{b2:>5} {dd:>+8}");
    }
}
