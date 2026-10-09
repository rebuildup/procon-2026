use bench::CandidateRegistry;
use domain::{MatchId, PlayerId};
use engine::MatchSim;
use engine::model::{
    ActionKind, Agent, AgentActions, Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState,
    SolverContext, Spot, Terrain,
};

fn setting(fuel: u32) -> MatchSetting {
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
        match_id: MatchId::new("coverage-test").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(8, 8).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 1), fuel),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(0, 2)),
        ],
        day_steps: 16,
        num_days: 2,
        day_steps_by_day: Vec::new(),
    }
}

#[test]
fn coverage_replans_daily_and_harvests_after_covering_series() {
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let result = MatchSim::default().run(setting(32), solver.as_ref()).unwrap();
    assert_eq!((result.variety, result.daily_variety_sum, result.total_balls), (3, 6, 8));
    assert_eq!(result.illegal_action_count, 0);
}

#[test]
fn coverage_uses_supply_contact_to_continue_beyond_initial_fuel() {
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let result = MatchSim::default().run(setting(2), solver.as_ref()).unwrap();
    assert_eq!((result.variety, result.daily_variety_sum, result.total_balls), (3, 6, 8));
    assert_eq!(result.fuel_short_count, 0);
}

#[test]
fn coverage_does_not_depend_on_offline_only_series_history() {
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .expect("registered coverage candidate")
        .build_solver()
        .unwrap();
    let mut state = MatchState::new(setting(32)).unwrap();
    let original = solver.decide(&SolverContext::from_state(&state));
    state.series_taken.extend([1, 2, 3]);
    assert_eq!(solver.decide(&SolverContext::from_state(&state)), original);
}

#[test]
fn coverage_official_normalization_preserves_every_action() {
    use bench::adapters::walk::BudgetMode;
    use bench::case::CaseSpec;
    use bench::kinds::{parse_kinds, resolve_kinds};
    use engine::model::AgentActions;
    let case: CaseSpec = serde_json::from_str(include_str!("fixtures/arena-701.json")).unwrap();
    for pattern in ["P,S", "P,S,S,P,S"] {
        let kinds = resolve_kinds(Some(&parse_kinds(pattern).unwrap()), case.setting.agents.len());
        let setting = case
            .to_engine_setting_with_kinds(&PlayerId::new("player-a").unwrap(), Some(&kinds))
            .unwrap();
        let mut state = MatchState::new(setting).unwrap();
        let solver = CandidateRegistry::builtin()
            .build("coverage:budget=official")
            .unwrap()
            .build_solver()
            .unwrap();
        let sim = MatchSim::new(case.sim_config().unwrap());
        while !state.is_finished() {
            let ctx = SolverContext::from_state(&state);
            let plan = solver.decide(&ctx);
            assert_eq!(
                BudgetMode::Official.apply(&ctx, plan.clone()),
                plan,
                "normalization changed day {} {pattern}",
                state.day
            );
            for chunk in plan.actions.chunks(kinds.len()) {
                sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
            }
        }
        assert_eq!(state.illegal_action_count, 0);
    }
}

