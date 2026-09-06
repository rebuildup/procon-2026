//! Acceptance test for `MctsSolver`: after a 1-step MCTS search the
//! returned action must be at least as good (by the heuristic `eval`) as
//! the greedy baseline.
//!
//! `docs/contest/strategy.md` §5.3 Phase 3: partial MCTS must not
//! regress against greedy.
//!
//! EXPECTED INTEGRATION: domain — the heuristic here mirrors what the
//! future `domain::GameState::evaluate_step` will expose. Once #18 lands,
//! this test binds to the canonical scorer instead of the stub.

use std::time::Duration;

use solver_mcts::rollout::Rng;
use solver_mcts::types::{ActionSpec, DayState, plan_one_step};
use solver_mcts::{MctsConfig, MctsSolver, Solver};

/// Score a day's plan against the same heuristic the rollout uses. Kept
/// in this test file because the canonical evaluator lives in domain once
/// #18 lands.
fn evaluate_after(state: &DayState, action: solver_mcts::types::Action) -> f64 {
    let next = state.apply(action);
    next.cumulative_value
}

fn fixture() -> DayState {
    DayState::new(
        vec![
            ActionSpec { index: 0, immediate_value: 1.0 },
            ActionSpec { index: 1, immediate_value: 5.0 },
            ActionSpec { index: 2, immediate_value: 3.0 },
            ActionSpec { index: 3, immediate_value: 4.0 },
            ActionSpec { index: 4, immediate_value: 2.0 },
        ],
        32,
    )
}

#[test]
fn mcts_best_child_matches_or_beats_greedy() {
    let state = fixture();
    let greedy = plan_one_step(&state).expect("greedy must produce an action");
    let greedy_score = evaluate_after(&state, greedy);

    // Use a deterministic seed so the test isn't flaky. The solver still
    // converges to greedy-equivalent in well under `iterations` because
    // the rollout policy shares the greedy ordering.
    let cfg = MctsConfig {
        iterations: 1_000,
        explore_constant: core::f64::consts::SQRT_2,
        rollout_depth: 12,
        epsilon: 0.1,
        time_budget: None,
        rng_seed: 0x00C0_FFEE_u64,
    };
    let solver = MctsSolver::with_config(cfg);

    let picked =
        solver.plan_step(&state, Duration::from_secs(60)).expect("solver must produce an action");
    let mcts_score = evaluate_after(&state, picked);

    assert!(
        mcts_score >= greedy_score,
        "MCTS ({}, score={}) regressed against greedy ({}, score={})",
        picked.index(),
        mcts_score,
        greedy.index(),
        greedy_score,
    );
}

#[test]
fn mcts_search_runs_within_day_budget() {
    let state = fixture();
    let cfg = MctsConfig {
        iterations: u32::MAX,
        time_budget: Some(Duration::from_millis(120)),
        ..MctsConfig::default()
    };
    let solver = MctsSolver::with_config(cfg);
    let started = std::time::Instant::now();
    let picked =
        solver.plan_step(&state, Duration::from_secs(60)).expect("solver must produce an action");
    let elapsed = started.elapsed();

    assert!(elapsed <= Duration::from_millis(250));
    assert_ne!(picked.index(), u16::MAX);
}

#[test]
fn rng_seed_zero_does_not_panic_and_is_deterministic() {
    let cfg = MctsConfig { iterations: 64, rng_seed: 0, ..MctsConfig::default() };
    let solver = MctsSolver::with_config(cfg);
    let state = fixture();
    let a = solver.plan_step(&state, Duration::from_secs(1)).expect("ok");
    let b = solver.plan_step(&state, Duration::from_secs(1)).expect("ok");
    assert_eq!(a.index(), b.index());

    // Sanity-check the in-crate PRNG has a non-trivial first sample.
    let mut rng = Rng::from_seed(0);
    let v = rng.next_f64();
    assert!((0.0..1.0).contains(&v));
}
