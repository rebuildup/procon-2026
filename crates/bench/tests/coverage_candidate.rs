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

/// `fuelaware` must reject unknown values loudly (Issue #348 review):
/// `treu` / `1` / `yes` used to silently build as plain coverage.
#[test]
fn fuelaware_rejects_unknown_modes_and_accepts_only_true_false() {
    let reg = CandidateRegistry::builtin();
    assert!(
        reg.build("coverage:budget=official:fuelaware=true").and_then(|c| c.build_solver()).is_ok()
    );
    assert!(
        reg.build("coverage:budget=official:fuelaware=false")
            .and_then(|c| c.build_solver())
            .is_ok()
    );
    assert!(reg.build("coverage:budget=official").and_then(|c| c.build_solver()).is_ok());
    for bad in ["treu", "1", "yes", "on"] {
        let id = format!("coverage:budget=official:fuelaware={bad}");
        assert!(
            reg.build(&id).and_then(|c| c.build_solver()).is_err(),
            "fuelaware={bad:?} must be rejected, not silently default to false"
        );
    }
}

// ---------------------------------------------------------------------------
// Issue #364 reservation-semantics fixtures.
//
// The decide loop splits its target bookkeeping into three concepts:
//
// 1. **acquired** (`day_claimed` / the acquired half of `reserved`) — series
//    really picked today; persistent, never erased by a reservation release.
// 2. **reservations** (per-patrol `reservations` map) — in-flight movement
//    claims owned by the patrol that made them; released only by their owner
//    (on landing+acquire, or on re-target away such as a fuel-refuel
//    interruption).
// 3. **active_target** (per-patrol continuing goal) — a re-selecting patrol
//    is exempt from the *foreign-reservation* penalty for its own goal, so
//    it keeps flying instead of oscillating; the exemption dies when the
//    goal is acquired.
//
// Each fixture below states the pre-fix behaviour it pins.
// ---------------------------------------------------------------------------

/// Replay the first day of `candidate` on `setting` and return the plan plus
/// the final simulator state (all decided actions executed).
fn replay_first_day(
    setting: MatchSetting,
    candidate: &str,
) -> (engine::model::AgentActions, MatchState) {
    let state = MatchState::new(setting).unwrap();
    let solver = CandidateRegistry::builtin().build(candidate).unwrap().build_solver().unwrap();
    let plan = solver.decide(&SolverContext::from_state(&state));
    let n = state.setting.agents.len();
    let mut run = state.clone();
    let sim = MatchSim::default();
    for chunk in plan.actions.chunks(n) {
        sim.step(&mut run, &AgentActions::new(chunk.to_vec()));
    }
    (plan, run)
}

fn axes_of(state: &MatchState) -> (u32, u32, u32) {
    let daily: u32 =
        state.series_acquired_by_day.values().map(|s| u32::try_from(s.len()).unwrap()).sum();
    let balls: u32 = state.agents.iter().map(engine::Agent::total_inventory).sum();
    (u32::try_from(state.series_taken.len()).unwrap(), daily, balls)
}

/// Plain board with spots at `(col, row)` -> `(series, stock)`.
fn spot_board(size: u8, spots: &[(i32, i32, u32, u32)]) -> Vec<Cell> {
    (0..i32::from(size))
        .flat_map(|row| {
            (0..i32::from(size)).map(move |col| {
                let pos = HexCoord::new(col, row);
                let spot = spots.iter().find(|(c, r, _, _)| *c == col && *r == row).map(
                    |(_, _, series, stock)| Spot {
                        series: *series,
                        pos,
                        max_stock: *stock,
                        current_stock: *stock,
                    },
                );
                Cell { coord: pos, terrain: Terrain::Plain, spot }
            })
        })
        .collect()
}