/// Minimal repro for the pre-#342 clock bug (Issue #342 step 1B, decomposition
/// stage B). Under the official tick clock a `Move` occupies its origin
/// terrain's step count, but the old planner re-decided **every element** from
/// the still-stale pre-landing `pos`:
///
/// * each in-flight element emitted a filler `Wait` (a real, billable wire
///   wait once the answer is run-length encoded);
/// * duplicate queued moves from the stale cell overshot targets;
/// * `route.cost <= day_steps - used[]` starved long legs because `used[]`
///   double-counted in-flight moves — on 701 day 0 the plan was
///   31/46/5/7/9 moves with 83 of the 146 weighted units burned as idle
///   (element-clock planner: 121/120/99/103/103 moves, budget spent exactly).
///
/// Repro map: two spots in one row, a 12-step day (6 plain moves of budget).
/// The re-tuned planner decides only at landing ticks and packs the answer as
/// one consecutive movement sequence: the patrol walks to A (2 moves), then
/// B (4 moves), picks both, and only then idles out the remaining budget. The
/// old loop starved after A: duplicate moves overshot it and the inflated
/// `used[]` made the 8-weighted B route exceed the shrinking remainder, so
/// the patrol ended at (3, 0) with one ball and interleaved waits.
#[test]
fn planner_decides_from_landing_cells_and_packs_the_tick_budget() {
    let cells = (0..8)
        .flat_map(|row| {
            (0..8).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = if row == 0 && col == 2 {
                    Some(Spot { series: 1, pos, max_stock: 1, current_stock: 1 })
                } else if row == 0 && col == 6 {
                    Some(Spot { series: 2, pos, max_stock: 1, current_stock: 1 })
                } else {
                    None
                };
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    let setting = MatchSetting {
        match_id: MatchId::new("coverage-clock-repro").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(8, 8).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 12),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::patrol_with_fuel(2, HexCoord::new(0, 2), 12),
        ],
        day_steps: 12,
        num_days: 1,
        day_steps_by_day: vec![12],
    };
    let state = MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .unwrap()
        .build_solver()
        .unwrap();
    let ctx = SolverContext::from_state(&state);
    let plan = solver.decide(&ctx);

    // The patrol's day plan is one packed movement sequence: every `Move`
    // precedes every `Wait`. Under the old per-element loop the same day
    // interleaved waits between the moves (each in-flight element re-decided
    // from the stale cell emitted a billable filler wait).
    let n = ctx.agent_ids.len();
    let patrol: Vec<ActionKind> = plan.actions.iter().step_by(n).map(|a| a.kind).collect();
    let moves = patrol.iter().filter(|k| matches!(k, ActionKind::Move(_))).count();
    let first_wait = patrol.iter().position(|k| matches!(k, ActionKind::Wait));
    assert_eq!(moves, 6, "the 12-step budget pays for exactly the 6 plain moves to both spots");
    assert_eq!(
        first_wait,
        Some(moves),
        "waits must trail the movement sequence, never split it (the old clock bug): \
         got {patrol:?}"
    );

    // The packed plan is already the official wire shape: re-normalizing is a
    // no-op (normalize drops no planned move, Issue #342 AC).
    assert_eq!(
        bench::adapters::walk::BudgetMode::Official.apply(&ctx, plan.clone()),
        plan,
        "normalize fixed point"
    );

    // And the plan actually lands the patrol on both spots and picks both
    // balls within the day (the old interleaved version overshot spot A,
    // starved the leg to B, and ended at (3, 0) with one ball).
    let sim = MatchSim::default();
    let mut run = state.clone();
    for chunk in plan.actions.chunks(n) {
        sim.step(&mut run, &AgentActions::new(chunk.to_vec()));
    }
    let patrol_end = run.agents.iter().find(|a| a.id == 0).unwrap();
    assert_eq!(patrol_end.pos, HexCoord::new(6, 0), "the patrol reaches the far spot");
    assert_eq!(patrol_end.total_inventory(), 2, "both spots acquired within the day");
    assert_eq!(run.illegal_action_count, 0);
}

/// Long-day board used to exercise the park-spreading gate (Issue #331):
/// Two series sit in opposite corners so a single patrol must choose a
/// target order; `day_claimed` spreading flips that order on large maps.
fn long_day_setting(edge: u8) -> MatchSetting {
    let e = i32::from(edge);
    let cells = (0..e)
        .flat_map(|row| {
            (0..e).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = if pos == HexCoord::new(e - 1, 0) {
                    Some(Spot { series: 1, pos, max_stock: 3, current_stock: 3 })
                } else if pos == HexCoord::new(0, e - 1) {
                    Some(Spot { series: 2, pos, max_stock: 3, current_stock: 3 })
                } else {
                    None
                };
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    MatchSetting {
        match_id: MatchId::new("coverage-gate-test").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(edge, edge).unwrap(), cells).unwrap(),
        agents: vec![
            Agent::patrol(0, HexCoord::new(0, 0), 80, 3),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(0, 2)),
        ],
        day_steps: 80,
        num_days: 2,
        day_steps_by_day: vec![80, 80],
    }
}

