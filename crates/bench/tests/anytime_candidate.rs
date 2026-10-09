//! Anytime candidate registry validation (Issue #340) and final-return
//! acceptance (Issue #354).
//!
//! Canonical id `anytime:budget=official` must build, and invalid
//! `budget_ms` / `trials` / `seed` values must be rejected explicitly
//! instead of being silently defaulted. The offline `trials` mode must be
//! reproducible: same id + seed, same plan (Codex P1 review finding — the
//! live `budget_ms` wall-clock stop is load-dependent and must not be used
//! for recorded offline evidence).
//!
//! The dictionary-order acceptance tests (Issue #354) pin the final-return
//! guard: an accepted axis-3 (B) improvement must reach the *returned*
//! `AgentActions`, not only the internal `best` variable.
use bench::CandidateRegistry;
use domain::{MatchId, PlayerId};
use engine::MatchSim;
use engine::model::{
    Agent, AgentActions, Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState, Spot, Terrain,
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

// ---------------------------------------------------------------------------
// Dictionary-order acceptance and the final-return guard (Issue #354)
// ---------------------------------------------------------------------------

/// Fixture for the B-improvement regression (Issue #354).
///
/// Construction intent (all Plain terrain, patrol step cost 2/cell):
///
/// - Two patrol starts are split (`(2, 1)` and `(4, 4)`), and the four
///   spots sit at hand-checked walking distances (2-7 cells, cube metric)
///   from those starts so that the visit ORDER decides how many picks fit
///   into the tight 26-step day budget (13 weighted moves).
/// - `park=far` variant 0 (the anytime initial plan) ends the day with
///   V=3 / D=3 / B=6; jitter variant 5 visits the spots in an order that
///   fits one extra pick: same V, same D, B=7.
/// - Variant seeds only jitter the coverage cost key (`cost * (8 + jitter)`
///   per target cell) and rotate the patrol order, so the flip between the
///   two orders is deterministic for a fixed seed. Pinned against the
///   release-0-4-0 tip `87595f2` in BOTH profiles (Issue #342 gate red:
///   the previous fixture's variant-4 premise no longer held after the
///   #353/#354 planner changes — it failed in dev and release alike).
///
/// Official axes: V = `series_taken` (tier 1), D = `series_acquired_by_day`
/// (tier 2), B = total balls picked (tier 3).
fn b_improvement_setting() -> MatchSetting {
    let spots = [(8i32, 1i32, 1u32), (7, 5, 2), (4, 5, 3), (3, 0, 3)];
    let (w, h) = (16u8, 16u8);
    let cells: Vec<Cell> = (0..i32::from(h))
        .flat_map(|row| {
            (0..i32::from(w)).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot =
                    spots.iter().find(|(c, r, _)| *c == col && *r == row).map(|(_, _, series)| {
                        Spot { series: *series, pos, max_stock: 3, current_stock: 3 }
                    });
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new("anytime-b-improvement").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(w, h).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(2, 1), 60),
            Agent::patrol_with_fuel(1, HexCoord::new(4, 4), 60),
            Agent::supply(2, HexCoord::new(2, 2)),
        ],
        day_steps: 26,
        num_days: 2,
        day_steps_by_day: Vec::new(),
    }
}

/// Replay a day plan through the real simulator and report the local
/// official axes: (`series_taken`, `series_today`, `balls`).
fn replay_axes(state0: &MatchState, plan: &AgentActions) -> (u32, u32, u32) {
    let mut state = state0.clone();
    let sim = MatchSim::default();
    let n = state0.setting.agents.len();
    for step in plan.actions.chunks(n) {
        sim.step(&mut state, &AgentActions::new(step.to_vec()));
    }
    let series_today = state
        .series_acquired_by_day
        .get(&state0.day)
        .map_or(0, |s| u32::try_from(s.len()).unwrap());
    let balls: u32 = state.agents.iter().map(engine::Agent::total_inventory).sum();
    (u32::try_from(state.series_taken.len()).unwrap(), series_today, balls)
}

