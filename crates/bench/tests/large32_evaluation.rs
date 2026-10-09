//! 32x32 large-map evaluation suite (Issue #331 / #333).
//!
//! Compares step-greedy / park=far / tour candidates on 32x32 fixtures with
//! 27 series across multiple terrain and distribution patterns.
//!
//! Uses the canonical [`bench::evaluate::Evaluator`] path so every
//! `(candidate, case)` pair lands as a full [`RunRecord`]: the official score
//! tuple (variety, `daily_variety_sum`, `total_balls`, `-response_time_us`)
//! plus the validity flags that decide whether a run may be ranked at all.
//!
//! Run (release profile so `response_time_us` is representative):
//! `cargo test -p bench --release --test large32_evaluation --offline --locked -- --nocapture`
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, RunRecord};
use bench::generator_32::{SeriesPattern, large32_suite, generate_large_case};
use bench::kinds::parse_kinds;
use bench::{CandidateRegistry};
use domain::PlayerId;

/// Short human label for a suite case, derived from its generation knobs.
fn case_label(i: usize, params: &bench::generator_32::LargeCaseParams) -> String {
    use bench::generator_32::TerrainMix;
    let base = match params.pattern {
        SeriesPattern::Uniform => "Uniform",
        SeriesPattern::Central => "Central",
        SeriesPattern::Peripheral => "Peripheral",
    };
    let tweak = if params.terrain.plain_pct == TerrainMix::BALANCED.plain_pct
        && params.terrain.road_pct == TerrainMix::BALANCED.road_pct
        && params.fuel_cap == 121
        && params.day_steps_base == 80
        && params.n_agents == 7
        && params.n_days == 7
    {
        String::new()
    } else if params.terrain.road_pct == TerrainMix::ROAD_HEAVY.road_pct {
        "+road-heavy".to_owned()
    } else if params.terrain.mountain_pct == TerrainMix::MOUNTAIN_HEAVY.mountain_pct {
        "+mountain-heavy".to_owned()
    } else if params.terrain.plain_pct == TerrainMix::FUEL_TIGHT.plain_pct {
        "+fuel-tight".to_owned()
    } else if params.day_steps_base == 50 {
        "+short-day".to_owned()
    } else if params.n_agents == 5 {
        "+few-agents".to_owned()
    } else if params.n_days == 10 {
        "+more-days".to_owned()
    } else {
        "+other".to_owned()
    };
    let _ = i;
    format!("{base}{tweak}")
}

/// Build an evaluator that stamps `source_commit_sha` onto every record.
fn evaluator() -> Evaluator {
    let team = PlayerId::new("player-a").expect("valid team id");
    let kinds = parse_kinds("P,S,P,S,P,S,P").expect("kinds pattern parses");
    Evaluator::new(
        EvalConfig::new(team, HarnessDeadline::default())
            .with_kinds(Some(kinds)),
    )
}

/// Per-candidate snapshot for one case: the three competitive tiers plus the
/// validity verdict. `response_time_us` is a *measurement* and is kept out of
/// the V/D/B axis on purpose.
#[derive(Debug, Clone, serde::Serialize)]
struct CandidateRow {
    candidate_id: String,
    variety: u32,
    daily_variety_sum: u32,
    total_balls: u32,
    response_time_us: u64,
    valid: bool,
    status: String,
    illegal_action_count: u32,
    fuel_short_count: u32,
    day_budget_exceeded_days: Vec<u32>,
}