fn first_day_plan(setting: MatchSetting, candidate: &str) -> engine::model::AgentActions {
    let state = MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin().build(candidate).unwrap().build_solver().unwrap();
    solver.decide(&SolverContext::from_state(&state))
}

/// Park spreading is gated to maps whose edge reaches 24: on small boards
/// nearest-first already covers every series, so `park=far` must not
/// distort positioning (Issue #331).
#[test]
fn park_far_matches_park_none_on_small_maps() {
    let none = first_day_plan(long_day_setting(8), "coverage:budget=official:park=none");
    let far = first_day_plan(long_day_setting(8), "coverage:budget=official:park=far");
    assert_eq!(far, none, "park=far must stay byte-identical to park=none when width/height < 24");
}

/// Post-#342 the spreading window's penalty is subsumed: the re-tuned
/// planner keeps series claims day-scoped for **both** park modes, where the
/// pre-parity per-element loop let `park=none` converge on already-claimed
/// series (the divergence this test used to pin was clock-induced). The two
/// modes now coincide on every board — pinned here on the largest spreading
/// window so a future claim-policy split cannot silently re-introduce the
/// convergence defect without being a deliberate choice.
#[test]
fn park_far_matches_park_none_under_the_spreading_window() {
    let none = first_day_plan(long_day_setting(24), "coverage:budget=official:park=none");
    let far = first_day_plan(long_day_setting(24), "coverage:budget=official:park=far");
    assert_eq!(far, none, "park=far shares the re-tuned claim policy with park=none");
}

/// The spreading itself (#331 現象): on a 24-edge board with both patrols
/// nearest the same corner spot, the day-scoped claim set must send the
/// second patrol to the *other* unclaimed series instead of converging on
/// its teammate's target. Under the pre-#342 clock this spreading only
/// existed for `park=far`; the re-tuned planner keeps it for both modes.
#[test]
fn series_claims_spread_patrols_across_unclaimed_series() {
    let e = i32::from(24u8);
    let cells = (0..e)
        .flat_map(|row| {
            (0..e).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = if pos == HexCoord::new(e - 1, 0) {
                    Some(Spot { series: 1, pos, max_stock: 1, current_stock: 1 })
                } else if pos == HexCoord::new(0, e - 1) {
                    Some(Spot { series: 2, pos, max_stock: 1, current_stock: 1 })
                } else {
                    None
                };
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect();
    let setting = MatchSetting {
        match_id: MatchId::new("coverage-spread-test").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(MapSize::new(24, 24).unwrap(), cells).unwrap(),
        // Both patrols sit nearest the (23, 0) spot; without the claim
        // policy the second one would chase the same series.
        agents: vec![
            Agent::patrol(0, HexCoord::new(0, 0), 200, 3),
            Agent::patrol(1, HexCoord::new(1, 0), 200, 3),
            Agent::supply(2, HexCoord::new(0, 1)),
            Agent::supply(3, HexCoord::new(0, 2)),
        ],
        day_steps: 80,
        num_days: 1,
        day_steps_by_day: vec![80],
    };
    let state = MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin()
        .build("coverage:budget=official")
        .unwrap()
        .build_solver()
        .unwrap();
    let sim = MatchSim::default();
    let mut run_state = state.clone();
    let plan = solver.decide(&SolverContext::from_state(&state));
    for chunk in plan.actions.chunks(4) {
        sim.step(&mut run_state, &AgentActions::new(chunk.to_vec()));
    }
    let acquired = run_state.series_acquired_by_day.get(&0).cloned().unwrap_or_default();
    assert!(
        acquired.contains(&1) && acquired.contains(&2),
        "the claim policy must spread the patrols across both series, got {acquired:?}"
    );
}