/// The final returned plan must be the B-improved trial, not the initial
/// plan (Issue #354 guard fix).
///
/// Pre-fix behaviour (branch `350` commit `31f2062`, "true B-improvement
/// acceptance"): the trial with the same V/D and higher B WAS accepted into
/// the internal `best`, but the final guard `if best_score.0 <=
/// initial_score.0 { return initial; }` compared only the axis-2 series
/// count. With series equal (`3 <= 3`) it returned the INITIAL plan, so the
/// B improvement never reached the returned `AgentActions`. This test pins
/// the fixed behaviour: `decide` must return the improved trial.
#[test]
fn anytime_final_return_reflects_b_improvement_trial() {
    let state = MatchState::new(b_improvement_setting()).unwrap();
    let ctx = engine::model::SolverContext::from_state(&state);
    let registry = CandidateRegistry::builtin();

    // The anytime initial solution is byte-identical to the park=far
    // baseline (variant 0).
    let baseline = registry
        .build("coverage:budget=official:park=far")
        .and_then(|c| c.build_solver())
        .expect("park=far builds");
    let initial = baseline.decide(&ctx);
    let initial_axes = replay_axes(&state, &initial);
    assert_eq!(initial_axes, (3, 3, 6), "fixture baseline must stay pinned");

    // `trials=2:seed=3`: trial 1 (variant 4) scores an exact tie
    // (3, 3, 6, 190) and is not accepted; trial 2 (variant 5) has the same
    // V and D but B 6 -> 7. Deterministic offline mode: no wall clock
    // anywhere in this path.
    let solver = registry
        .build("anytime:trials=2:seed=3:budget=official")
        .and_then(|c| c.build_solver())
        .expect("anytime trials id builds");
    let improved = solver.decide(&ctx);
    let improved_axes = replay_axes(&state, &improved);

    // STRICT final-return guard (Issue #354 / #364): the returned plan must
    // BE the B-improved trial — not the initial plan. The pre-fix guard
    // compared only the axis-2 series count and returned the initial plan on
    // a B-only improvement, so both assertions below fail against it (and an
    // implementation that always returns the initial plan would pass the
    // weaker "not worse" form — the `>=` relaxation is therefore reverted).
    assert_ne!(improved, initial, "trial #2 (variant 5) must replace the returned plan");
    assert_eq!(improved_axes, (3, 3, 7), "same V, same D, one extra ball");
}

/// With no improving trial, the initial park=far plan must still come back
/// untouched — the guard fix must not start returning a worse or mutated
/// plan when the exploration finds nothing.
#[test]
fn anytime_no_improvement_returns_initial_plan_unchanged() {
    let state = MatchState::new(b_improvement_setting()).unwrap();
    let ctx = engine::model::SolverContext::from_state(&state);
    let registry = CandidateRegistry::builtin();
    let baseline = registry
        .build("coverage:budget=official:park=far")
        .and_then(|c| c.build_solver())
        .expect("park=far builds");
    let initial = baseline.decide(&ctx);

    // seed=1, trials=1: the single trial (variant) replays the same tour and
    // scores exactly the initial axes (3, 3, 6) — NO trial improves (equal
    // V/D/B, equal-or-worse positioning). The guarantee is byte-identity:
    // the initial park=far plan is returned untouched. A final guard that
    // returned a trial on an all-axes tie ("improved-when-equal") or that
    // discarded the initial plan would produce a different `AgentActions`
    // and fail this assertion (the `>=` non-regression form accepted both).
    let solver = registry
        .build("anytime:trials=1:seed=1:budget=official")
        .and_then(|c| c.build_solver())
        .expect("anytime trials id builds");
    let returned = solver.decide(&ctx);
    let returned_axes = replay_axes(&state, &returned);
    assert_eq!(
        returned_axes,
        replay_axes(&state, &initial),
        "no improving trial: the axes must stay exactly the initial (3, 3, 6)"
    );
    assert_eq!(
        returned, initial,
        "no improving trial: the initial plan must be returned byte-identically"
    );
}

/// Regression test: B-only improvement must be returned (Issue #364).
///
/// Uses `fixture:large-32x32` where anytime produces V=4/D=4/B=21 vs
/// coverage's V=4/D=4/B=14. V and D are identical; B strictly increases.
/// The final returned plan must reflect this B gain.
#[test]
fn anytime_b_only_improvement_returned_on_large32() {
    let case = bench::CaseSpec::from_bundled_fixture("large-32x32").unwrap();
    let pattern = bench::kinds::parse_kinds("P,S,P,S,P").unwrap();
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a").unwrap();
    let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).unwrap();
    let state = MatchState::new(setting).unwrap();
    let ctx = engine::model::SolverContext::from_state(&state);
    let registry = CandidateRegistry::builtin();

    let baseline = registry
        .build("coverage:budget=official:park=far")
        .and_then(|c| c.build_solver())
        .expect("park=far builds");
    let initial = baseline.decide(&ctx);
    let initial_axes = replay_axes(&state, &initial);

    // Fixed trials mode: deterministic, no wall clock.
    let anytime = registry
        .build("anytime:trials=64:seed=42:budget=official")
        .and_then(|c| c.build_solver())
        .expect("anytime builds");
    let returned = anytime.decide(&ctx);
    let returned_axes = replay_axes(&state, &returned);

    // V must be identical
    assert_eq!(returned_axes.0, initial_axes.0, "V must be the same for B_WIN classification");
    // D must be identical
    assert_eq!(returned_axes.1, initial_axes.1, "D must be the same for B_WIN classification");
    // B must be strictly greater
    assert!(
        returned_axes.2 > initial_axes.2,
        "B must strictly increase for B_WIN: got {} vs initial {}",
        returned_axes.2,
        initial_axes.2
    );
}
