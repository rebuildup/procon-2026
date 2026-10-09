//! Match 1857 composition sweep (provisional, MatchSim-based).
//!
//! Live shape: 16x16, 4 agents (P,S,P,S), 4 days, daySteps=[61,39,104,39],
//! V=12 / D=48 (MAX) / B=74; 1st place B=114. All Patrols active, fuel always
//! full, no stranding.
//!
//! Question: which of `P,S,P,S` / `P,P,P,S` / `P,P,S,S` raises B past 74
//! without degrading V or D?
//!
//! # Provisional
//!
//! Everything here is **MatchSim-based offline evaluation**, not live-server
//! parity. `park=far` is a no-op on this shape (spreading window needs edge
//! at least 24 and day pairs at least 70 steps; Match 1857 is 16x16 with
//! daySteps=[61,39,104,39]), so both candidate IDs should agree on park
//! behaviour and differ only where `fuelaware` changes supply pairing.
//!
//! The board is a purpose-built fixture that reproduces the *shape* of match
//! 1857 (size / agents / days / daySteps / 12 series), not its exact map —
//! spot layout is a 6x4 lattice of 24 spots (2 per brand) on plain terrain.
//!
//! Two stock regimes are reported:
//!
//! - `stocks=2` — stock headroom, the regime live 1st place implies
//!   (B=114 > 74 leaves unharvested balls on the board). Extra Patrols can
//!   double-dip a spot in one day (`SPOT_DAILY_PICK_PER_AGENT = 1` per agent).
//! - `stocks=1` — stock-tight: every spot yields one ball per day no matter
//!   how many Patrols visit. Calibrates nearer the live P,S,P,S B=74.

use bench::CandidateRegistry;
use bench::case::{CaseSource, CaseSpec};
use bench::kinds::{parse_kinds, resolve_kinds};
use engine::MatchSim;
use engine::model::MatchState;
use protocol::match_config::{CellType, Map, MatchSetting as WireSetting, Spot as WireSpot};

/// Live match-1857 day clock. Official `W+H ..= 4(W+H)` range on 16x16 is
/// `32..=128`, so every entry is legal.
const DAY_STEPS: [i32; 4] = [61, 39, 104, 39];

/// Live match-1857 team size.
const N_AGENTS: usize = 4;

/// Live match-1857 variety (12 brands on the board).
const N_BRANDS: usize = 12;

/// Two spots per brand -> 24 spots on a 6x4 lattice.
const SPOTS_PER_BRAND: usize = 2;

/// Q14 fuel cap: 3 x day-1 steps (top of the official `1..=3` band). Matches
/// "fuel always full" once Supply keeps contact — no stranding expected.
const FUEL_LIMITS: i32 = 3 * 61;

/// Match 1857 live baseline (P,S,P,S).
const LIVE_V: u32 = 12;
const LIVE_D: u32 = 48;
const LIVE_B: u32 = 74;
const TARGET_B: u32 = 114;

/// Compositions to compare (4 slots each).
const COMPOSITIONS: &[(&str, &str)] = &[
    ("P,S,P,S", "live baseline (2 Patrol / 2 Supply)"),
    ("P,P,P,S", "3 Patrol / 1 Supply"),
    ("P,P,S,S", "2 Patrol / 2 Supply staggered"),
];

/// Candidates named in the task.
const CANDIDATES: &[&str] =
    &["coverage:budget=official:park=far", "coverage:budget=official:park=far:fuelaware=true"];

/// Stock regimes: (stocks per spot, label).
const STOCK_REGIMES: &[(i32, &str)] = &[
    (2, "stocks=2 (headroom — live 1st place implies unharvested balls)"),
    (1, "stocks=1 (stock-tight — one ball per spot per day, any team)"),
];

/// Row-major cell index on a 16-wide board.
fn cell(x: i32, y: i32) -> i32 {
    y * 16 + x
}

