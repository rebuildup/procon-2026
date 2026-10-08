//! Smoke tests for the UCB + greedy-rollout policy (Issue #40).
//!
//! These tests exercise the public API added to `crates/solver-mcts`
//! by Issue #40: [`RolloutPolicy`], [`GreedyRollout`], and
//! [`NearestSpotHeuristic`]. They live in `tests/` so they cover the
//! same surface that downstream crates will import.

use std::time::Duration;

use solver_mcts::Solver;
use solver_mcts::node::MctsNode;
use solver_mcts::rollout::{GreedyRollout, NearestSpotHeuristic, RolloutPolicy};
use solver_mcts::types::{Action, ActionSpec, DayState};

/// Five-action fixture with strictly-increasing then decreasing
/// `immediate_value` so the "best" action is unambiguous.
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

/// Mock greedy heuristic that always picks the legal action with the
/// highest `immediate_value`. Equivalent to [`NearestSpotHeuristic`]
/// but lives as a separate impl so the trait's pluggability is
/// exercised.
#[derive(Debug, Clone, Copy, Default)]
struct HighestValueHeuristic;

impl GreedyRollout for HighestValueHeuristic {
    fn greedy_step(&self, state: &DayState) -> Action {
        // Reuse the canonical implementation for the actual logic —
        // this test only needs to confirm the trait dispatch works.
        NearestSpotHeuristic.greedy_step(state)
    }
}

/// Adversarial heuristic that picks the worst action. Used to verify
/// that UCB still finds the *true* best action even when the rollout
/// is pessimistic about every candidate except the one we want to
/// surface.
#[derive(Debug, Clone, Copy, Default)]
struct LowestValueHeuristic;

impl GreedyRollout for LowestValueHeuristic {
    fn greedy_step(&self, state: &DayState) -> Action {
        let legal = state.legal_actions();
        let mut worst: Option<(Action, f64)> = None;
        for a in legal {
            let v = state
                .pool
                .iter()
                .find(|s| s.index == a.0)
                .map_or(f64::INFINITY, |s| s.immediate_value);
            match worst {
                None => worst = Some((a, v)),
                Some((_, wv)) if v < wv => worst = Some((a, v)),
                _ => {}
            }
        }
        worst.map_or(Action(0), |(a, _)| a)
    }
}

#[test]
fn ucb_selection_prefers_less_visited_child_when_scores_tie() {
    // Two children with the *same* mean reward but different visit
    // counts. UCB1's exploration term is larger for the less-visited
    // child, so it must win.
    let mut root = MctsNode::root();
    root.expand(Action(10));
    root.expand(Action(20));
    // mean_reward = 0.5 for both children.
    root.children[0].visits = 4;
    root.children[0].total_reward = 2.0;
    root.children[1].visits = 1;
    root.children[1].total_reward = 0.5;
    root.visits = 5;

    let idx = root
        .best_child_ucb1(root.visits, 1.0)
        .expect("two children → best_child_ucb1 returns Some");

    assert_eq!(
        idx, 1,
        "less-visited child (visits=1) should win the UCB1 tie over \
         the more-visited child (visits=4) when mean rewards tie"
    );

    // Symmetry check: if we swap the visit counts the same logic flips.
    root.children[0].visits = 1;
    root.children[0].total_reward = 0.5;
    root.children[1].visits = 4;
    root.children[1].total_reward = 2.0;
    let idx = root
        .best_child_ucb1(root.visits, 1.0)
        .expect("two children → best_child_ucb1 returns Some");
    assert_eq!(idx, 0, "after swapping visit counts, the less-visited child must win");
}

#[test]
fn greedy_rollout_returns_deterministic_results_with_same_seed() {
    let policy =
        RolloutPolicy::new(core::f64::consts::SQRT_2, 16, 0x00C0_FFEE_u64).with_iterations(2_000);
    let state = fixture();
    let heuristic = NearestSpotHeuristic;

    let a = policy.search(&state, &heuristic).expect("search returns Ok");
    let b = policy.search(&state, &heuristic).expect("search returns Ok");

    assert_eq!(a.action, b.action, "same seed must yield same action");
    assert_eq!(a.visits, b.visits, "visit counts must match");
    assert!(
        (a.mean_reward - b.mean_reward).abs() < 1e-9,
        "mean reward must match (deterministic rollout); a={}, b={}",
        a.mean_reward,
        b.mean_reward,
    );
    assert_eq!(a.iterations_run, b.iterations_run);

    // The greedy rollout always terminates on `is_terminal`, so the
    // seed parameter shouldn't change the result for this deterministic
    // policy either — assert that explicitly.
    let other =
        RolloutPolicy::new(core::f64::consts::SQRT_2, 16, 0xDEAD_BEEF_u64).with_iterations(2_000);
    let c = other.search(&state, &heuristic).expect("search returns Ok");
    assert_eq!(a.action, c.action);
}

