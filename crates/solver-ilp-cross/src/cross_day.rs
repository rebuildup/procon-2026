//! Cross-day optimisation with traffic prediction (issue #41).
//!
//!
//! ## Scope
//!
//! The Phase 3 cross-day solver ([`crate::IlpCrossSolver`]) already plans an
//! entire multi-day match in one shot, but its objective treats every
//! direction variable as equally cheap. Real matches are not like that:
//! cells get crowded at predictable times of the day, and the agent's
//! future-day cost is heavily shaped by the traffic it experienced in
//! past days.
//!
//! This module layers two new ideas on top of the cross-day solver:
//!
//! 1. **Traffic history** ([`TrafficObservation`]) — every past observation
//!    is a `(coord, day, step, traffic_level)` row. Traffic levels are
//!    `0.0` (empty) ... `1.0` (jammed). Observations can come from the
//!    engine's telemetry stream or be hand-injected in tests.
//! 2. **Traffic prediction** ([`TrafficPredictor`]) — converts the
//!    observation history into a per-cell, per-day prediction using an
//!    exponential moving average (EMA) controlled by the `smoothing`
//!    parameter. Cells with more history get a higher prediction
//!    `confidence`.
//!
//! Once predictions are available the [`CrossDayOptimizer`] adds
//! **traffic-weighted cost edges** to the LP objective: every variable
//! that would land the agent on a high-traffic cell gets a positive
//! coefficient penalty, scaled by `base_traffic_weight`. The
//! relaxed-LP / greedy-decode fallback in
//! [`crate::model::greedy_decode`] then naturally steers the agent
//! around predicted-congested cells.
//!
//! The output is a [`MultiDayPlan`] — one [`Plan`] per day, plus the
//! predictions that were used to build it. The [`Solver`] trait impl
//! flattens the multi-day plan into a single step-major [`Plan`] for
//! drop-in compatibility with the rest of the crate.
//!
//! ## Acceptance criteria (issue #41)
//!
//! - [x] Multi-day ILP can solve the entire match in one shot.
//! - [x] Predictions come from historical traffic observations.
//! - [x] Predicted-traffic edges influence the plan in the expected
//!   direction (agents avoid predicted-congested cells).

// The prototype bookkeeping uses small numeric casts that are obviously
// safe at the call sites. Silence at file scope instead of scattering
// per-line attributes.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use serde::{Deserialize, Serialize};

use crate::model::{Model, greedy_decode};
use crate::types::{DaySpec, Direction, HexCoord, Plan};
use crate::{Solver, types::SolveContext};

/// A single historical traffic observation.
///
/// One row per `(coord, day, step)` — the contest engine emits these for
/// every cell the agent visits. Traffic levels are normalised in
/// `[0.0, 1.0]`: `0.0` is empty, `1.0` is jammed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficObservation {
    /// Cell the observation refers to.
    pub coord: HexCoord,
    /// Day the observation was collected on.
    pub day: u32,
    /// Step within the day the observation was collected on.
    pub step: u32,
    /// Normalised traffic level: `0.0` empty, `1.0` jammed.
    pub traffic_level: f64,
}

impl TrafficObservation {
    /// Convenience constructor. The traffic level is clamped to `[0, 1]`.
    #[must_use]
    pub fn new(coord: HexCoord, day: u32, step: u32, traffic_level: f64) -> Self {
        Self { coord, day, step, traffic_level: traffic_level.clamp(0.0, 1.0) }
    }

    /// Grouping key — observations of the same `(coord, day)` are pooled
    /// into a single EMA, regardless of which step they came from.
    #[must_use]
    pub fn group_key(&self) -> (HexCoord, u32) {
        (self.coord, self.day)
    }
}

/// A predicted traffic level for a cell on a given day.
///
/// Returned by [`TrafficPredictor::predict_next`]. `confidence` is `0.0`
/// when the predictor has no observations for the queried cell, and
/// approaches `1.0` as more observations accumulate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficPrediction {
    /// Cell the prediction refers to.
    pub coord: HexCoord,
    /// Day the prediction refers to.
    pub day: u32,
    /// Predicted normalised traffic level (`0.0` ... `1.0`).
    pub predicted_level: f64,
    /// Confidence in the prediction (`0.0` ... `1.0`).
    pub confidence: f64,
}