/// Fixture (a) — multi-patrol same-series/spot conflict (Issue #364).
///
/// Two patrols start side by side; series 1 has two spots and series 2 one.
/// The first patrol claims the nearest series-1 spot; the **reservation**
/// must make the second patrol take a different series instead of
/// double-booking the same spot, and the **acquired** claim must stop the
/// first patrol from re-visiting series 1's second spot.
///
/// Semantics verification (the score-equivalent pre-fix variants are caught
/// by the fixtures below): on every base the outcome must be the
/// conflict-free split — every spot picked at most once, both series
/// acquired, and no patrol starved by double-booking. The convergence
/// variants (target oscillation, zombie claims, erased acquired claims)
/// surface as wasted travel or lost series and are pinned by the
/// fuel-return, post-acquire and oscillation fixtures.
#[test]
fn reservation_split_prevents_same_spot_double_booking() {
    let setting = MatchSetting {
        match_id: MatchId::new("coverage-364-conflict").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(
            MapSize::new(16, 16).unwrap(),
            spot_board(16, &[(4, 0, 1, 1), (12, 0, 1, 1), (4, 6, 2, 1)]),
        )
        .unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 100),
            Agent::patrol_with_fuel(1, HexCoord::new(1, 0), 100),
            Agent::supply(2, HexCoord::new(0, 1)),
        ],
        day_steps: 48,
        num_days: 1,
        day_steps_by_day: vec![48],
    };
    let (_, run) = replay_first_day(setting, "coverage:budget=official");

    // No double-booking: every spot is picked at most once.
    for picks in run.spot_picks_today.values() {
        assert!(*picks <= 1, "a spot was double-booked: {picks} picks");
    }
    // The reservations split the patrols across BOTH series.
    assert_eq!(axes_of(&run), (2, 2, 3), "both series acquired, three balls total");
    // And no patrol ends the day empty-handed (the split fed both).
    for agent in &run.agents {
        if agent.kind == engine::model::AgentKind::Patrol {
            assert!(
                engine::Agent::total_inventory(agent) >= 1,
                "patrol {} starved by double-booking",
                agent.id
            );
        }
    }
}

/// Fixture (b) — fuel-refuel target interruption (Issue #364).
///
/// Patrol 0 (low fuel, `fuelaware=true`) flies toward the near series-1 spot
/// but is interrupted by the fuel-return override (`fuel < 30`) and detours
/// to its paired Supply. Its **own reservation must be released** at the
/// interruption: a day-global set would keep the zombie claim alive, locking
/// patrol 1 — the patrol that can actually reach the spot in time — out of
/// series 1 for the whole day, so only the slow refuelling patrol could ever
/// pick it. The abandoned flight itself must NOT mark the spot acquired, and
/// after refuelling patrol 0 resumes coverage toward its active target.
///
/// Pre-fix behaviour pinned: the reservation stayed in the day-global
/// `reserved` set forever, so patrol 1 never contested series 1 and patrol 0
/// (back late from the refuel detour) picked it itself.
///
/// The fixture isolates the reservation mechanics, not V/D optimization —
/// whether patrol 1 still reaches its abandoned series-2 spot after the
/// re-target is a score question covered by the 701 gate.
#[test]
fn fuel_return_releases_own_reservation_for_other_patrols() {
    let mut patrol0 = Agent::patrol_with_fuel(0, HexCoord::new(0, 1), 80);
    patrol0.fuel = 34; // drops below the fuel-return threshold five moves in
    let setting = MatchSetting {
        match_id: MatchId::new("coverage-364-fuel-return").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(
            MapSize::new(24, 24).unwrap(),
            spot_board(24, &[(22, 0, 1, 1), (23, 23, 2, 1)]),
        )
        .unwrap(),
        agents: vec![
            patrol0,
            Agent::patrol_with_fuel(1, HexCoord::new(20, 1), 200),
            Agent::supply(2, HexCoord::new(0, 3)),
        ],
        day_steps: 80,
        num_days: 1,
        day_steps_by_day: vec![80],
    };
    let (plan, run) = replay_first_day(setting, "coverage:budget=official:fuelaware=true");
    println!(
        "DBG p0inv={:?} p1inv={:?} p0pos={:?} p1pos={:?}",
        run.agents.iter().find(|a| a.id == 0).unwrap().inventory,
        run.agents.iter().find(|a| a.id == 1).unwrap().inventory,
        run.agents.iter().find(|a| a.id == 0).unwrap().pos,
        run.agents.iter().find(|a| a.id == 1).unwrap().pos
    );
    let p0_actions: Vec<String> =
        plan.actions.iter().step_by(3).map(|a| format!("{:?}", a.kind)).collect();
    println!("DBG p0 plan: {}", p0_actions.join(","));

    let daily = run.series_acquired_by_day.get(&0).cloned().unwrap_or_default();
    assert!(
        daily.contains(&1),
        "series 1 must be acquired after the interrupted patrol released it, got {daily:?}"
    );
    let p0 = run.agents.iter().find(|a| a.id == 0).unwrap();
    let p1 = run.agents.iter().find(|a| a.id == 1).unwrap();
    assert!(
        !p0.inventory.contains_key(&1),
        "the refuel-interrupted patrol must not be the one that acquired series 1 (patrol 0 picked {:?})",
        p0.inventory
    );
    assert!(
        p1.inventory.contains_key(&1),
        "the released reservation must let patrol 1 claim series 1 (patrol 1 picked {:?})",
        p1.inventory
    );
    // Patrol 0 resumed coverage after the refuel: it kept flying to its
    // active target (the journey interrupted at the detour), instead of
    // idling at the refuel point. Whether it then reaches a stocked spot
    // depends on patrol 1's re-target race, so the resumption is pinned by
    // travelled distance, not by a pick.
    let p0_moves =
        plan.actions.iter().step_by(3).filter(|a| matches!(a.kind, ActionKind::Move(_))).count();
    assert!(
        p0_moves >= 15,
        "patrol 0 must resume its journey after the refuel instead of idling ({p0_moves} moves)"
    );
}

