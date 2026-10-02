//! Integration smoke test for `solver-greedy`.
//!
//! Scenario (matches issue #22 acceptance criteria):
//! - 3 agents.
//! - 6 spots.
//! - 2 udon series (`series_id = 0` and `series_id = 1`), with series 0
//!   over-represented in the candidate pool so the variety-first
//!   tie-break has to fire.
//!
//! We only assert **structural** invariants here:
//! - every call returns one `Action` per agent,
//! - chosen actions respect availability (`stock > 0`) and fuel,
//! - when distances tie, the agent picks the *less-represented* series.
//!
//! The deeper correctness is exercised by the unit tests inside
//! `src/greedy.rs`; this file is the "skeleton wired into the bench shape"
//! acceptance gate.

use solver_greedy::{
    Action, Agent, AgentId, GreedySolver, HexCoord, SolveContext, Solver, Spot, SpotId,
};

fn spot(id: u16, q: i32, r: i32, series: u16, stock: u32) -> Spot {
    Spot { id: SpotId(id), coord: HexCoord::new(q, r), series_id: series, stock }
}

fn agent(id: u8, q: i32, r: i32, fuel: u32) -> Agent {
    Agent { id: AgentId(id), coord: HexCoord::new(q, r), fuel }
}

/// Smoke test: 3 agents, 6 spots, 2 series.
///
/// Map layout (axial hex coordinates):
///
/// ```text
///      (-1,0) s1 [s0]   (1,0) s2 [s1]
///  (-3,0) s4 [s0]  a0@(0,0)  (-4,0) s5 [s0]
///      (0,1) s3 [s0]
///                       (3,-3) s6 [s1]
///               a1@(-3,0)               a2@(3,-3)
/// ```
///
/// Distilled:
/// - Agents: a0 @ (0,0), a1 @ (-3,0), a2 @ (3,-3).
/// - Spots:
///   - s1(-1,0) series 0  — distance 1 from a0
///   - s2( 1,0) series 1  — distance 1 from a0 (variety tie-break picks this)
///   - s3( 0,1) series 0  — distance 1 from a0 (extra series 0 rep)
///   - s4(-3,0) series 0  — distance 0 from a1
///   - s5(-4,0) series 0  — distance 1 from a1 (extra series 0 rep)
///   - s6( 3,-3) series 1 — distance 0 from a2
///
/// Pool composition: series 0 = 4 spots, series 1 = 2 spots.
///
/// Expected Phase 1 greedy picks:
/// - a0 → s2 (distance 1, series 1 — *less represented* than series 0)
/// - a1 → s4 (distance 0, series 0 — on a1's own cell)
/// - a2 → s6 (distance 0, series 1 — on a2's own cell)
#[test]
fn smoke_3_agents_6_spots_2_series() {
    let agents = vec![
        agent(0, 0, 0, 10),  // a0 — central
        agent(1, -3, 0, 10), // a1 — far west
        agent(2, 3, -3, 10), // a2 — far south-east
    ];
    let spots = vec![
        spot(1, -1, 0, 0, 1), // s1 — distance 1 from a0, series 0
        spot(2, 1, 0, 1, 1),  // s2 — distance 1 from a0, series 1 (variety pick)
        spot(3, 0, 1, 0, 1),  // s3 — distance 1 from a0, series 0
        spot(4, -3, 0, 0, 1), // s4 — distance 0 from a1, series 0
        spot(5, -4, 0, 0, 1), // s5 — distance 1 from a1, series 0
        spot(6, 3, -3, 1, 1), // s6 — distance 0 from a2, series 1
    ];

    let ctx = SolveContext::for_test(0, agents.clone(), spots.clone());
    let actions = GreedySolver::new().decide(&ctx);

    // Structural: exactly one action per agent, in agent order.
    assert_eq!(actions.len(), agents.len(), "one action per agent");
    for (idx, action) in actions.iter().enumerate() {
        assert_eq!(action.agent(), agents[idx].id, "agent order preserved");
    }

    // Every action either targets an available spot or is a Stay.
    for action in &actions {
        match action {
            Action::Stay { .. } => {}
            Action::MoveToward { spot, .. } => {
                let s = spots
                    .iter()
                    .find(|s| s.id == *spot)
                    .expect("MoveToward must reference a known spot");
                assert!(s.is_available(), "must not move toward an empty spot");
            }
        }
    }

    // Variety-first tie-break: a0 has three distance-1 candidates (s1, s2,
    // s3). Pool has series 0 = 4, series 1 = 2. Solver must prefer the
    // less-represented series, i.e. series 1 = s2.
    assert_eq!(
        actions[0],
        Action::MoveToward { agent: AgentId(0), spot: SpotId(2) },
        "a0 should pick the variety-first winner s2 (series 1)"
    );

    // a1 sits on s4 (distance 0).
    assert_eq!(
        actions[1],
        Action::MoveToward { agent: AgentId(1), spot: SpotId(4) },
        "a1 should pick the spot on its own cell"
    );

    // a2 sits on s6 (distance 0).
    assert_eq!(
        actions[2],
        Action::MoveToward { agent: AgentId(2), spot: SpotId(6) },
        "a2 should pick the spot on its own cell"
    );
}