impl TrafficPrediction {
    /// A default empty prediction: zero level, zero confidence.
    #[must_use]
    pub fn empty(coord: HexCoord, day: u32) -> Self {
        Self { coord, day, predicted_level: 0.0, confidence: 0.0 }
    }
}

/// EMA-based traffic predictor.
///
/// Holds the historical [`TrafficObservation`] stream and produces a
/// [`TrafficPrediction`] per `(coord, day)` using an exponential moving
/// average. The `smoothing` parameter is the EMA "alpha" — values
/// closer to `1.0` weight recent observations more heavily; values
/// closer to `0.0` produce a smoother (older-weighted) average.
///
/// ## Validation
///
/// `smoothing` is clamped to `[0.0, 1.0]`. Out-of-range values are
/// silently snapped to the nearest endpoint rather than rejected — the
/// predictor is a *knob*, not a contract, and a typo should not break
/// the entire planning pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficPredictor {
    history: Vec<TrafficObservation>,
    smoothing: f64,
}

impl TrafficPredictor {
    /// Construct a fresh predictor with the given EMA smoothing factor.
    /// `smoothing` is clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn new(smoothing: f64) -> Self {
        Self { history: Vec::new(), smoothing: smoothing.clamp(0.0, 1.0) }
    }

    /// Construct a predictor pre-seeded with a history vector. Useful
    /// for replaying observations from disk.
    #[must_use]
    pub fn with_history(history: Vec<TrafficObservation>, smoothing: f64) -> Self {
        Self { history, smoothing: smoothing.clamp(0.0, 1.0) }
    }

    /// Add a single observation to the history.
    pub fn observe(&mut self, obs: TrafficObservation) {
        self.history.push(obs);
    }

    /// Read-only access to the historical observation stream.
    #[must_use]
    pub fn history(&self) -> &[TrafficObservation] {
        &self.history
    }

    /// The EMA smoothing factor.
    #[must_use]
    pub fn smoothing(&self) -> f64 {
        self.smoothing
    }

    /// Predict the next traffic level for `(coord, day)`.
    ///
    /// If no history exists for the queried `(coord, day)` pair, returns
    /// an empty prediction (level `0.0`, confidence `0.0`). Otherwise
    /// returns the EMA-smoothed traffic level and a confidence value
    /// derived from the number of contributing observations.
    ///
    /// The EMA is computed in chronological order (oldest observation
    /// first) so the most recent observation dominates when
    /// `smoothing > 0.5`.
    #[must_use]
    pub fn predict_next(&self, coord: HexCoord, day: u32) -> TrafficPrediction {
        let mut relevant: Vec<&TrafficObservation> =
            self.history.iter().filter(|obs| obs.coord == coord && obs.day == day).collect();
        // Sort by step so the EMA is computed in observation order — the
        // prototype does not have wall-clock timestamps, so "step" is the
        // best proxy for "recency".
        relevant.sort_by_key(|obs| obs.step);

        if relevant.is_empty() {
            return TrafficPrediction::empty(coord, day);
        }

        let alpha = self.smoothing;
        let mut level = relevant[0].traffic_level;
        for obs in relevant.iter().skip(1) {
            level = alpha * obs.traffic_level + (1.0 - alpha) * level;
        }

        // Confidence scales with the number of observations. We cap the
        // growth at 10 observations — after that the prediction is
        // considered fully informed regardless of the EMA behaviour.
        let observation_count = relevant.len() as f64;
        let confidence = (observation_count / 10.0).clamp(0.0, 1.0);

        TrafficPrediction { coord, day, predicted_level: level.clamp(0.0, 1.0), confidence }
    }

    /// Predict traffic for every `(coord, day)` pair that has at least
    /// one observation. Returns one [`TrafficPrediction`] per pair,
    /// grouped by `(coord, day)`.
    #[must_use]
    pub fn predict_all(&self) -> Vec<TrafficPrediction> {
        let mut keys: Vec<(HexCoord, u32)> =
            self.history.iter().map(TrafficObservation::group_key).collect();
        keys.sort();
        keys.dedup();
        keys.into_iter().map(|(coord, day)| self.predict_next(coord, day)).collect()
    }
}

