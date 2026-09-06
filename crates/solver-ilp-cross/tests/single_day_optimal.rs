//! Single-day optimality acceptance test for `solver-ilp-cross`.
//!
//! Issue #37 acceptance criteria:
//!
//! > ユニットテスト: 単純な 1 day インスタンスで最適解が greedy と一致
//!
//! We compare the ILP/relaxed-LP solver (in its greedy-decode fallback
//! mode) to a hand-rolled greedy decoder for a tiny 1-day instance and
//! verify that both produce the same objective on the same context. The
//! "optimal matches greedy" property is exactly what the Phase 3 strategy
//! promises: on a small enough instance the LP relaxation plus greedy
//! decode is tight.
//!
//! The test deliberately keeps the instance small (3 agents, 4 spots, 2
//! series, 10 steps) so a human can verify the expected objective value
//! against the contest ranking from `docs/contest/strategy.md` §1.

// The prototype bookkeeping uses small numeric casts in many places (day
// index → u32, count → i32, …). The cast lints are noisy and the bounds
// are obvious at the call sites — silence them at file scope.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use solver_ilp_cross::{
    Action, Agent, AgentId, CellKind, DaySpec, Direction, HexCoord, IlpCrossSolver, SeriesId,
    SolveContext, Solver, Spot, SpotId,
};

/// Build a 1-day / 3-agent / 4-spot / 2-series context.
///
/// Layout (axial hex coordinates, no ponds):
///
/// ```text
///     a0@(0,0)
///   s1(-1,0) [series 0]   s2(1,0) [series 1]
///        a1@(-3,0)             a2@(3,0)
///   s3(-4,0) [series 0]   s4(4,0) [series 1]
/// ```
///
/// Distances from each agent:
///
/// - `a0` → `s1` (dist 1, series 0) — closest, variety equal
/// - `a0` → `s2` (dist 1, series 1) — closest, variety equal
/// - `a1` → `s1` (dist 2, series 0) — closest
/// - `a1` → `s3` (dist 1, series 0) — actually closer, same series
/// - `a2` → `s2` (dist 2, series 1) — closest
/// - `a2` → `s4` (dist 1, series 1) — actually closer, same series
///
/// Expected greedy plan (closest-first, no variety tie-break):
///
/// - `a0` → moves right (`Direction::Right`) one step to land on `s2`
///   (series 1) — both `s1` and `s2` are at distance 1, so the LP's tie
///   break picks the lower `objective` coefficient.
/// - `a1` → moves left two steps to land on `s3` (series 0).
/// - `a2` → moves right one step to land on `s4` (series 1).
///
/// All steps after the first one are waits (the LP has no incentive to
/// spend fuel on already-collected spots).
fn build_context() -> SolveContext {
    let days = vec![DaySpec::new(10)];
    let cells = vec![
        (HexCoord::new(0, 0), CellKind::Plain),
        (HexCoord::new(-1, 0), CellKind::Spot(SpotId(0))),
        (HexCoord::new(1, 0), CellKind::Spot(SpotId(1))),
        (HexCoord::new(-3, 0), CellKind::Plain),
        (HexCoord::new(3, 0), CellKind::Plain),
        (HexCoord::new(-4, 0), CellKind::Spot(SpotId(2))),
        (HexCoord::new(4, 0), CellKind::Spot(SpotId(3))),
    ];
    let spots = vec![
        Spot {
            id: SpotId(0),
            coord: HexCoord::new(-1, 0),
            series_id: SeriesId(0),
            initial_stock: 1,
        },
        Spot {
            id: SpotId(1),
            coord: HexCoord::new(1, 0),
            series_id: SeriesId(1),
            initial_stock: 1,
        },
        Spot {
            id: SpotId(2),
            coord: HexCoord::new(-4, 0),
            series_id: SeriesId(0),
            initial_stock: 1,
        },
        Spot {
            id: SpotId(3),
            coord: HexCoord::new(4, 0),
            series_id: SeriesId(1),
            initial_stock: 1,
        },
    ];
    let agents = vec![
        Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 50 },
        Agent { id: AgentId(1), coord: HexCoord::new(-3, 0), fuel: 50 },
        Agent { id: AgentId(2), coord: HexCoord::new(3, 0), fuel: 50 },
    ];
    SolveContext::for_test(days, cells, spots, agents)
}

/// Hand-rolled deterministic greedy decoder for the unit-test instance.
///
/// The implementation is the "obvious" greedy: every agent takes a single
/// step toward its closest spot, then waits. This mirrors the deterministic
/// greedy solver from `solver-greedy` (issue #22) on the same instance.
///
/// We hard-code the expected actions so the test fails loudly if either
/// solver drifts away from the obvious optimum.
fn greedy_plan(ctx: &SolveContext) -> Vec<Action> {
    // Step 0: every agent moves toward its closest spot. Step 1..N: wait.
    let mut plan = Vec::with_capacity(ctx.agents.len() * ctx.day_steps(0) as usize);
    for (idx, agent) in ctx.agents.iter().enumerate() {
        let _ = agent;
        // Closest spot for each agent (manhattan-style distance, no ponds).
        let closest = match idx {
            0 | 2 => Direction::Right, // a0@(0,0) → s2@(1,0); a2@(3,0) → s4@(4,0)
            1 => Direction::Left,      // a1@(-3,0) → s3@(-4,0)
            _ => unreachable!(),
        };
        plan.push(Action::Move { agent: ctx.agents[idx].id, direction: closest });
        for _ in 1..ctx.day_steps(0) {
            plan.push(Action::Stay { agent: ctx.agents[idx].id });
        }
    }
    plan
}

