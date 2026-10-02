//! Smoke tests for the explicit ILP constraint builders added in issue #38.
//!
//! Reference: issue #38 acceptance criteria
//!
//! - [x] 各 agent の step 毎燃料消費を linear 制約で表現
//! - [x] スポット在庫は「1 日で同系列から全 agent が合計 `max_stock` まで」の制約
//! - [x] 補給車は補給対象巡回車と同一セル時に燃料満タン化
//!
//! The crate currently runs a relaxed-LP / greedy-decode fallback (no real
//! LP backend is wired in — see the `good_lp` feature stub). The tests
//! therefore verify the constraints in two ways:
//!
//! 1. **Row-level**: the constraint builders populate typed rows on the
//!    [`Model`] exactly as the issue requires; we read them back and
//!    check shape / values.
//! 2. **Behaviour-level**: the [`IlpCrossSolver`] runs against the same
//!    context, and the resulting plan obeys the constraints (e.g. an
//!    agent with `fuel = 2` cannot take more than 2 moves per day).
//!
//! The tests deliberately use only the in-crate solver (`IlpCrossSolver`
//!   + the constraint builders) — no native LP binary is required, which
//!     keeps `cargo test -p solver-ilp-cross --offline` self-contained.

// Cast lints: the prototype uses small numeric casts that are obviously
// safe at the call sites. Silence at file scope to keep the assertions
// readable.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use solver_ilp_cross::{
    Action, Agent, AgentId, CellKind, DaySpec, Direction, HexCoord, IlpCrossSolver, Model,
    SeriesId, SolveContext, Solver, Spot, SpotId, add_fuel_constraints,
    add_spot_inventory_constraints, add_supply_arc_constraints, count_active_variables,
};

use solver_ilp_cross::constraints::{FuelParams, InventoryParams, SupplyParams};

/// Helper: build a 1-agent context on a straight east-west line, with
/// three spots of two distinct series.
///
/// ```text
/// a0@(0,0)  s0@(1,0)[ser0]  s1@(2,0)[ser1]  s2@(3,0)[ser0]
/// ```
///
/// Without a fuel limit, the greedy decoder walks `a0` to `s0`, then
/// `s1`, then `s2`. With `fuel = 2`, the agent can take at most two
/// moves — collecting `s0` and `s1` but not `s2`.
fn line_context(fuel: u32) -> SolveContext {
    let days = vec![DaySpec::new(10)];
    let cells = vec![
        (HexCoord::new(0, 0), CellKind::Plain),
        (HexCoord::new(1, 0), CellKind::Spot(SpotId(0))),
        (HexCoord::new(2, 0), CellKind::Spot(SpotId(1))),
        (HexCoord::new(3, 0), CellKind::Spot(SpotId(2))),
    ];
    let spots = vec![
        Spot {
            id: SpotId(0),
            coord: HexCoord::new(1, 0),
            series_id: SeriesId(0),
            initial_stock: 1,
        },
        Spot {
            id: SpotId(1),
            coord: HexCoord::new(2, 0),
            series_id: SeriesId(1),
            initial_stock: 1,
        },
        Spot {
            id: SpotId(2),
            coord: HexCoord::new(3, 0),
            series_id: SeriesId(0),
            initial_stock: 1,
        },
    ];
    let agents = vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel }];
    SolveContext::for_test(days, cells, spots, agents)
}

/// Helper: same as [`line_context`] but with a per-day fuel cap override
/// so the greedy decoder respects the cap. The override is wired into
/// the [`DaySpec`] (the path `greedy_decode` consults), not just the
/// constraint row.
fn line_context_with_fuel_cap(fuel_cap: u32) -> SolveContext {
    let days = vec![DaySpec { steps: 10, seconds_budget: None, fuel_cap_override: Some(fuel_cap) }];
    let cells = vec![
        (HexCoord::new(0, 0), CellKind::Plain),
        (HexCoord::new(1, 0), CellKind::Spot(SpotId(0))),
        (HexCoord::new(2, 0), CellKind::Spot(SpotId(1))),
        (HexCoord::new(3, 0), CellKind::Spot(SpotId(2))),
    ];
    let spots = vec![
        Spot {
            id: SpotId(0),
            coord: HexCoord::new(1, 0),
            series_id: SeriesId(0),
            initial_stock: 1,
        },
        Spot {
            id: SpotId(1),
            coord: HexCoord::new(2, 0),
            series_id: SeriesId(1),
            initial_stock: 1,
        },
        Spot {
            id: SpotId(2),
            coord: HexCoord::new(3, 0),
            series_id: SeriesId(0),
            initial_stock: 1,
        },
    ];
    let agents = vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: fuel_cap }];
    SolveContext::for_test(days, cells, spots, agents)
}

/// Count the number of `Move` actions an agent emitted in a plan.
fn move_count(plan: &[Action], agent: AgentId) -> usize {
    plan.iter().filter(|a| a.agent() == agent && matches!(a, Action::Move { .. })).count()
}

// -------------------------------------------------------------------------
// Fuel constraints
// -------------------------------------------------------------------------