/// Determinism: running the solver twice on the same context yields the
/// exact same actions. Important for replay + bench reproducibility.
#[test]
fn decide_is_deterministic() {
    let ctx = SolveContext::for_test(
        7,
        vec![agent(0, 0, 0, 10), agent(1, 1, 0, 10), agent(2, 0, 1, 10)],
        vec![spot(0, 0, 0, 0, 1), spot(1, 1, 1, 1, 1), spot(2, -1, 0, 1, 1)],
    );
    let a1 = GreedySolver::new().decide(&ctx);
    let a2 = GreedySolver::new().decide(&ctx);
    assert_eq!(a1, a2);
}

/// End-of-day (all stock gone) ⇒ every agent stays.
#[test]
fn empty_stock_everyone_stays() {
    let ctx = SolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 10), agent(1, 1, 0, 10), agent(2, 0, 1, 10)],
        vec![spot(0, 0, 0, 0, 0), spot(1, 1, 1, 1, 0), spot(2, -1, 0, 1, 0)],
    );
    let actions = GreedySolver::new().decide(&ctx);
    for a in actions {
        assert!(matches!(a, Action::Stay { .. }), "expected Stay, got {a:?}");
    }
}

#[test]
fn assignments_respect_stock_and_spread_series() {
    let ctx = SolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 10), agent(1, 0, 0, 10), agent(2, 0, 0, 10)],
        vec![spot(0, 1, 0, 0, 1), spot(1, -1, 0, 1, 1), spot(2, 0, 1, 2, 1)],
    );
    let actions = GreedySolver::new().decide(&ctx);
    assert_eq!(
        actions,
        vec![
            Action::MoveToward { agent: AgentId(0), spot: SpotId(0) },
            Action::MoveToward { agent: AgentId(1), spot: SpotId(1) },
            Action::MoveToward { agent: AgentId(2), spot: SpotId(2) },
        ]
    );
}

#[test]
fn reservations_are_local_and_exhausted_stock_causes_wait() {
    let ctx = SolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 0), agent(1, 0, 0, 10), agent(2, 0, 0, 10)],
        vec![spot(0, 1, 0, 0, 1)],
    );
    let original = ctx.clone();
    let actions = GreedySolver::new().decide(&ctx);
    assert_eq!(
        actions,
        vec![
            Action::Stay { agent: AgentId(0) },
            Action::MoveToward { agent: AgentId(1), spot: SpotId(0) },
            Action::Stay { agent: AgentId(2) },
        ]
    );
    assert_eq!(ctx, original);
    assert_eq!(GreedySolver::new().decide(&ctx), actions);
}

#[test]
fn assigned_series_breaks_ties_before_pool_frequency() {
    let ctx = SolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 10), agent(1, 0, 0, 10)],
        vec![spot(0, 1, 0, 0, 3), spot(1, -1, 0, 1, 3), spot(2, 2, 0, 1, 3)],
    );
    let actions = GreedySolver::new().decide(&ctx);
    assert_eq!(actions[0], Action::MoveToward { agent: AgentId(0), spot: SpotId(0) });
    assert_eq!(actions[1], Action::MoveToward { agent: AgentId(1), spot: SpotId(1) });
}

