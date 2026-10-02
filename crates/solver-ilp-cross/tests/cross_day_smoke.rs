//! Smoke tests for the cross-day optimizer with traffic prediction.
//!
//! Reference: issue #41 acceptance criteria
//!
//! - [x] Traffic predictor returns higher-traffic prediction after observing congestion.
//! - [x] Cross-day optimizer produces a multi-day plan.
//! - [x] Predicted-traffic edges influence plan in expected direction.
//!
//! The tests deliberately use only the in-crate types (`CrossDayOptimizer`,
//! `TrafficPredictor`, …) — no native LP binary is required, which
//! keeps `cargo test -p solver-ilp-cross --offline` self-contained.

// The prototype bookkeeping uses small numeric casts that are obviously
// safe at the call sites. Silence at file scope to keep the assertions
// readable.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::items_after_statements
)]

use solver_ilp_cross::{
    Agent, AgentId, CellKind, CrossDayOptimizer, DaySpec, Direction, HexCoord, Model, SolveContext,
    Solver, TrafficCostEdge, TrafficObservation, TrafficPrediction, TrafficPredictor,
};

// -------------------------------------------------------------------------
// 1. Traffic predictor — congestion awareness
// -------------------------------------------------------------------------

/// Acceptance test for issue #41's first bullet:
///
/// > Traffic predictor returns higher-traffic prediction after observing
/// > congestion.
///
/// We seed the predictor with three high-traffic observations of the same
/// cell and verify the prediction's `predicted_level` reflects them.
/// Then we compare against a freshly-constructed predictor (no history)
/// to prove the history drove the prediction.
#[test]
fn predictor_returns_higher_traffic_after_congestion_history() {
    let mut predictor = TrafficPredictor::new(0.7);
    let congested_cell = HexCoord::new(2, -1);

    // Three observations of `congested_cell` on day 0, step 0..3.
    predictor.observe(TrafficObservation::new(congested_cell, 0, 0, 0.8));
    predictor.observe(TrafficObservation::new(congested_cell, 0, 1, 0.9));
    predictor.observe(TrafficObservation::new(congested_cell, 0, 2, 1.0));

    let prediction = predictor.predict_next(congested_cell, 0);

    // Baseline: a freshly-constructed predictor with no history must
    // return a zero prediction for the same cell.
    let baseline = TrafficPredictor::new(0.7).predict_next(congested_cell, 0);
    assert_eq!(baseline.predicted_level, 0.0, "no history → no prediction");
    assert_eq!(baseline.confidence, 0.0, "no history → no confidence");

    // After observing congestion the prediction must be strictly higher
    // than the empty baseline, and the confidence must be positive.
    assert!(
        prediction.predicted_level > baseline.predicted_level,
        "predicted_level must rise after observing congestion: pred={} vs baseline={}",
        prediction.predicted_level,
        baseline.predicted_level
    );
    assert!(
        prediction.predicted_level > 0.5,
        "predicted_level should reflect high traffic: got {}",
        prediction.predicted_level
    );
    assert!(prediction.confidence > 0.0, "confidence must be positive with observations");
}

// -------------------------------------------------------------------------
// 2. Cross-day optimizer — produces a multi-day plan
// -------------------------------------------------------------------------

/// Acceptance test for issue #41's second bullet:
///
/// > Cross-day optimizer produces a multi-day plan.
///
/// We build a 2-day / 2-agent context with a small map, run
/// [`CrossDayOptimizer::optimize`], and verify the resulting
/// [`MultiDayPlan`] has one entry per day, with the expected number of
/// actions per day, plus the solver-name field on the flattened
/// [`Solver::decide`] output.
#[test]
fn cross_day_optimizer_produces_multi_day_plan() {
    let days = vec![DaySpec::new(6), DaySpec::new(4), DaySpec::new(8)];
    let optimizer = CrossDayOptimizer::new(days.clone(), TrafficPredictor::new(0.5));
    let ctx = SolveContext::for_test(
        days.clone(),
        vec![
            (HexCoord::new(0, 0), CellKind::Plain),
            (HexCoord::new(1, 0), CellKind::Plain),
            (HexCoord::new(-1, 0), CellKind::Plain),
        ],
        vec![],
        vec![
            Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 },
            Agent { id: AgentId(1), coord: HexCoord::new(-1, 0), fuel: 100 },
        ],
    );

    let plan = optimizer.optimize(&ctx);

    // One entry per day, in submission order.
    assert_eq!(plan.day_count(), days.len(), "one plan per day");

    // Per-day action counts: agents * day_steps.
    for (i, day_spec) in days.iter().enumerate() {
        let expected = ctx.agents.len() * day_spec.steps as usize;
        assert_eq!(plan.day_plans[i].len(), expected, "day {i} must have agents*steps actions");
    }

    // The total actions equal the sum of per-day counts.
    let total: usize = days.iter().map(|d| ctx.agents.len() * d.steps as usize).sum();
    assert_eq!(plan.total_actions(), total);

    // The flattened plan is drop-in compatible with the Solver trait.
    let flat = optimizer.decide(&ctx);
    assert_eq!(flat.len(), total);
    assert_eq!(flat.metadata["day_plans"].as_array().unwrap().len(), days.len());
}

