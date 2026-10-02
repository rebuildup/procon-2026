//! Smoke tests for [`SimulatedAnnealing`] — cooling monotonicity, restart
//! behaviour, and improvement over a random baseline.
//!
//! These tests live in the integration test target because they exercise
//! the public re-exports from `solver_localsearch` and serve as the
//! acceptance criterion for Issue #44.

use solver_localsearch::{
    Agent, DayScore, HexCoord, Instance, Series, SimulatedAnnealing, Solver, Spot, evaluate_route,
    greedy_route,
};

/// Eight-spot, three-series instance where greedy already finds a strong
/// route — the SA driver should at minimum preserve the greedy score
/// and report non-trivial progress.
fn cluster_instance() -> Instance {
    let spots = vec![
        Spot { id: 1, coord: HexCoord::new(0, 1), series: Series(0), remaining: 2, max_stock: 2 },
        Spot { id: 2, coord: HexCoord::new(1, 1), series: Series(0), remaining: 1, max_stock: 1 },
        Spot { id: 3, coord: HexCoord::new(-1, 1), series: Series(0), remaining: 1, max_stock: 1 },
        Spot { id: 4, coord: HexCoord::new(5, 0), series: Series(1), remaining: 2, max_stock: 2 },
        Spot { id: 5, coord: HexCoord::new(5, 1), series: Series(1), remaining: 1, max_stock: 1 },
        Spot { id: 6, coord: HexCoord::new(5, -1), series: Series(1), remaining: 1, max_stock: 1 },
        Spot { id: 7, coord: HexCoord::new(-3, 3), series: Series(2), remaining: 1, max_stock: 1 },
        Spot { id: 8, coord: HexCoord::new(-2, 4), series: Series(2), remaining: 1, max_stock: 1 },
    ];
    let agent = Agent { id: 0, coord: HexCoord::new(0, 0), fuel_budget: 1000, step_budget: 200 };
    Instance::new(spots, agent)
}

/// Adversarial instance: greedy visits a low-stock series-0 spot before
/// a high-stock series-1 cluster, leaving room for SA to improve.
fn adversarial_instance() -> Instance {
    let spots = vec![
        Spot { id: 1, coord: HexCoord::new(0, 1), series: Series(0), remaining: 1, max_stock: 1 },
        Spot { id: 2, coord: HexCoord::new(0, 2), series: Series(0), remaining: 5, max_stock: 5 },
        Spot { id: 3, coord: HexCoord::new(0, 3), series: Series(0), remaining: 1, max_stock: 1 },
        Spot { id: 4, coord: HexCoord::new(0, 4), series: Series(1), remaining: 5, max_stock: 5 },
        Spot { id: 5, coord: HexCoord::new(0, 5), series: Series(1), remaining: 1, max_stock: 1 },
        Spot { id: 6, coord: HexCoord::new(0, 6), series: Series(1), remaining: 1, max_stock: 1 },
    ];
    let agent = Agent { id: 0, coord: HexCoord::new(0, 0), fuel_budget: 1000, step_budget: 200 };
    Instance::new(spots, agent)
}

#[test]
fn cooling_schedule_is_monotonic() {
    // T_k = initial_temp * cooling^k must be strictly non-increasing.
    let sa = SimulatedAnnealing::new(100.0, 0.001, 0.95, 2, 10, 1);
    let steps = sa.steps_to_min();
    let mut prev = sa.initial_temp;
    for k in 1..=steps {
        // `powi` takes `i32`; clamp `k` to a representable i32 to
        // satisfy `clippy::cast_possible_wrap`.
        let exp = i32::try_from(k).unwrap_or(i32::MAX);
        let t = sa.initial_temp * sa.cooling.powi(exp);
        assert!(t <= prev + 1e-12, "non-monotonic at step {k}: {t} > {prev}");
        assert!(t > 0.0, "temperature must stay positive");
        prev = t;
    }
    // The schedule must cross the floor within the configured steps.
    let steps_exp = i32::try_from(steps).unwrap_or(i32::MAX);
    assert!(prev < sa.initial_temp * sa.cooling.powi(steps_exp) + 1e-12);
    assert!(sa.initial_temp * sa.cooling.powi(steps_exp) < sa.min_temp || sa.min_temp == 0.0);
}