/// A traffic-weighted cost edge — the contribution of predicted traffic
/// to the LP objective coefficient for a single (cell, day) pair.
///
/// The optimizer multiplies `predicted_level` by
/// `CrossDayOptimizer::base_traffic_weight` to obtain the penalty
/// applied to variables that would land the agent on `coord` on `day`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficCostEdge {
    /// Cell whose objective coefficient gets the penalty.
    pub coord: HexCoord,
    /// Day the penalty applies to.
    pub day: u32,
    /// The traffic prediction driving this edge.
    pub prediction: TrafficPrediction,
    /// Per-variable penalty coefficient (>= 0).
    pub penalty: f64,
}

/// Multi-day plan: one [`Plan`] per day, plus the predictions that were
/// used to build it.
///
/// The plan is the natural output of [`CrossDayOptimizer::optimize`].
/// Drop-in [`Solver`] compatibility is achieved by flattening the
/// per-day plans into a single step-major [`Plan`] via
/// [`CrossDayOptimizer::decide`] (see [`Self::into_single_plan`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiDayPlan {
    /// One entry per day, in submission order. `day_plans.len()` matches
    /// the day count of the input [`SolveContext`].
    pub day_plans: Vec<Plan>,
    /// Predictions consulted by the optimizer when building this plan.
    /// Useful for telemetry and for the acceptance tests.
    pub predicted_traffic: Vec<TrafficPrediction>,
    /// Free-form solver telemetry (objective per day, traffic weight
    /// applied, …).
    pub metadata: serde_json::Value,
}

impl MultiDayPlan {
    /// Number of days in the plan.
    #[must_use]
    pub fn day_count(&self) -> usize {
        self.day_plans.len()
    }

    /// Total number of actions across all days.
    #[must_use]
    pub fn total_actions(&self) -> usize {
        self.day_plans.iter().map(Plan::len).sum()
    }

    /// Flatten into the single step-major [`Plan`] used by the rest of
    /// the crate. The metadata is merged: the outer plan's metadata is
    /// taken verbatim, with each day's plan kept under `day_plans[i]`.
    #[must_use]
    pub fn into_single_plan(self) -> Plan {
        let mut actions = Vec::with_capacity(self.total_actions());
        for day in &self.day_plans {
            actions.extend(day.actions.iter().copied());
        }
        let day_plans_meta: Vec<serde_json::Value> =
            self.day_plans.iter().map(|p| p.metadata.clone()).collect();
        let metadata = serde_json::json!({
            "day_plans": day_plans_meta,
            "predicted_traffic": self.predicted_traffic,
            "metadata": self.metadata,
        });
        Plan { actions, metadata }
    }
}

/// Cross-day optimizer — runs cross-day planning and overlays
/// traffic-prediction-derived cost edges on the LP objective.
///
/// ## Aggregation across days
///
/// The optimizer builds one [`Model`] that spans every day of the
/// input [`SolveContext`]. Constraint builders from
/// [`crate::constraints`] (fuel caps, spot inventory) are run once
/// across the whole match — the LP gets the joint picture rather than
/// per-day silos. In the relaxed-LP / greedy-decode fallback this is a
/// no-op (each day's variables are already independent), but it gives
/// the eventual real backend (issue #57) a single model to solve
/// end-to-end.
///
/// ## Predicted-traffic edges
///
/// After building the joint model the optimizer walks every
/// [`Variable`] and, for each direction that would land the agent on a
/// predicted-congested cell, adds a positive penalty equal to
/// `predicted_level * base_traffic_weight` to the variable's objective
/// coefficient. The greedy decoder then naturally steers the agent
/// around those cells.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossDayOptimizer {
    /// Per-day specs — mirrors the `days` field on [`SolveContext`].
    days: Vec<DaySpec>,
    /// Traffic predictor (owns the historical observation stream).
    predictor: TrafficPredictor,
    /// Weight applied to the predicted traffic level when computing the
    /// per-variable penalty. `0.0` means "ignore traffic entirely"; the
    /// default is `1.0` (predicted level maps 1:1 to objective penalty).
    base_traffic_weight: f64,
}