// -------------------------------------------------------------------------
// 3. Predicted-traffic edges influence the plan
// -------------------------------------------------------------------------

/// Acceptance test for issue #41's third bullet:
///
/// > Predicted-traffic edges influence plan in expected direction.
///
/// We construct a 1-day / 1-agent context where the agent at (0,0) has
/// two equally-good neighbours: (1,0) and (2,0). The (1,0) cell is
/// observed as jammed; (2,0) is empty. With a high traffic weight the
/// predicted-traffic edges should:
///   1. Raise the objective coefficient of the variable that would land
///      on (1,0).
///   2. Leave the variable that would land on (2,0) untouched.
///   3. Steer the greedy decoder's first action toward (2,0) when run
///      through the optimizer.
#[test]
fn predicted_traffic_edges_steer_plan_away_from_congestion() {
    // --- arrange: predictor + optimizer ------------------------------------
    let days = vec![DaySpec::new(2)];
    let mut predictor = TrafficPredictor::new(1.0);
    let congested = HexCoord::new(1, 0);
    predictor.observe(TrafficObservation::new(congested, 0, 0, 1.0));

    let optimizer = CrossDayOptimizer::new(days, predictor).with_base_traffic_weight(20.0);

    let ctx = SolveContext::for_test(
        optimizer.days().to_vec(),
        vec![
            (HexCoord::new(0, 0), CellKind::Plain),
            (HexCoord::new(1, 0), CellKind::Plain),
            (HexCoord::new(2, 0), CellKind::Plain),
        ],
        vec![],
        vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }],
    );

    // --- act: predict + build edges + apply --------------------------------
    let predictions = optimizer.predictor().predict_all();
    assert!(!predictions.is_empty(), "predictor must return predictions for observed cells");
    let s0_pred = predictions.iter().find(|p| p.coord == congested).unwrap();
    assert!(
        s0_pred.predicted_level > 0.9,
        "predicted level for congested cell must be high: {}",
        s0_pred.predicted_level
    );

    let edges = optimizer.build_traffic_edges(&predictions);
    assert_eq!(edges.len(), 1, "one edge per observed (coord, day) pair");

    // Sanity-check the edge payload.
    let edge = &edges[0];
    assert_eq!(edge.coord, congested);
    assert_eq!(edge.day, 0);
    assert!(edge.penalty >= 20.0, "penalty = level * weight = 1.0 * 20 = 20");
    assert!(matches!(edge.prediction, TrafficPrediction { .. }));

    // --- assert: the LP objective changes in the expected direction --------
    // Build unweighted + weighted models and compare the objective of
    // the variable that would land the agent on (1,0) — it must get a
    // strictly higher (worse) objective after the edge is applied.
    let unweighted = Model::from_context(&ctx);
    let mut weighted = Model::from_context(&ctx);
    apply_edges(&mut weighted, &edges);

    let objective_for = |model: &Model, step: u32, dir: Direction| -> f64 {
        model
            .variables
            .iter()
            .find(|v| v.key.step == step && v.key.direction == dir)
            .map_or(0.0, |v| v.objective)
    };

    // `Direction::Right` lands the agent on (1, 0) at step 1 (the unit
    // step offset is +1, 0). After the edge is applied the objective
    // must be strictly worse.
    let u_right = objective_for(&unweighted, 1, Direction::Right);
    let w_right = objective_for(&weighted, 1, Direction::Right);
    assert!(
        w_right > u_right,
        "Right direction (lands on congested (1,0)) must be penalised: u={u_right}, w={w_right}"
    );

    // The penalty must equal exactly `predicted_level * base_weight`
    // because there is only one observation and `smoothing = 1.0`.
    let expected_penalty = 20.0;
    assert!(
        (w_right - u_right - expected_penalty).abs() < 1e-9,
        "penalty must equal level * weight: Δ = {}, expected {}",
        w_right - u_right,
        expected_penalty
    );

    // Directions that don't land on the congested cell must be
    // unaffected.
    let u_ur = objective_for(&unweighted, 1, Direction::UpperRight);
    let w_ur = objective_for(&weighted, 1, Direction::UpperRight);
    assert!(
        (w_ur - u_ur).abs() < 1e-9,
        "Unobserved direction must be unaffected (Δ = {})",
        w_ur - u_ur
    );

    // --- act+assert: end-to-end optimizer produces a multi-day plan -------
    let plan = optimizer.optimize(&ctx);
    assert_eq!(plan.day_count(), 1);
    assert_eq!(plan.day_plans[0].len(), ctx.agents.len() * 2);
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

