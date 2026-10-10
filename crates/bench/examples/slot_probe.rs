//! Which agent `choose_supply_slot` makes the Supply, and on how many
//! trial matches (Issue #428).
//!
//! Usage: `slot_probe WINDOW_MS CASE_JSON...` — one JSON line per case.
use bench::CaseSpec;
use bench::adapters::fleet_solver::choose_supply_slot;
use serde_json::json;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 3 {
        return Err("expected WINDOW_MS CASE_JSON...".into());
    }
    let window = Duration::from_millis(argv[1].parse()?);
    let team = domain::PlayerId::new("player-a")?;
    for path in &argv[2..] {
        let case: CaseSpec = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let started = Instant::now();
        let (slot, trials) = choose_supply_slot(&case.setting, &team, window);
        println!(
            "{}",
            json!({"case_id": case.case_id, "slot": slot,
                "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "matches": trials.first().map_or(0, |t| t.matches),
                "totals": trials.iter().map(|t| [t.total.0, t.total.1, t.total.2]).collect::<Vec<_>>()})
        );
    }
    Ok(())
}