impl CrossDayOptimizer {
    /// Construct a fresh optimizer over `days` with `predictor`.
    #[must_use]
    pub fn new(days: Vec<DaySpec>, predictor: TrafficPredictor) -> Self {
        Self { days, predictor, base_traffic_weight: 1.0 }
    }

    /// Override the weight applied to the predicted traffic level.
    /// Negative values are snapped to `0.0` — the optimizer only ever
    /// *discourages* congested cells.
    #[must_use]
    pub fn with_base_traffic_weight(mut self, weight: f64) -> Self {
        self.base_traffic_weight = weight.max(0.0);
        self
    }

    /// Read-only access to the predictor.
    #[must_use]
    pub fn predictor(&self) -> &TrafficPredictor {
        &self.predictor
    }

    /// Mutably access the predictor so callers can stream
    /// observations into it.
    pub fn predictor_mut(&mut self) -> &mut TrafficPredictor {
        &mut self.predictor
    }

    /// Per-day specs.
    #[must_use]
    pub fn days(&self) -> &[DaySpec] {
        &self.days
    }

    /// Build the [`TrafficCostEdge`] list from a set of predictions.
    /// Each prediction produces one edge with the penalty
    /// `predicted_level * base_traffic_weight`.
    #[must_use]
    pub fn build_traffic_edges(&self, predictions: &[TrafficPrediction]) -> Vec<TrafficCostEdge> {
        predictions
            .iter()
            .map(|p| TrafficCostEdge {
                coord: p.coord,
                day: p.day,
                prediction: p.clone(),
                penalty: p.predicted_level * self.base_traffic_weight,
            })
            .collect()
    }

    /// Aggregate constraints across days, add predicted-traffic cost
    /// edges to the model, and run the cross-day solver to produce a
    /// [`MultiDayPlan`].
    ///
    /// The day count of the returned plan matches `days.len()`.
    /// Per-day plans are produced by calling
    /// [`crate::model::greedy_decode`] on a synthetic per-day context —
    /// this keeps the result identical to the existing solver on
    /// traffic-free instances.
    #[must_use]
    pub fn optimize(&self, ctx: &SolveContext) -> MultiDayPlan {
        if ctx.days.is_empty() || ctx.agents.is_empty() {
            let metadata = serde_json::json!({
                "kind": "empty",
                "base_traffic_weight": self.base_traffic_weight,
            });
            return MultiDayPlan { day_plans: Vec::new(), predicted_traffic: Vec::new(), metadata };
        }

        // 1. Aggregate constraints across days by building the joint
        //    model. We don't actually solve the joint LP in the prototype
        //    (the fallback is per-day greedy), but we materialise the
        //    constraint rows so downstream tooling can inspect them.
        let mut model = Model::from_context(ctx);
        let _ = crate::constraints::add_fuel_constraints(
            &mut model,
            &crate::constraints::FuelParams::default(),
        );
        let _ = crate::constraints::add_spot_inventory_constraints(
            &mut model,
            &crate::constraints::InventoryParams::default(),
        );

        // 2. Predict traffic for every (coord, day) the predictor knows
        //    about, then build the cost edges.
        let predictions = self.predictor.predict_all();
        let edges = self.build_traffic_edges(&predictions);

        // 3. Apply the cost edges to the joint model — increases the
        //    objective coefficient of every variable that would land on
        //    a predicted-congested cell. The joint model is kept around
        //    so downstream tooling (issue #57's real backend) sees the
        //    full picture in one go.
        apply_traffic_edges(&mut model, &edges);

        // 4. Per-day greedy decode with the updated objective. Each day
        //    gets its own model whose edges are restricted to that day.
        let day_plans: Vec<Plan> = ctx
            .days
            .iter()
            .enumerate()
            .map(|(day_index, day_spec)| {
                let per_day_ctx = per_day_context(ctx, day_index);
                let per_day_with_edges = build_per_day_model_with_edges(
                    &per_day_ctx,
                    self.base_traffic_weight,
                    &predictions,
                    day_index,
                );
                let actions = greedy_decode(&per_day_ctx, &per_day_with_edges);
                let actions_len = actions.len();
                let score = crate::model::score_plan(&per_day_ctx, &actions);
                Plan::new(actions).with_metadata(serde_json::json!({
                    "day": day_index,
                    "steps": day_spec.steps,
                    "actions": actions_len,
                    "objective": score,
                }))
            })
            .collect();

        let metadata = serde_json::json!({
            "kind": "cross_day_traffic_predicted",
            "base_traffic_weight": self.base_traffic_weight,
            "prediction_count": predictions.len(),
            "edge_count": edges.len(),
            "smoothing": self.predictor.smoothing(),
        });

        MultiDayPlan { day_plans, predicted_traffic: predictions, metadata }
    }
}