/// 16x16 fixture matching match 1857's *shape*.
///
/// Terrain is all plain so reachability never masks the composition effect;
/// the mountain cross from `medium-16x16` is omitted on purpose. Spots sit on
/// a 6x4 lattice (columns 2/4/7/9/12/14, rows 2/5/10/13) so every brand has
/// two well-separated instances and a 39-step day can still cover all 12
/// series with two collectors.
fn match_1857_case(stocks: i32) -> CaseSpec {
    let cells = vec![vec![CellType::TYPE_0; 16]; 16];

    let xs = [2, 4, 7, 9, 12, 14];
    let ys = [2, 5, 10, 13];
    let mut spots = Vec::with_capacity(N_BRANDS * SPOTS_PER_BRAND);
    let mut index = 0usize;
    for &y in &ys {
        for &x in &xs {
            spots.push(WireSpot {
                brand: i32::try_from(index % N_BRANDS).expect("brand fits"),
                pos: cell(x, y),
                stocks,
            });
            index += 1;
        }
    }
    assert_eq!(spots.len(), 24);

    // Four starts near the interior corners, clear of the spot lattice.
    let agents = vec![cell(1, 1), cell(14, 1), cell(1, 14), cell(14, 14)];

    let setting = WireSetting {
        starts_at: 0,
        day_seconds: vec![60; 4],
        day_steps: DAY_STEPS.to_vec(),
        map: Map { height: 16, width: 16, cells },
        spots,
        agents,
        fuel_limits: FUEL_LIMITS,
        players: 1,
        busy_threshold: 1,
        jammed_threshold: 2,
    };

    CaseSpec {
        case_id: format!("generated:match-1857-shape:16x16:4agents:4days:stocks={stocks}"),
        seed: Some(1857),
        generation_parameters: Some(serde_json::json!({
            "archetype": "match-1857-shape",
            "match": 1857,
            "width": 16,
            "height": 16,
            "n_agents": N_AGENTS,
            "n_days": 4,
            "day_steps": DAY_STEPS,
            "n_brands": N_BRANDS,
            "spots_per_brand": SPOTS_PER_BRAND,
            "spot_stocks": stocks,
            "fuel_limits": FUEL_LIMITS,
            "note": "shape fixture, not the live board; provisional MatchSim evidence",
        })),
        source: CaseSource::Generated,
        setting,
    }
}

/// Returns (`variety`, `daily_variety_sum`, `total_balls`, `fuel_short_count`).
fn evaluate(case: &CaseSpec, kinds_str: &str, candidate_id: &str) -> (u32, u32, u32, u32) {
    let pattern = parse_kinds(kinds_str).expect("kinds parse");
    let kinds = resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a").expect("valid team id");
    let setting = case
        .to_engine_setting_with_kinds(&team, Some(&kinds))
        .expect("case converts with this composition");
    let sim_config = case.sim_config().expect("case declares a sim config");
    let solver = CandidateRegistry::builtin()
        .build(candidate_id)
        .expect("candidate registered")
        .build_solver()
        .expect("solver builds");
    let state = MatchState::new(setting).expect("validated setting");
    let sim = MatchSim::new(sim_config);
    let result = sim.run(state.setting.clone(), solver.as_ref()).expect("sim runs");
    (result.variety, result.daily_variety_sum, result.total_balls, result.fuel_short_count)
}

/// One stock regime x composition x candidate measurement.
struct Row {
    kinds: &'static str,
    candidate: &'static str,
    v: u32,
    d: u32,
    b: u32,
    fuel_short: u32,
}

fn sweep_regime(stocks: i32) -> Vec<Row> {
    let case = match_1857_case(stocks);
    case.sim_config().expect("fixture sim config is valid");
    assert_eq!(
        case.day_steps(),
        DAY_STEPS.iter().map(|&s| u32::try_from(s).unwrap()).collect::<Vec<_>>()
    );
    assert_eq!(case.setting.agents.len(), N_AGENTS);
    assert_eq!(case.setting.map.height, 16);
    assert_eq!(case.setting.map.width, 16);
    assert_eq!(case.setting.spots.len(), N_BRANDS * SPOTS_PER_BRAND);

    for brand in 0..N_BRANDS {
        let hits =
            case.setting.spots.iter().filter(|s| s.brand == i32::try_from(brand).unwrap()).count();
        assert_eq!(hits, SPOTS_PER_BRAND, "brand {brand} has {hits} spots");
    }

    let mut rows = Vec::new();
    for (kinds, _note) in COMPOSITIONS {
        for cid in CANDIDATES {
            let (v, d, b, fuel_short) = evaluate(&case, kinds, cid);
            rows.push(Row { kinds, candidate: cid, v, d, b, fuel_short });
        }
    }
    rows
}

fn best_at_max_vd(rows: &[Row]) -> Option<(String, u32, u32, u32)> {
    let mut best: Option<(String, u32, u32, u32)> = None;
    for row in rows {
        if row.v < LIVE_V || row.d < LIVE_D || row.fuel_short > 0 {
            continue;
        }
        let better = best.as_ref().is_none_or(|(_, _, _, bb)| row.b > *bb);
        if better {
            best = Some((format!("{} / {}", row.kinds, row.candidate), row.v, row.d, row.b));
        }
    }
    best
}