/// Fixture (c) — post-acquire reservation release (Issue #364 P1-B).
///
/// A patrol acquires series 1 at the near spot; series 1 still has a second
/// spot closer than the unclaimed series 2. The acquired claim must persist:
/// the patrol's own-target exemption dies at the pick, so the remaining spot
/// of the acquired series loses to the unclaimed series.
///
/// Pre-fix behaviour pinned: the patrol's `self_targeted` exemption survived
/// the pick (or the acquired claim was erased from `reserved` outright), so
/// the patrol re-visited series 1's second spot BEFORE the unclaimed series
/// and the day ended with series 2 never reached.
#[test]
fn acquired_claim_persists_no_revisit_before_unclaimed_series() {
    let setting = MatchSetting {
        match_id: MatchId::new("coverage-364-acquired").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(
            MapSize::new(16, 16).unwrap(),
            spot_board(16, &[(3, 0, 1, 1), (3, 2, 1, 1), (15, 0, 2, 1)]),
        )
        .unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 100),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(0, 2)),
        ],
        day_steps: 32,
        num_days: 1,
        day_steps_by_day: vec![32],
    };
    let (_, run) = replay_first_day(setting, "coverage:budget=official");

    assert_eq!(
        axes_of(&run),
        (2, 2, 2),
        "the patrol must take the unclaimed series instead of re-visiting the acquired one"
    );
    // Series 1's second spot was never visited: its stock is untouched.
    let untouched = run
        .setting
        .map
        .get(HexCoord::new(3, 2))
        .and_then(|c| c.spot.as_ref())
        .map(|s| s.current_stock)
        .unwrap();
    assert_eq!(untouched, 1, "the acquired series' other spot must not be re-visited");
}

/// Fixture (d) — oscillation: a multi-step journey must not switch targets
/// mid-flight (Issue #364 P1-A).
///
/// The patrol's nearest target (series 2, off the direct path) takes several
/// single-cell moves to reach. Every landing tick re-decides; the patrol's
/// OWN active target must not be deprioritized as a foreign reservation, or
/// the patrol abandons it on the first re-evaluation, detours to the farther
/// series-1 spot, and can no longer afford the trip back (round trip).
///
/// Pre-fix behaviour pinned: with the target left inside the day-global
/// `reserved` set (no self-exemption), the patrol switched targets at the
/// first landing tick and the day ended with only one series acquired.
#[test]
fn patrol_keeps_flying_to_its_active_target_without_oscillation() {
    let setting = MatchSetting {
        match_id: MatchId::new("coverage-364-oscillation").unwrap(),
        team: PlayerId::new("player-a").unwrap(),
        map: MapGrid::new(
            MapSize::new(16, 16).unwrap(),
            spot_board(16, &[(14, 0, 1, 1), (2, 4, 2, 1)]),
        )
        .unwrap(),
        agents: vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 100),
            Agent::supply(1, HexCoord::new(0, 1)),
            Agent::supply(2, HexCoord::new(0, 2)),
        ],
        day_steps: 36,
        num_days: 1,
        day_steps_by_day: vec![36],
    };
    let (_, run) = replay_first_day(setting, "coverage:budget=official");

    assert_eq!(
        axes_of(&run),
        (2, 2, 2),
        "the patrol must keep flying to its active target (no mid-flight switch, no round trip)"
    );
}