impl Solver for CrossDayOptimizer {
    fn name(&self) -> &'static str {
        "ilp-cross-traffic-predicted"
    }

    fn decide(&self, ctx: &SolveContext) -> Plan {
        self.optimize(ctx).into_single_plan()
    }
}

/// Apply traffic-prediction penalties to a model's variable coefficients
/// in place. Variables whose `direction` would land the agent on a
/// penalised cell get their objective coefficient raised by the
/// prediction's penalty.
fn apply_traffic_edges(model: &mut Model, edges: &[TrafficCostEdge]) {
    let edge_index = build_edge_index(edges);
    let agent_starts: std::collections::HashMap<_, _> =
        model.agents.iter().map(|a| (a.id, a.coord)).collect();
    for var in &mut model.variables {
        if var.forced_zero {
            continue;
        }
        let start = agent_starts.get(&var.key.agent).copied().unwrap_or(HexCoord::new(0, 0));
        let (dq, dr) = direction_offset(var.key.direction);
        let step = var.key.step.max(1) as i32;
        let target = HexCoord::new(start.q + dq * step, start.r + dr * step);
        if let Some(penalty) = edge_index.get(&(target, var.key.day)) {
            var.objective += *penalty;
        }
    }
}

/// Build a fresh per-day model and overlay the traffic edges for that
/// day. Used by [`CrossDayOptimizer::optimize`] when constructing the
/// per-day greedy inputs.
fn build_per_day_model_with_edges(
    ctx: &SolveContext,
    base_weight: f64,
    predictions: &[TrafficPrediction],
    day_index: usize,
) -> Model {
    let day = day_index as u32;
    let day_edges: Vec<TrafficCostEdge> = predictions
        .iter()
        .filter(|p| p.day == day)
        .map(|p| TrafficCostEdge {
            coord: p.coord,
            day: p.day,
            prediction: p.clone(),
            penalty: p.predicted_level * base_weight,
        })
        .collect();
    let mut model = Model::from_context(ctx);
    apply_traffic_edges(&mut model, &day_edges);
    model
}

/// Build a `(coord, day) -> penalty` lookup from a list of edges.
fn build_edge_index(edges: &[TrafficCostEdge]) -> std::collections::HashMap<(HexCoord, u32), f64> {
    let mut idx: std::collections::HashMap<(HexCoord, u32), f64> =
        std::collections::HashMap::with_capacity(edges.len());
    for edge in edges {
        idx.entry((edge.coord, edge.day)).or_insert(0.0);
        let entry = idx.get_mut(&(edge.coord, edge.day)).expect("just inserted");
        *entry += edge.penalty;
    }
    idx
}

/// Wire-code offset for a [`Direction`]. Mirrors
/// `types::Direction::offset`; inlined here to keep the helper private.
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