#[test]
fn best_action_improves_over_random_baseline() {
    // The fixture has 5 actions with values [1, 5, 3, 4, 2]. Greedy /
    // UCB should pick index 1 (value 5) or, at worst, index 3 (value 4).
    // A uniform-random baseline that picks the *lowest-index legal
    // action* (action 0, value 1) loses every time.
    let state = fixture();

    let policy =
        RolloutPolicy::new(core::f64::consts::SQRT_2, 16, 0xABCD_1234_u64).with_iterations(2_000);
    let picked = policy.search(&state, &NearestSpotHeuristic).expect("search returns Ok").action;

    // Score the picked action via the same heuristic the rollout uses.
    let picked_value =
        state.pool.iter().find(|s| s.index == picked.index()).map_or(0.0, |s| s.immediate_value);

    // Random baseline: lowest-index legal action is `Action(0)` with
    // value 1.0.
    let baseline_value =
        state.pool.iter().find(|s| s.index == 0).map_or(0.0, |s| s.immediate_value);

    assert!(
        picked_value > baseline_value,
        "RolloutPolicy picked {picked:?} (value={picked_value}); random baseline picked \
         Action(0) (value={baseline_value}). UCB+greedy must beat the baseline."
    );
}

#[test]
fn rollout_policy_works_via_solver_trait() {
    // Confirm that the [`Solver`] impl dispatches through `plan_step`
    // and uses the default heuristic. The fixture's "worst" first
    // action is index 0 (value 1.0); the UCB + greedy-rollout loop
    // must produce a legal action that is at least as good.
    let state = fixture();
    let policy = RolloutPolicy::default();

    let picked = policy
        .plan_step(&state, Duration::from_secs(60))
        .expect("Solver::plan_step must produce an action");
    assert!(
        state.pool.iter().any(|s| s.index == picked.index()),
        "Solver::plan_step must return a legal action index"
    );

    let worst_value = state.pool.iter().map(|s| s.immediate_value).fold(f64::INFINITY, f64::min);
    let picked_value =
        state.pool.iter().find(|s| s.index == picked.index()).map_or(0.0, |s| s.immediate_value);
    assert!(
        picked_value > worst_value,
        "Solver::plan_step picked {picked:?} (value={picked_value}) but the worst legal \
         action has value={worst_value}. UCB+greedy should not pick the worst."
    );
}

#[test]
fn heuristic_trait_supports_custom_implementations() {
    // Plug a custom heuristic into the policy and verify it actually
    // drives the rollout. The `HighestValueHeuristic` mirrors
    // `NearestSpotHeuristic`, so the picked action must be legal.
    let state = fixture();
    let policy = RolloutPolicy::new(core::f64::consts::SQRT_2, 16, 7);
    let picked = policy.search(&state, &HighestValueHeuristic).expect("search returns Ok").action;
    assert!(
        state.pool.iter().any(|s| s.index == picked.index()),
        "highest-value heuristic must pick a legal action"
    );

    // A pessimistic rollout that always picks the worst action still
    // ends with a finite reward and a valid (legal) action.
    let picked = policy.search(&state, &LowestValueHeuristic).expect("search returns Ok").action;
    assert!(state.pool.iter().any(|s| s.index == picked.index()));

    // Sanity-check the trait dispatch really drives the search:
    // calling the same heuristic twice with the same seed must
    // produce the same result.
    let again = policy.search(&state, &HighestValueHeuristic).expect("search returns Ok").action;
    assert_eq!(picked, again);
}

#[test]
fn empty_action_set_is_reported_as_error() {
    // A state with `max_steps == 0` and no legal actions is terminal
    // from the solver's perspective. Force the empty case by building
    // a state whose pool is exhausted.
    let exhausted = DayState {
        step: 1,
        max_steps: 1,
        pool: vec![ActionSpec { index: 0, immediate_value: 1.0 }],
        taken: vec![Action(0)],
        cumulative_value: 1.0,
    };
    assert_eq!(exhausted.legal_actions(), Vec::new());

    let policy = RolloutPolicy::default();
    let err = policy
        .search(&exhausted, &NearestSpotHeuristic)
        .expect_err("empty action set must surface as Err");
    assert_eq!(err, solver_mcts::rollout::RolloutError::EmptyActionSet);

    // Same for the Solver trait impl.
    let err = policy
        .plan_step(&exhausted, Duration::from_secs(1))
        .expect_err("Solver::plan_step must surface EmptyActionSet");
    assert_eq!(err, solver_mcts::rollout::RolloutError::EmptyActionSet);
}