impl CandidateRow {
    fn from_record(r: &RunRecord) -> Self {
        Self {
            candidate_id: r.candidate_id.clone(),
            variety: r.variety,
            daily_variety_sum: r.daily_variety_sum,
            total_balls: r.total_balls,
            response_time_us: r.response_time_us,
            valid: r.scored,
            status: r.status.to_string(),
            illegal_action_count: r.illegal_action_count,
            fuel_short_count: r.fuel_short_count,
            day_budget_exceeded_days: r.day_budget_exceeded_days.clone(),
        }
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn large32_suite_evaluation() {
    let suite = large32_suite();
    let candidates = [
        ("greedy", "coverage:budget=official:park=none"),
        ("park=far", "coverage:budget=official:park=far"),
        ("tour", "tour:budget=official"),
    ];

    // ---- generator-fix verification (Issue #333) ----
    // Uniform / Central / Peripheral must produce distinct spot layouts.
    let mut layout_sigs: Vec<Vec<i32>> = Vec::new();
    for params in suite.iter().take(3) {
        let case = generate_large_case(params).expect("generate");
        let mut positions: Vec<i32> = case.setting.spots.iter().map(|s| s.pos).collect();
        positions.sort_unstable();
        layout_sigs.push(positions);
    }
    assert_ne!(layout_sigs[0], layout_sigs[1], "Uniform and Central spot layouts must differ");
    assert_ne!(layout_sigs[0], layout_sigs[2], "Uniform and Peripheral spot layouts must differ");
    assert_ne!(layout_sigs[1], layout_sigs[2], "Central and Peripheral spot layouts must differ");
    let central_avg_dist = {
        let case = generate_large_case(&suite[1]).expect("generate");
        avg_manhattan_from_center(&case)
    };
    let peripheral_avg_dist = {
        let case = generate_large_case(&suite[2]).expect("generate");
        avg_manhattan_from_center(&case)
    };
    assert!(
        central_avg_dist < peripheral_avg_dist,
        "Central spots must sit closer to map center than Peripheral (central={central_avg_dist:.2} peripheral={peripheral_avg_dist:.2})"
    );
    println!(
        "generator-fix: spot layouts distinct; avg |dr|+|dc| central={central_avg_dist:.2} peripheral={peripheral_avg_dist:.2}"
    );

    // ---- evaluation ----
    let registry = CandidateRegistry::builtin();
    let built: Vec<_> = candidates
        .iter()
        .map(|(short, cid)| {
            let c = registry.build(cid).unwrap_or_else(|e| panic!("{cid}: {e}"));
            (short.to_string(), c)
        })
        .collect();

    let eval = evaluator();
    let mut rows: Vec<(String, Vec<CandidateRow>)> = Vec::new();

    println!("=== 32x32 Evaluation Suite (Issue #333) ===");
    println!(
        "{:8} {:20} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>9} {:>9} {:>9} {:>5}",
        "case", "pattern",
        "V_g", "D_g", "B_g", "V_p", "D_p", "B_p", "V_t", "D_t", "B_t",
        "rt_g_us", "rt_p_us", "rt_t_us", "valid"
    );

    for (i, params) in suite.iter().enumerate() {
        let case = generate_large_case(params).expect("generate");
        let label = case_label(i, params);
        let mut case_rows = Vec::new();
        for (_, cand) in &built {
            let record = eval.evaluate_one(cand, &case);
            case_rows.push(CandidateRow::from_record(&record));
        }
        let all_valid = case_rows.iter().all(|r| r.valid);
        println!(
            "case-{:<3} {:20} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>3} {:>4} {:>4} {:>9} {:>9} {:>9} {:>5}",
            i, label,
            case_rows[0].variety, case_rows[0].daily_variety_sum, case_rows[0].total_balls,
            case_rows[1].variety, case_rows[1].daily_variety_sum, case_rows[1].total_balls,
            case_rows[2].variety, case_rows[2].daily_variety_sum, case_rows[2].total_balls,
            case_rows[0].response_time_us, case_rows[1].response_time_us, case_rows[2].response_time_us,
            if all_valid { "ok" } else { "FAIL" }
        );
        rows.push((label, case_rows));
    }

    // ---- per-candidate validity breakdown ----
    println!("\n=== validity (per candidate) ===");
    for (j, (short, _)) in candidates.iter().enumerate() {
        let valid_n = rows.iter().filter(|(_, r)| r[j].valid).count();
        println!("  {short:10} valid={valid_n}/{}", suite.len());
        for (i, (label, r)) in rows.iter().enumerate() {
            if !r[j].valid {
                println!(
                    "    case-{i} {label}: status={} illegal={} fuel_short={} day_budget_over={:?}",
                    r[j].status, r[j].illegal_action_count, r[j].fuel_short_count, r[j].day_budget_exceeded_days
                );
            }
        }
    }

    // ---- win/loss on the official lexicographic order ----
    // Tier 1 variety, tier 2 daily_variety_sum, tier 3 total_balls are
    // deterministic axis metrics. Tier 4 (-response_time_us) is a *measurement*
    // and is only consulted when tiers 1-3 tie exactly; it is reported
    // separately and never folded into the axis win counts.
    let mut axis_wins = [0usize; 3];
    let mut axis_losses = [0usize; 3];
    let mut rt_tiebreak_wins = [0usize; 3];
    let mut valid_axis_wins = [0usize; 3];
    let mut valid_cases = 0usize;

    for (_, case_rows) in &rows {
        let best_axis = case_rows
            .iter()
            .map(|r| (r.variety, r.daily_variety_sum, r.total_balls))
            .max()
            .unwrap();
        for (j, r) in case_rows.iter().enumerate() {
            let axis = (r.variety, r.daily_variety_sum, r.total_balls);
            if axis == best_axis {
                axis_wins[j] += 1;
            } else {
                axis_losses[j] += 1;
            }
        }
        // Response-time tie-break only among axis-tied candidates.
        let tied: Vec<usize> = (0..3)
            .filter(|&j| {
                (case_rows[j].variety, case_rows[j].daily_variety_sum, case_rows[j].total_balls)
                    == best_axis
            })
            .collect();
        if tied.len() > 1 {
            let best_rt = tied
                .iter()
                .map(|&j| case_rows[j].response_time_us)
                .min()
                .unwrap();
            for &j in &tied {
                if case_rows[j].response_time_us == best_rt {
                    rt_tiebreak_wins[j] += 1;
                }
            }
        }
        // Valid-only axis tally (invalid runs are observed but not rankable).
        if case_rows.iter().all(|r| r.valid) {
            valid_cases += 1;
            for (j, r) in case_rows.iter().enumerate() {
                let axis = (r.variety, r.daily_variety_sum, r.total_balls);
                if axis == best_axis {
                    valid_axis_wins[j] += 1;
                }
            }
        }
    }

    println!("\n=== axis wins (V,D,B lexicographic; ties credit all; {} cases incl. invalid) ===", suite.len());
    for (j, (name, _)) in candidates.iter().enumerate() {
        println!("  {name:10} axis_wins={} axis_losses={}", axis_wins[j], axis_losses[j]);
    }
    println!("\n=== axis wins on valid-only cases ({valid_cases}/{} cases where every candidate scored) ===", suite.len());
    for (j, (name, _)) in candidates.iter().enumerate() {
        println!("  {name:10} valid_axis_wins={}", valid_axis_wins[j]);
    }
    println!("\n=== response-time tie-break wins (only among axis-tied candidates; measurement, not an axis) ===");
    for (j, (name, _)) in candidates.iter().enumerate() {
        println!("  {name:10} rt_tiebreak_wins={}", rt_tiebreak_wins[j]);
    }

    // ---- machine-readable JSON block for evidence capture ----
    let commit = std::env::var("PROCON_SOURCE_COMMIT_SHA").unwrap_or_else(|_| "unknown".to_owned());
    let cases_json: Vec<serde_json::Value> = rows
        .iter()
        .enumerate()
        .map(|(i, (label, case_rows))| {
            serde_json::json!({
                "case": i,
                "pattern": label,
                "valid": case_rows.iter().all(|r| r.valid),
                "candidates": case_rows,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "source_commit_sha": commit,
        "issue": "#333",
        "suite": "32x32 evaluation suite (9 cases x 3 candidates)",
        "candidates": candidates.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
        "kinds_pattern": "P,S,P,S,P,S,P",
        "results": cases_json,
        "summary": {
            "axis_wins_all_cases": {
                "greedy": axis_wins[0],
                "park_far": axis_wins[1],
                "tour": axis_wins[2],
            },
            "axis_losses_all_cases": {
                "greedy": axis_losses[0],
                "park_far": axis_losses[1],
                "tour": axis_losses[2],
            },
            "axis_wins_valid_only": {
                "greedy": valid_axis_wins[0],
                "park_far": valid_axis_wins[1],
                "tour": valid_axis_wins[2],
            },
            "valid_cases_all_candidates": valid_cases,
            "response_time_tiebreak_wins": {
                "greedy": rt_tiebreak_wins[0],
                "park_far": rt_tiebreak_wins[1],
                "tour": rt_tiebreak_wins[2],
            },
            "invalid_case_policy": "valid = RunRecord.scored (no illegal actions, no fuel shorts, no day-budget overruns, no malformed plan). Invalid runs are recorded with their observed numbers but excluded from valid-only tallies; all-case tallies include them and are labelled as such.",
        },
    });
    println!("\n=== BEGIN evaluation.json ===");
    println!("{}", serde_json::to_string_pretty(&doc).expect("json"));
    println!("=== END evaluation.json ===");
}

/// Average Manhattan distance of spot positions from the map center (32x32).
fn avg_manhattan_from_center(case: &bench::CaseSpec) -> f64 {
    let edge = 32i32;
    let n = u32::try_from(case.setting.spots.len().max(1)).unwrap_or(u32::MAX);
    let sum: i32 = case
        .setting
        .spots
        .iter()
        .map(|s| {
            let row = s.pos / edge;
            let col = s.pos % edge;
            (row - edge / 2).abs() + (col - edge / 2).abs()
        })
        .sum();
    f64::from(sum) / f64::from(n)
}