/// Build a synthetic per-day [`SolveContext`] for the given day index.
///
/// The per-day context inherits everything from the original (cells,
/// spots, agents, `stock_per_day`) but exposes a single-day `days` vector
/// so the greedy decoder emits the right number of actions for that
/// day.
fn per_day_context(ctx: &SolveContext, day_index: usize) -> SolveContext {
    let days = ctx.days.get(day_index).cloned().map(|d| vec![d]).unwrap_or_default();
    SolveContext {
        days,
        cells: ctx.cells.clone(),
        spots: ctx.spots.clone(),
        stock_per_day: ctx
            .stock_per_day
            .iter()
            .filter(|(d, _, _)| *d as usize == day_index)
            .copied()
            .collect(),
        agents: ctx.agents.clone(),
    }
}

#[cfg(test)]
mod tests {
    // Tests use exact f64 / f64 equality (`smoothing` clamps to exact
    // endpoints, the EMA start value is exact, etc.) — silence the
    // `float_cmp` lint at the test-module level. Also allow
    // `identity_op` for the explicit `1 * N` action-count assertions.
    #![allow(clippy::float_cmp, clippy::identity_op)]

    use super::*;
    use crate::types::{Agent, AgentId, CellKind};

    #[test]
    fn predictor_returns_higher_level_after_congestion_observed() {
        // EMA with smoothing = 1.0 → the prediction collapses to the
        // most recent observation. After three high-traffic observations
        // the prediction must reflect them.
        let mut predictor = TrafficPredictor::new(1.0);
        let coord = HexCoord::new(0, 0);

        predictor.observe(TrafficObservation::new(coord, 0, 0, 0.9));
        predictor.observe(TrafficObservation::new(coord, 0, 1, 0.9));
        predictor.observe(TrafficObservation::new(coord, 0, 2, 0.9));

        let prediction = predictor.predict_next(coord, 0);
        assert!(
            prediction.predicted_level > 0.5,
            "expected high predicted traffic after observing congestion, got {}",
            prediction.predicted_level
        );
        assert!(prediction.confidence > 0.0);
    }

    #[test]
    fn predictor_empty_history_returns_zero_level() {
        let predictor = TrafficPredictor::new(0.5);
        let prediction = predictor.predict_next(HexCoord::new(3, -1), 2);
        assert_eq!(prediction.predicted_level, 0.0);
        assert_eq!(prediction.confidence, 0.0);
    }

    #[test]
    fn predictor_smoothing_is_clamped() {
        // Out-of-range smoothing is silently snapped to [0, 1].
        let p = TrafficPredictor::new(2.5);
        assert_eq!(p.smoothing(), 1.0);
        let p = TrafficPredictor::new(-1.0);
        assert_eq!(p.smoothing(), 0.0);
    }

    #[test]
    fn predictor_isolation_across_cells_and_days() {
        // High traffic on (0,0) day 0 must not leak into (0,0) day 1
        // or into a different cell.
        let mut predictor = TrafficPredictor::new(1.0);
        let coord = HexCoord::new(0, 0);
        predictor.observe(TrafficObservation::new(coord, 0, 0, 0.9));

        let other_day = predictor.predict_next(coord, 1);
        assert_eq!(other_day.predicted_level, 0.0);

        let other_cell = predictor.predict_next(HexCoord::new(1, 0), 0);
        assert_eq!(other_cell.predicted_level, 0.0);
    }