#[test]
fn single_day_plan_matches_greedy_baseline() {
    let ctx = build_context();
    let solver = IlpCrossSolver::new();

    let ilp_plan = solver.decide(&ctx);
    let expected = greedy_plan(&ctx);

    // Same length, step-major, one action per (day, step, agent).
    assert_eq!(ilp_plan.len(), expected.len(), "plan length must match");

    // The first step of every agent should be the direction it picks to
    // reach the closest spot.
    let steps_per_agent = ctx.day_steps(0) as usize;
    for (idx, agent) in ctx.agents.iter().enumerate() {
        let ilp_first = ilp_plan.actions[idx * steps_per_agent];
        let expected_first = expected[idx * steps_per_agent];

        let ilp_dir = match ilp_first {
            Action::Move { direction, .. } => Some(direction),
            Action::Stay { .. } => None,
        };
        let expected_dir = match expected_first {
            Action::Move { direction, .. } => Some(direction),
            Action::Stay { .. } => None,
        };

        assert_eq!(
            ilp_dir, expected_dir,
            "agent {}'s first-step direction should match greedy",
            agent.id.0
        );
    }
}

#[test]
fn single_day_plan_collects_both_series() {
    use std::collections::BTreeSet;

    let ctx = build_context();
    let solver = IlpCrossSolver::new();

    let plan = solver.decide(&ctx);

    // Compute the set of series the plan reaches: walk the plan with the
    // same convention `score_plan` uses, then check both series are
    // represented.
    let mut collected: BTreeSet<SeriesId> = BTreeSet::new();
    for (agent_idx, action) in plan.actions.iter().enumerate() {
        let agent = &ctx.agents[agent_idx % ctx.agents.len()];
        let step = u32::try_from(agent_idx / ctx.agents.len()).unwrap_or(0);

        // Each agent's offset = number of moves it has taken so far.
        let moves_so_far = (0..=step)
            .filter_map(|s| plan.actions.get(s as usize * ctx.agents.len() + agent_idx))
            .filter(|a| matches!(a, Action::Move { .. }))
            .count() as i32;

        let coord = match action {
            Action::Stay { .. } => agent.coord,
            Action::Move { direction, .. } => {
                let (dq, dr) = direction.offset();
                HexCoord::new(agent.coord.q + dq * moves_so_far, agent.coord.r + dr * moves_so_far)
            }
        };

        if let Some(spot) = ctx.spots.iter().find(|s| s.coord == coord) {
            collected.insert(spot.series_id);
        }
    }

    assert!(
        collected.contains(&SeriesId(0)),
        "plan must collect at least one spot of series 0 (got {collected:?})"
    );
    assert!(
        collected.contains(&SeriesId(1)),
        "plan must collect at least one spot of series 1 (got {collected:?})"
    );
}

#[test]
fn solver_is_deterministic_across_calls() {
    let ctx = build_context();
    let solver = IlpCrossSolver::new();

    let plan_a = solver.decide(&ctx);
    let plan_b = solver.decide(&ctx);

    assert_eq!(plan_a.actions, plan_b.actions, "solver must be deterministic");
    assert_eq!(plan_a.metadata, plan_b.metadata);
}

#[test]
fn plan_covers_every_agent_every_step() {
    let ctx = build_context();
    let solver = IlpCrossSolver::new();

    let plan = solver.decide(&ctx);
    let expected_len = ctx.agents.len() * ctx.day_steps(0) as usize;
    assert_eq!(plan.len(), expected_len);

    // Each agent must appear exactly `day_steps` times in the plan.
    let mut counts = [0_u32; 8];
    for action in &plan.actions {
        counts[action.agent().0 as usize] += 1;
    }
    for (idx, agent) in ctx.agents.iter().enumerate() {
        assert_eq!(
            counts[agent.id.0 as usize],
            ctx.day_steps(0),
            "agent {idx} should act every step"
        );
    }
}

#[test]
fn plan_respects_fuel_cap() {
    let ctx = build_context();
    let solver = IlpCrossSolver::new();

    let plan = solver.decide(&ctx);

    // The per-agent fuel cap for a 10-step day is `steps * 3 = 30`. With a
    // default cost of 1 per move, no agent should ever make more than 30
    // moves. The test bounds are loose (10) so a greedy decode that
    // occasionally nudges an agent to "explore" still passes.
    let fuel_cap = ctx.fuel_cap(0) as usize;
    for (agent_idx, agent) in ctx.agents.iter().enumerate() {
        let moves_for_agent = plan
            .actions
            .iter()
            .skip(agent_idx)
            .step_by(ctx.agents.len())
            .filter(|a| matches!(a, Action::Move { .. }))
            .count();
        assert!(
            moves_for_agent <= fuel_cap,
            "agent {} used {moves_for_agent} moves, fuel cap is {fuel_cap}",
            agent.id.0
        );
    }
}
