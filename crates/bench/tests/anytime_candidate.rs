//! Anytime candidate registry validation (Issue #340).
//!
//! Canonical id `anytime:budget=official` must build, and invalid
//! `budget_ms` / `trials` / `seed` values must be rejected explicitly
//! instead of being silently defaulted. The offline `trials` mode must be
//! reproducible: same id + seed, same plan (Codex P1 review finding — the
//! live `budget_ms` wall-clock stop is load-dependent and must not be used
//! for recorded offline evidence).
use bench::CandidateRegistry;
use domain::{MatchId, PlayerId};
use engine::model::{
    Agent, Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState, Spot, Terrain,
};

fn setting() -> MatchSetting {
    let cells = (0..8)
        .flat_map(|row| {
            (0..8).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = (row == 1 && (1..=4).contains(&col)).then_some(Spot {
                    series: if col < 3 { 1 } else { u32::try_from(col - 1).unwrap() },
                    pos,
                    max_stock: 3,
                    current_stock: 3,
                });
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new("anytime-test").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(8, 8).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 1), 32),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(0, 2)),
        ],
        day_steps: 16,
        num_days: 2,
        day_steps_by_day: Vec::new(),
    }
}

#[test]
fn anytime_budget_official_constructs() {
    let registry = CandidateRegistry::builtin();
    assert!(registry.build("anytime:budget=official").and_then(|c| c.build_solver()).is_ok());
}

/// Offline `trials` mode must be reproducible: two fresh solvers built from
/// the same id+seed decide identically. Because the trials stop is purely
/// count-based (no `Instant` reads), this holds across processes and
/// machines too, not just within one run.
#[test]
fn anytime_trials_mode_is_reproducible_for_same_id_and_seed() {
    let solver_a = CandidateRegistry::builtin()
        .build("anytime:trials=64:seed=42:budget=official")
        .and_then(|c| c.build_solver())
        .expect("offline trials id builds");
    let solver_b = CandidateRegistry::builtin()
        .build("anytime:trials=64:seed=42:budget=official")
        .and_then(|c| c.build_solver())
        .expect("offline trials id builds");
    let state = MatchState::new(setting()).unwrap();
    let ctx = engine::model::SolverContext::from_state(&state);
    assert_eq!(solver_a.decide(&ctx), solver_b.decide(&ctx), "same id+seed, same plan");
    assert_eq!(
        solver_a.decide(&ctx),
        solver_b.decide(&ctx),
        "repeated decide on the same context is also stable"
    );
}

#[test]
fn anytime_rejects_invalid_trials() {
    let registry = CandidateRegistry::builtin();
    assert!(
        registry
            .build("anytime:trials=abc:budget=official")
            .and_then(|c| c.build_solver())
            .is_err()
    );
    assert!(
        registry.build("anytime:trials=-1:budget=official").and_then(|c| c.build_solver()).is_err()
    );
    assert!(
        registry.build("anytime:trials=0:budget=official").and_then(|c| c.build_solver()).is_ok()
    );
}

#[test]
fn anytime_rejects_invalid_budget_ms() {
    let registry = CandidateRegistry::builtin();
    // Invalid budget_ms must be rejected, not silently defaulted.
    assert!(
        registry
            .build("anytime:budget_ms=abc:budget=official")
            .and_then(|c| c.build_solver())
            .is_err()
    );
    assert!(
        registry
            .build("anytime:budget_ms=-1:budget=official")
            .and_then(|c| c.build_solver())
            .is_err()
    );
    assert!(
        registry
            .build("anytime:budget_ms=60000:budget=official")
            .and_then(|c| c.build_solver())
            .is_err()
    );
    // Valid values must work.
    assert!(
        registry
            .build("anytime:budget_ms=0:budget=official")
            .and_then(|c| c.build_solver())
            .is_ok()
    );
    assert!(
        registry
            .build("anytime:budget_ms=5000:budget=official")
            .and_then(|c| c.build_solver())
            .is_ok()
    );
    assert!(
        registry
            .build("anytime:budget_ms=59000:budget=official")
            .and_then(|c| c.build_solver())
            .is_ok()
    );
}

#[test]
fn anytime_rejects_invalid_seed() {
    let registry = CandidateRegistry::builtin();
    assert!(
        registry.build("anytime:seed=abc:budget=official").and_then(|c| c.build_solver()).is_err()
    );
    assert!(
        registry.build("anytime:seed=-1:budget=official").and_then(|c| c.build_solver()).is_err()
    );
    assert!(
        registry.build("anytime:seed=42:budget=official").and_then(|c| c.build_solver()).is_ok()
    );
}