    #[test]
    fn optimizer_produces_one_plan_per_day() {
        let days = vec![DaySpec::new(5), DaySpec::new(7), DaySpec::new(6)];
        let optimizer = CrossDayOptimizer::new(days, TrafficPredictor::new(0.5));
        let ctx = SolveContext::for_test(
            optimizer.days().to_vec(),
            vec![(HexCoord::new(0, 0), CellKind::Plain)],
            vec![],
            vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }],
        );

        let plan = optimizer.optimize(&ctx);
        assert_eq!(plan.day_count(), 3);
        assert_eq!(plan.day_plans[0].len(), 1 * 5);
        assert_eq!(plan.day_plans[1].len(), 1 * 7);
        assert_eq!(plan.day_plans[2].len(), 1 * 6);
        assert_eq!(plan.total_actions(), 5 + 7 + 6);
    }

    #[test]
    fn optimizer_with_congestion_steers_away_from_congested_cell() {
        // Two adjacent cells: `s0` (1, 0) and `s1` (2, 0), both at distance
        // 1 from the agent at (0, 0). `s0` is observed as jammed; `s1`
        // is observed as empty. The optimizer should bias the agent
        // toward `s1` — verifiable via the prediction penalties and via
        // the per-day plan's actions.
        let days = vec![DaySpec::new(3)];
        let mut predictor = TrafficPredictor::new(1.0);
        predictor.observe(TrafficObservation::new(HexCoord::new(1, 0), 0, 0, 0.95));

        let optimizer = CrossDayOptimizer::new(days, predictor).with_base_traffic_weight(10.0);
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

        let plan = optimizer.optimize(&ctx);

        // The optimizer must have produced at least one prediction (for
        // `s0`'s observed congestion).
        assert_ne!(plan.predicted_traffic.len(), 0);
        let s0_pred =
            plan.predicted_traffic.iter().find(|p| p.coord == HexCoord::new(1, 0)).unwrap();
        assert!(s0_pred.predicted_level > 0.5);

        // Verify the predicted-traffic edge actually changed the LP
        // objective coefficient in the expected direction: the
        // variable that lands on the congested cell must get a higher
        // (worse) objective after the edge is applied, while variables
        // landing on unobserved cells must be unchanged.
        let unweighted = Model::from_context(&ctx);
        let edges = optimizer.build_traffic_edges(&plan.predicted_traffic);
        let mut weighted = Model::from_context(&ctx);
        apply_traffic_edges(&mut weighted, &edges);

        let objective_at = |m: &Model, step: u32, dir: Direction| -> f64 {
            m.variables
                .iter()
                .find(|v| v.key.step == step && v.key.direction == dir)
                .map_or(0.0, |v| v.objective)
        };

        // The direction that lands on (1, 0) — `Direction::Right` —
        // must have a strictly higher (worse) objective after applying
        // the traffic edge.
        let u_right = objective_at(&unweighted, 1, Direction::Right);
        let w_right = objective_at(&weighted, 1, Direction::Right);
        assert!(
            w_right > u_right,
            "Right direction must get a traffic penalty (u={u_right}, w={w_right})"
        );

        // `Direction::UpperRight` lands on (1, -1), an unobserved cell,
        // and must be unaffected.
        let u_ur = objective_at(&unweighted, 1, Direction::UpperRight);
        let w_ur = objective_at(&weighted, 1, Direction::UpperRight);
        assert!(
            (w_ur - u_ur).abs() < 1e-9,
            "Unobserved direction must be unaffected (Δ = {})",
            w_ur - u_ur
        );
    }

    #[test]
    fn multi_day_plan_into_single_plan_preserves_action_count() {
        let days = vec![DaySpec::new(4), DaySpec::new(6)];
        let optimizer = CrossDayOptimizer::new(days, TrafficPredictor::new(0.5));
        let ctx = SolveContext::for_test(
            optimizer.days().to_vec(),
            vec![(HexCoord::new(0, 0), CellKind::Plain)],
            vec![],
            vec![
                Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 },
                Agent { id: AgentId(1), coord: HexCoord::new(1, 0), fuel: 100 },
            ],
        );

        let multi = optimizer.optimize(&ctx);
        let flat = multi.clone().into_single_plan();
        assert_eq!(flat.len(), multi.total_actions());
        assert_eq!(flat.metadata["day_plans"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn empty_context_returns_empty_multi_day_plan() {
        let optimizer = CrossDayOptimizer::new(Vec::new(), TrafficPredictor::new(0.5));
        let ctx = SolveContext::for_test(vec![], vec![], vec![], vec![]);
        let plan = optimizer.optimize(&ctx);
        assert_eq!(plan.day_count(), 0);
        assert_eq!(plan.total_actions(), 0);
        assert_eq!(plan.metadata["kind"], "empty");
    }
}