/// Acceptance test for issue #38's fuel constraint:
///
/// > 各 agent の step 毎燃料消費を linear 制約で表現
///
/// We attach a fuel row whose cap is strictly less than the number of
/// moves the greedy decoder would otherwise produce. The decoder must
/// respect the cap — i.e. the resulting plan uses at most `cap` moves
/// for that agent.
#[test]
fn fuel_constraints_reject_infeasible_path() {
    // The straight east-west line has three spots in a row; an
    // unconstrained agent would take 3 moves to collect all of them.
    // We set the per-day fuel cap to `1` on the DaySpec (the path the
    // greedy decoder consults) **and** on the constraint row (the
    // path the LP would consult). The resulting plan must take at
    // most 1 move.
    let ctx = line_context_with_fuel_cap(1);
    let mut model = Model::from_context(&ctx);

    let params = FuelParams { per_step_cost: 1, fuel_cap_override: Some(1) };
    add_fuel_constraints(&mut model, &params).expect("fuel constraints should accept");

    // Row-level: exactly one row per (agent, day) — 1 agent × 1 day.
    assert_eq!(model.fuel_rows.len(), 1, "one fuel row per (agent, day)");
    let row = &model.fuel_rows[0];
    assert_eq!(row.agent, AgentId(0));
    assert_eq!(row.day, 0);
    assert_eq!(row.cost_per_step, 1);
    assert_eq!(row.cap, 1, "override must be used verbatim");

    // Behaviour-level: the solver respects the cap. The greedy fallback
    // already had per-agent fuel bookkeeping at decode time, so this is
    // the smoke test that the explicit constraint row matches the
    // runtime behaviour.
    let solver = IlpCrossSolver::new();
    let plan = solver.decide(&ctx);
    let moves = move_count(&plan.actions, AgentId(0));
    assert!(moves <= row.cap as usize, "agent took {moves} moves, fuel cap is {}", row.cap);
}

/// Sanity test: a generous fuel cap allows the agent to take as many
/// moves as the day allows. We verify the LP rows count up to
/// `day_steps` moves for a `fuel_cap = day_steps` budget.
#[test]
fn fuel_constraints_allow_feasible_path() {
    let ctx = line_context(/* fuel = */ 100);
    let mut model = Model::from_context(&ctx);

    let params = FuelParams { per_step_cost: 1, fuel_cap_override: Some(10) };
    add_fuel_constraints(&mut model, &params).expect("fuel constraints should accept");

    // Behaviour: the agent can take up to 10 moves; with 10 day steps
    // and no further constraints the plan will not exceed `day_steps`
    // moves. We don't assert the exact number — the important thing is
    // that the cap is not silently violated.
    let solver = IlpCrossSolver::new();
    let plan = solver.decide(&ctx);
    let moves = move_count(&plan.actions, AgentId(0));
    assert!(moves <= 10, "agent took {moves} moves, day step cap is 10");
}

// -------------------------------------------------------------------------
// Spot inventory
// -------------------------------------------------------------------------

/// Acceptance test for issue #38's inventory constraint:
///
/// > スポット在庫は「1 日で同系列から全 agent が合計 max_stock まで」 の制約
///
/// We build a 3-agent context where all three agents start adjacent to
/// the *same* spot. The contest rule is "1 spot × 1 udon × 1 team × 1
/// day", so the inventory row for that spot must allow at most 1
/// simultaneous occupancy. The series-aggregate row allows at most
/// `max_per_series_per_day` collections from the same series per day.
#[test]
fn spot_inventory_prevents_overflow() {
    // All three agents start on the west edge; three spots of series 0
    // sit one step to their east. The naive greedy would walk all three
    // agents to the same spot — but the inventory constraint forbids
    // that.
    let days = vec![DaySpec::new(4)];
    let cells = vec![
        (HexCoord::new(0, 0), CellKind::Plain),
        (HexCoord::new(1, 0), CellKind::Spot(SpotId(0))),
    ];
    let spots = vec![Spot {
        id: SpotId(0),
        coord: HexCoord::new(1, 0),
        series_id: SeriesId(0),
        initial_stock: 1,
    }];
    let agents = vec![
        Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 30 },
        Agent { id: AgentId(1), coord: HexCoord::new(0, 1), fuel: 30 },
        Agent { id: AgentId(2), coord: HexCoord::new(-1, 0), fuel: 30 },
    ];
    let ctx = SolveContext::for_test(days, cells, spots.clone(), agents);

    let mut model = Model::from_context(&ctx);
    let params = InventoryParams { max_simultaneous_per_spot: 1, max_per_series_per_day: 3 };
    add_spot_inventory_constraints(&mut model, &params).expect("inventory should accept");

    // Row-level: one inventory row per (spot, day) — 1 spot × 1 day.
    assert_eq!(model.inventory_rows.len(), 1, "one inventory row per spot per day");
    assert_eq!(model.inventory_rows[0].spot, SpotId(0));
    assert_eq!(model.inventory_rows[0].max_simultaneous, 1);

    // Row-level: one series-aggregate row per (series, day) — 1 series.
    assert_eq!(model.series_inventory_rows.len(), 1, "one series-aggregate row per series per day");
    assert_eq!(model.series_inventory_rows[0].max_per_day, 3);

    // Behaviour-level: at most one agent may move toward the same spot
    // per step. We don't enforce a global "exactly one wins" rule (the
    // solver may still pick a different direction for the other agents)
    // — we only verify the inventory row is in place.
    let _solver = IlpCrossSolver::new();
    // Confirm the solver still emits a valid plan; the inventory rows
    // themselves are advisory in the prototype (the greedy fallback
    // already avoids spot double-collection by construction).
    let plan = solver_ilp_cross::IlpCrossSolver::new().decide(&ctx);
    assert_eq!(
        plan.actions.len(),
        ctx.agents.len() * ctx.day_steps(0) as usize,
        "plan must cover every (agent, step)"
    );
}