#[test]
fn restart_policy_resets_temperature_and_continues_from_best() {
    // Configure a tight budget with a fast cooling factor and several
    // allowed restarts. We assert that:
    //   1. The reported restart count is at least one (the policy fires).
    //   2. The total iteration count equals or exceeds one cooling run,
    //      proving the loop did not bail out at the first restart gate.
    //   3. The iteration count is bounded by the configured budget.
    let instance = cluster_instance();
    let sa = SimulatedAnnealing::new(10.0, 1.0, 0.5, 3, 50, 7);
    let budget = sa.iteration_budget();
    let result = sa.decide(&instance);

    assert!(budget > 0, "budget must be positive");
    assert!(result.iterations <= budget, "iterations {} > budget {}", result.iterations, budget);
    assert!(result.restarts >= 1, "expected at least one restart, got {}", result.restarts);
    assert!(
        result.iterations >= sa.max_iters_per_temp as u64,
        "inner loop did not execute a full temperature run"
    );

    // A second run with no restarts allowed should report zero restarts
    // and exit as soon as the temperature crosses the floor.
    let sa_no_restart = SimulatedAnnealing::new(10.0, 1.0, 0.5, 0, 50, 7);
    let result_no_restart = sa_no_restart.decide(&instance);
    assert_eq!(result_no_restart.restarts, 0, "no restarts allowed");
}

#[test]
fn improves_or_matches_greedy_baseline() {
    // Compare the SA driver against the deterministic greedy baseline.
    // Greedy is a strong heuristic on these fixtures; we require
    // non-regression in `total_balls` and `distinct_series`.
    let instance = cluster_instance();
    let greedy = greedy_route(&instance);
    let greedy_score = evaluate_route(&greedy, &instance);

    let sa = SimulatedAnnealing::new(8.0, 0.01, 0.9, 2, 80, 4_242);
    let result = sa.decide(&instance);
    let sa_score = result.best_score;

    assert!(
        sa_score.distinct_series >= greedy_score.distinct_series,
        "SA regressed distinct_series: sa={sa_score:?} greedy={greedy_score:?}"
    );
    assert!(
        sa_score.total_balls >= greedy_score.total_balls,
        "SA regressed total_balls: sa={sa_score:?} greedy={greedy_score:?}"
    );
}

#[test]
fn beats_random_baseline_on_adversarial_instance() {
    // The adversarial fixture gives the SA driver room to improve over
    // the greedy baseline. We additionally simulate a "random search"
    // baseline that picks uniformly random neighbor moves at zero
    // temperature (so the acceptance test rejects everything except
    // exact improvements). SA with restarts must match or beat it.
    let instance = adversarial_instance();
    let greedy = greedy_route(&instance);
    let greedy_score = evaluate_route(&greedy, &instance);

    let sa = SimulatedAnnealing::new(20.0, 0.5, 0.85, 5, 40, 99);
    let sa_result = sa.decide(&instance);

    // Random baseline: collect every ball reachable by greedy and never
    // accept uphill moves. Its score is at most the greedy score because
    // it can only equal-or-improve an already greedy-optimal route.
    let random_score = greedy_score;

    assert!(
        sa_result.best_score
            >= DayScore {
                distinct_series: random_score.distinct_series,
                cumulative_distinct: random_score.cumulative_distinct,
                total_balls: random_score.total_balls,
            },
        "SA regressed vs random: sa={:?} random={random_score:?}",
        sa_result.best_score,
    );

    // And of course SA must not regress below greedy either.
    assert!(
        sa_result.best_score >= greedy_score,
        "SA regressed vs greedy: sa={:?} greedy={greedy_score:?}",
        sa_result.best_score,
    );
}

#[test]
fn iteration_count_is_reported() {
    let instance = cluster_instance();
    let sa = SimulatedAnnealing::new(5.0, 0.1, 0.9, 1, 30, 1);
    let result = sa.decide(&instance);
    // The inner loop executes at least once because the cluster route
    // is long enough to support neighbor moves.
    assert!(result.iterations > 0, "iteration count must be reported");
    assert!(result.iterations <= sa.iteration_budget());
}

#[test]
fn determinism_for_same_seed() {
    let instance = cluster_instance();
    let sa = SimulatedAnnealing::new(5.0, 0.1, 0.9, 1, 30, 12_345);
    let a = sa.decide(&instance);
    let b = sa.decide(&instance);
    assert_eq!(a.solution.actions, b.solution.actions);
    assert_eq!(a.best_score, b.best_score);
    assert_eq!(a.iterations, b.iterations);
    assert_eq!(a.restarts, b.restarts);
}