#[test]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "diagnostic sweep prints bounded board scores"
)]
fn match_1857_composition_sweep() {
    println!("=== Match 1857 composition sweep (PROVISIONAL / MatchSim-based) ===");
    println!(
        "board: 16x16, 4 agents, 4 days, daySteps={DAY_STEPS:?}, {N_BRANDS} brands x {SPOTS_PER_BRAND} spots, fuel={FUEL_LIMITS}",
    );
    println!("live:  V={LIVE_V} D={LIVE_D} (MAX) B={LIVE_B} | 1st B={TARGET_B}");
    println!("note:  park=far is a no-op here (edge 16 < 24; day pairs < 70)");
    println!();

    let mut headroom_rows: Vec<Row> = Vec::new();

    for (stocks, regime_label) in STOCK_REGIMES {
        println!("--- {regime_label} ---");
        println!(
            "{:>10} {:>28} {:>4} {:>4} {:>4} {:>6}",
            "kinds", "candidate", "V", "D", "B", "fuel-"
        );
        let rows = sweep_regime(*stocks);
        for row in &rows {
            let short =
                row.candidate.strip_prefix("coverage:budget=official:").unwrap_or(row.candidate);
            println!(
                "{:>10} {:>28} {:>4} {:>4} {:>4} {:>6}",
                row.kinds, short, row.v, row.d, row.b, row.fuel_short
            );
        }

        println!("composition effect on B (avg over candidates):");
        for (kinds, note) in COMPOSITIONS {
            let subset: Vec<_> = rows.iter().filter(|r| r.kinds == *kinds).collect();
            let avg_b = subset.iter().map(|r| r.b).sum::<u32>() / subset.len() as u32;
            let min_v = subset.iter().map(|r| r.v).min().unwrap();
            let min_d = subset.iter().map(|r| r.d).min().unwrap();
            let db = avg_b as i32 - LIVE_B as i32;
            println!(
                "{kinds:>10}  avg B={avg_b:>3} (d vs live {db:+})  min V={min_v} min D={min_d}  — {note}"
            );
        }
        match best_at_max_vd(&rows) {
            Some((name, v, d, b)) => {
                println!("best at MAX V/D: {name} -> V={v} D={d} B={b}");
            }
            None => println!("no row kept V={LIVE_V} D={LIVE_D} without fuel shorts"),
        }
        println!();

        if *stocks == 2 {
            headroom_rows = rows;
        }
    }

    // Primary success criteria, evaluated on the headroom regime (live 1st
    // place B=114 proves unharvested balls exist on the real board).
    println!("--- success check (stocks=2 headroom regime) ---");
    match best_at_max_vd(&headroom_rows) {
        Some((name, v, d, b)) => {
            println!("best V/D-safe composition: {name} -> V={v} D={d} B={b}");
            if b > LIVE_B {
                println!(
                    "SUCCESS (provisional): B={b} > live {LIVE_B} while V={v} (>= {LIVE_V}) and D={d} (>= {LIVE_D})"
                );
            } else {
                println!("no composition beat live B={LIVE_B} on this shape fixture (best B={b})");
            }
            if b >= TARGET_B {
                println!("reached 1st-place band B>={TARGET_B} (provisional)");
            }
        }
        None => println!("no composition kept V={LIVE_V} D={LIVE_D} without fuel shorts"),
    }

    let hit = headroom_rows
        .iter()
        .any(|r| r.v >= LIVE_V && r.d >= LIVE_D && r.b > LIVE_B && r.fuel_short == 0);
    assert!(
        hit,
        "expected at least one composition with B > {LIVE_B} while V>={LIVE_V} D>={LIVE_D} \
         on the headroom regime (rows: {:?})",
        headroom_rows.iter().map(|r| (r.kinds, r.v, r.d, r.b, r.fuel_short)).collect::<Vec<_>>()
    );
}

/// Park/gate sanity: on 16x16 the two candidate IDs must not diverge because
/// of `park=far` (the spreading window is closed). They may still diverge via
/// `fuelaware`. This pins the "park is a no-op" claim the sweep prints.
#[test]
fn park_far_is_a_no_op_on_match_1857_shape() {
    for stocks in [2, 1] {
        let case = match_1857_case(stocks);
        for kinds in ["P,S,P,S", "P,P,P,S", "P,P,S,S"] {
            for fuelaware in ["", ":fuelaware=true"] {
                let none_id = format!("coverage:budget=official:park=none{fuelaware}");
                let far_id = format!("coverage:budget=official:park=far{fuelaware}");
                let none = evaluate(&case, kinds, &none_id);
                let far = evaluate(&case, kinds, &far_id);
                assert_eq!(
                    none, far,
                    "park=none vs park=far must match on 16x16 \
                     (stocks={stocks}, {kinds}, {far_id}): {none:?} vs {far:?}"
                );
            }
        }
    }
}