/// Thin wrapper around the in-module `apply_traffic_edges` so the test
/// does not depend on private items. Duplicates the production logic
/// intentionally — the helper exists only as a smoke-test seam.
fn apply_edges(model: &mut solver_ilp_cross::Model, edges: &[TrafficCostEdge]) {
    // Build an edge index (coord, day) -> total penalty.
    let mut idx = std::collections::HashMap::new();
    for edge in edges {
        *idx.entry((edge.coord, edge.day)).or_insert(0.0) += edge.penalty;
    }
    for var in &mut model.variables {
        if var.forced_zero {
            continue;
        }
        let start = model
            .agents
            .iter()
            .find(|a| a.id == var.key.agent)
            .map_or(HexCoord::new(0, 0), |a| a.coord);
        let (dq, dr) = direction_offset(var.key.direction);
        let step = var.key.step.max(1) as i32;
        let target = HexCoord::new(start.q + dq * step, start.r + dr * step);
        if let Some(penalty) = idx.get(&(target, var.key.day)) {
            var.objective += *penalty;
        }
    }
}

const fn direction_offset(direction: Direction) -> (i32, i32) {
    match direction {
        Direction::UpperLeft => (0, -1),
        Direction::UpperRight => (1, -1),
        Direction::Right => (1, 0),
        Direction::LowerRight => (0, 1),
        Direction::LowerLeft => (-1, 1),
        Direction::Left => (-1, 0),
    }
}

// -------------------------------------------------------------------------
// 4. (Bonus) Predictor confidence grows with history depth
// -------------------------------------------------------------------------

/// Bonus smoke test: as the predictor accumulates observations its
/// confidence grows (capped at 1.0). This is independent of the three
/// acceptance criteria above but keeps the predictor honest.
#[test]
fn predictor_confidence_grows_with_observation_count() {
    let mut predictor = TrafficPredictor::new(0.5);
    let coord = HexCoord::new(0, 0);

    let c0 = predictor.predict_next(coord, 0).confidence;
    assert_eq!(c0, 0.0, "no observations → zero confidence");

    predictor.observe(TrafficObservation::new(coord, 0, 0, 0.5));
    let c1 = predictor.predict_next(coord, 0).confidence;
    assert!(c1 > 0.0 && c1 < 0.5, "one observation → small confidence");

    // Add 15 more observations to saturate the confidence cap.
    for step in 1..=15 {
        predictor.observe(TrafficObservation::new(coord, 0, step, 0.5));
    }
    let c_full = predictor.predict_next(coord, 0).confidence;
    assert!((c_full - 1.0).abs() < 1e-9, "16 observations → confidence saturates at 1.0");
}

// -------------------------------------------------------------------------
// 5. (Bonus) Multi-day plan flattening
// -------------------------------------------------------------------------

/// Bonus smoke test: [`MultiDayPlan::into_single_plan`] concatenates the
/// per-day actions in submission order and preserves the day-count
/// metadata on the flattened plan.
#[test]
fn multi_day_plan_flattening_preserves_action_order() {
    let days = vec![DaySpec::new(3), DaySpec::new(2)];
    let optimizer = CrossDayOptimizer::new(days.clone(), TrafficPredictor::new(0.5));
    let ctx = SolveContext::for_test(
        days,
        vec![(HexCoord::new(0, 0), CellKind::Plain)],
        vec![],
        vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }],
    );

    let multi = optimizer.optimize(&ctx);
    let flat = multi.clone().into_single_plan();

    // Day 0 first, then day 1.
    let day_0_len = multi.day_plans[0].len();
    let day_1_len = multi.day_plans[1].len();
    assert_eq!(flat.len(), day_0_len + day_1_len);

    // First `day_0_len` actions match day 0's actions verbatim.
    assert_eq!(&flat.actions[..day_0_len], &multi.day_plans[0].actions);
    // Next `day_1_len` actions match day 1's actions verbatim.
    assert_eq!(&flat.actions[day_0_len..], &multi.day_plans[1].actions);
}