fn independent_baseline(ctx: &SolveContext) -> Vec<Action> {
    ctx.agents
        .iter()
        .map(|agent| {
            if !agent.can_move() {
                return Action::Stay { agent: agent.id };
            }
            ctx.spots
                .iter()
                .filter(|spot| spot.is_available())
                .min_by_key(|spot| {
                    let frequency = ctx
                        .spots
                        .iter()
                        .filter(|other| other.is_available() && other.series_id == spot.series_id)
                        .count();
                    (agent.coord.distance(spot.coord), frequency, spot.series_id, spot.id.0)
                })
                .map_or(Action::Stay { agent: agent.id }, |spot| Action::MoveToward {
                    agent: agent.id,
                    spot: spot.id,
                })
        })
        .collect()
}

fn assignment_score(ctx: &SolveContext, actions: &[Action]) -> (usize, u32) {
    let mut remaining = ctx.spots.clone();
    let mut series = std::collections::BTreeSet::new();
    let mut served = 0;
    for action in actions {
        if let Action::MoveToward { spot, .. } = action {
            let target = remaining.iter_mut().find(|candidate| candidate.id == *spot).unwrap();
            if target.stock > 0 {
                target.stock -= 1;
                series.insert(target.series_id);
                served += 1;
            }
        }
    }
    (series.len(), served)
}

#[test]
fn compare_colocated_assignment_suite() {
    let mut baseline_total = (0, 0);
    let mut coordinated_total = (0, 0);
    let mut cases = 0;
    for count in 3..=8 {
        for series_count in 1..=6 {
            for stock in [1, 2, 8] {
                let agents = (0..count).map(|id| agent(id, 0, 0, 10)).collect();
                let coords = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
                let spots = (0_u16..6)
                    .zip(coords)
                    .map(|(id, (q, r))| spot(id, q, r, id % series_count, stock))
                    .collect();
                let ctx = SolveContext::for_test(0, agents, spots);
                let baseline = assignment_score(&ctx, &independent_baseline(&ctx));
                let actions = GreedySolver::new().decide(&ctx);
                let coordinated = assignment_score(&ctx, &actions);
                assert!(coordinated >= baseline);
                let assigned =
                    actions.iter().filter(|a| matches!(a, Action::MoveToward { .. })).count();
                assert_eq!(usize::try_from(coordinated.1).unwrap(), assigned);
                for target in &ctx.spots {
                    let reservations = actions
                        .iter()
                        .filter(
                            |a| matches!(a, Action::MoveToward { spot, .. } if *spot == target.id),
                        )
                        .count();
                    assert!(reservations <= usize::try_from(target.stock).unwrap());
                }
                assert_eq!(
                    coordinated.0,
                    usize::from(count.min(u8::try_from(series_count).unwrap()))
                );
                baseline_total.0 += baseline.0;
                baseline_total.1 += baseline.1;
                coordinated_total.0 += coordinated.0;
                coordinated_total.1 += coordinated.1;
                cases += 1;
            }
        }
    }
    assert!(coordinated_total > baseline_total);
    println!(
        "assignment-only cases={cases} baseline={baseline_total:?} coordinated={coordinated_total:?}"
    );
}

#[test]
fn overlapping_agents_are_stable() {
    let ctx = SolveContext::for_test(
        0,
        vec![agent(0, 0, 0, 10), agent(1, 0, 0, 10), agent(2, 0, 0, 10)],
        vec![spot(0, 1, 0, 0, 1), spot(1, -1, 0, 1, 1), spot(2, 0, 1, 0, 1), spot(3, 0, -1, 1, 1)],
    );
    let actions = GreedySolver::new().decide(&ctx);
    assert_eq!(actions.len(), 3);
    // Every action must reference one of the four known spots.
    for a in &actions {
        if let Action::MoveToward { spot, .. } = a {
            assert!(
                *spot == SpotId(0)
                    || *spot == SpotId(1)
                    || *spot == SpotId(2)
                    || *spot == SpotId(3),
                "unexpected spot id: {spot:?}"
            );
        }
    }
}