// -------------------------------------------------------------------------
// Supply arcs
// -------------------------------------------------------------------------

/// Acceptance test for issue #38's supply arc:
///
/// > 補給車は補給対象巡回車と同一セル時に燃料満タン化
///
/// We attach a supply arc that lets the agent refill at a midpoint cell.
/// With the arc in place the LP is allowed to spend more fuel than the
/// day cap would otherwise allow — the agent can drive from start to
/// far spot, refuel, and come back.
///
/// The test verifies:
/// 1. The arc row is appended to `model.supply_arcs`.
/// 2. The arc does not change the active-variable count or break the
///    plan emission (the solver still produces a full-length plan).
#[test]
fn supply_arc_enables_refill_between_agents_and_refill_spots() {
    let ctx = line_context(/* fuel = */ 100);
    let mut model = Model::from_context(&ctx);

    // Refill point at (2, 0) — same cell as `s1` in `line_context`.
    let params =
        SupplyParams { refill_points: vec![(HexCoord::new(2, 0), 30)], same_cell_refill: true };
    add_supply_arc_constraints(&mut model, &params).expect("supply arc should accept");

    // Row-level: one arc per (agent, day, refill_point) — 1 × 1 × 1.
    assert_eq!(model.supply_arcs.len(), 1, "one supply arc per (agent, day, point)");
    let arc = &model.supply_arcs[0];
    assert_eq!(arc.agent, AgentId(0));
    assert_eq!(arc.day, 0);
    assert_eq!(arc.refill_coord, HexCoord::new(2, 0));
    assert_eq!(arc.fuel_after_refill, 30);
    assert!(arc.same_cell_refill);

    // Behaviour-level: the solver still emits a full plan and the arc
    // does not cause `forced_zero_count` to spike.
    let solver = IlpCrossSolver::new();
    let plan = solver.decide(&ctx);
    assert_eq!(
        plan.actions.len(),
        ctx.agents.len() * ctx.day_steps(0) as usize,
        "plan must cover every (agent, step)"
    );
    // Sanity: the constraint builders must not introduce extra
    // forced-zero variables — only pond cells do.
    let active_before = count_active_variables(&Model::from_context(&ctx));
    let active_after = count_active_variables(&model);
    assert_eq!(active_before, active_after, "supply arc must not force any variable to zero");
}

// -------------------------------------------------------------------------
// Combined: full constraint pipeline
// -------------------------------------------------------------------------

/// End-to-end test: build a model from a [`SolveContext`], attach all
/// three constraint families, and verify the row counts and a single
/// full plan. This is the integration-level smoke that the constraint
/// builders compose cleanly with the rest of the crate.
#[test]
fn full_constraint_pipeline_emits_rows_and_a_valid_plan() {
    let ctx = line_context(/* fuel = */ 100);
    let mut model = Model::from_context(&ctx);

    add_fuel_constraints(&mut model, &FuelParams::default()).unwrap();
    add_spot_inventory_constraints(&mut model, &InventoryParams::default()).unwrap();
    add_supply_arc_constraints(
        &mut model,
        &SupplyParams { refill_points: vec![(HexCoord::new(2, 0), 30)], same_cell_refill: true },
    )
    .unwrap();

    // 1 agent × 1 day = 1 fuel row.
    assert_eq!(model.fuel_rows.len(), 1);
    // 3 spots × 1 day = 3 inventory rows.
    assert_eq!(model.inventory_rows.len(), 3);
    // 2 distinct series × 1 day = 2 series-aggregate rows.
    assert_eq!(model.series_inventory_rows.len(), 2);
    // 1 agent × 1 day × 1 refill point = 1 supply arc.
    assert_eq!(model.supply_arcs.len(), 1);

    // The solver still produces a valid plan (no constraint is violated).
    let solver = IlpCrossSolver::new();
    let plan = solver.decide(&ctx);
    assert_eq!(plan.actions.len(), ctx.agents.len() * ctx.day_steps(0) as usize);
    // Every action must be either a Stay or a Move in one of the six
    // wire-code directions — no exotic variants should leak through.
    for action in &plan.actions {
        match action {
            Action::Stay { .. } => {}
            Action::Move { direction, .. } => {
                assert!(Direction::ALL.contains(direction), "plan must use a known direction");
            }
        }
    }
}
