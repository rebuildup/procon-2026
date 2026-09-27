//! Official submission-boundary plan normalization (Issue #226).
//!
//! # The rule the official server enforces
//!
//! `POST /` is validated per agent against a **terrain-weighted** step
//! budget, not against the number of elements in the answer array:
//!
//! ```text
//! Σ(step cost of the cell each move *departs*) + Σ(wait runs) == daySteps[day]
//! ```
//!
//! The step-cost table (募集要項 表1, Q&A Q11 — the same values as
//! `domain::cell::MOVE_STEPS`) is `Plain 2`, `Mountain 3`,
//! `Road(Smooth) 1 / Road(Busy) 2 / Road(Jammed) 4`; `Pond` is impassable
//! and a move into it is illegal. Which cell the cost is charged for is
//! **the origin — the cell the agent stands on when it departs** — that is
//! what the localhost probes below pin down (an entered-cell reading would
//! predict the opposite verdict on the last three rows). A move is also
//! rejected when it leaves the board, enters impassable terrain, or a
//! patrol cannot pay its fuel (`engine::rules::fuel_cost`, Q&A Q11);
//! **one illegal action invalidates the whole day's plan**
//! (`revision = -1`).
//!
//! Probed against the official binary on localhost (Issue #226 evidence):
//! day 0 of `cfg/example.json` (`daySteps = 20`), one moving agent, the
//! others all-wait, kinds `[0, 1, 0]`:
//!
//! | plan (agent, start) | element sum | origin-weighted sum | entered-weighted sum | result |
//! |---|---:|---:|---:|---|
//! | agent0 `[2, -19]` (starts on 平地) | 20 | 21 | 21 | **rejected** |
//! | agent0 `[2, -18]` | 19 | 20 | 20 | **accepted** (`revision: 1`) |
//! | agent1 `[2, -19]` (平地 → 道路) | 20 | 21 | **20** | **rejected** |
//! | agent1 `[2, -18]` (平地 → 道路) | 19 | **20** | 19 | **accepted** (`revision: 1`) |
//! | agent2 `[2, -17]` (平地 → 山地) | 18 | **19** | 20 | **rejected** |
//! | all agents `[-20]` (wait control) | 20 | 20 | 20 | accepted |
//!
//! (Bold = the value the official server's verdict is consistent with.
//! Rule shape: exact equality — the 平地→山地 probe sums to 19 under the
//! origin table and is rejected, so the check is `==`, not `<=`.)
//!
//! # Why normalization lives here
//!
//! Candidates plan on the engine's own clock — one action per step, with
//! `day_steps` actions per agent — and `cli::action_adapter` validates that
//! element sum. That clock is *not* the official budget, so a
//! straight-through submission is rejected whenever any move costs more
//! than 1 step (every move that is not onto a smooth road). This module
//! replays a plan through the engine's own day-consumption rules
//! ([`crate::transitions::try_move`] for bounds / terrain / fuel and
//! [`terrain_step_cost`] for the step budget) and rewrites it into a plan
//! that is exactly `day_steps` weighted steps:
//!
//! 1. consume the agent's actions in order while they stay legal and fit
//!    the remaining budget (movement prefix preserved),
//! 2. stop at the first action that would overflow the budget or that the
//!    official server would reject (off-board, impassable, fuel-short),
//! 3. pad the shortfall with a single trailing wait-run (`-R`),
//!
//! so `Σ weighted == day_steps` **exactly**. Waits are state-neutral in
//! [`crate::MatchSim`], therefore — for plans that end in waits, which is
//! what the walk/loop candidates emit — the normalized plan simulates
//! identically to the original (property-tested below). Offline evaluation
//! and candidate output are untouched: this runs only at the wire
//! submission boundary (`cli::coordinator_loop`, the single choke point
//! that both `CandidateBridge` and `EngineLoopAdapter` decisions flow
//! through).

use protocol::match_config::ActionPlanBody;

use crate::model::{Action, Agent, Direction, MapGrid, Terrain};
use crate::transitions::{MoveOutcome, phase_fuel, try_move};

/// Terrain / traffic step cost charged when a move *departs* this cell
/// (the official budget bills the origin cell — see the module table).
///
/// `None` marks impassable terrain (`Pond`). Road costs follow the live
/// traffic state (`TrafficState::step_cost`: `1 / 2 / 4`).
///
/// Spec: 募集要項 表1 + Q&A Q11; cross-checked against
/// `domain::cell::MOVE_STEPS` by [`tests::step_cost_table_matches_the_domain_table`].
#[must_use]
pub const fn terrain_step_cost(terrain: Terrain) -> Option<u32> {
    match terrain {
        Terrain::Plain => Some(2),
        Terrain::Mountain => Some(3),
        Terrain::Pond => None,
        Terrain::Road(state) => Some(state.step_cost()),
    }
}

/// Engine direction for a wire code (`0..=5`, official order:
/// `0` 左上, `1` 右上, `2` 右, `3` 右下, `4` 左下, `5` 左).
fn direction_for_wire(code: i32) -> Option<Direction> {
    let idx = usize::try_from(code).ok()?;
    Direction::ALL.get(idx).copied()
}

/// Official validation of one agent's day plan: every action must be legal
/// (bounds / terrain / fuel — the same rules `MatchSim` applies) and the
/// terrain-weighted step sum must equal `day_steps` exactly.
///
/// Returns the validated weighted sum on success. Used by tests and by the
/// E2E mock so element-count-only plans cannot pass as conformant.
///
/// # Errors
/// Human-readable reason for the first violation (illegal wire code,
/// illegal move, budget overflow, or a sum that is not exactly
/// `day_steps`).
pub fn validate_agent_plan(
    agent: &Agent,
    map: &MapGrid,
    day_steps: u32,
    actions: &[i32],
) -> Result<u32, String> {
    // Shadow agent: fuel is consumed as moves replay, exactly like the
    // simulator's fuel phase (`phase_fuel` skips supply agents).
    let mut shadow = vec![agent.clone()];
    let mut moves = Vec::new();
    let mut events = Vec::new();
    let mut used: u32 = 0;

    for (i, &code) in actions.iter().enumerate() {
        if code <= -1 {
            let wait = code.unsigned_abs();
            used = used
                .checked_add(wait)
                .ok_or_else(|| format!("action {i}: wait sum overflows the budget"))?;
            continue;
        }
        let dir = direction_for_wire(code)
            .ok_or_else(|| format!("action {i}: {code} is not a legal wire action code"))?;
        let agent_id = shadow[0].id;
        // Budget: the **origin** cell's step cost (see the module table —
        // the plain→mountain probe `[2,-17]` is rejected although an
        // entered-based sum would be exactly 20, and the plain→road pair
        // `[2,-19]` / `[2,-18]` splits on the origin 平地 cost 2).
        let origin = map.get(shadow[0].pos).ok_or_else(|| format!("action {i}: off the board"))?;
        let step = terrain_step_cost(origin.terrain)
            .ok_or_else(|| format!("action {i}: standing on impassable terrain"))?;
        match try_move(&shadow[0], Action::step(agent_id, dir), map, &mut moves, &mut events) {
            MoveOutcome::Moved { to, cost } => {
                used = used
                    .checked_add(step)
                    .ok_or_else(|| format!("action {i}: step sum overflows"))?;
                if used > day_steps {
                    return Err(format!(
                        "action {i}: weighted step sum would reach {used}, \
                         over the {day_steps} budget"
                    ));
                }
                phase_fuel(&mut shadow, &[(agent_id, cost)], &mut events);
                shadow[0].pos = to; // `try_move` is pure; the sim's apply_moves does this
            }
            MoveOutcome::Wait { reason } => {
                return Err(format!("action {i}: official rules reject this move ({reason})"));
            }
        }
    }

    if used != day_steps {
        return Err(format!("weighted step sum {used}, expected exactly {day_steps}"));
    }
    Ok(used)
}

/// Normalize one agent's day plan to the official terrain-weighted budget.
///
/// Replays `actions` (wire codes) from `agent`'s live position, keeping the
/// longest legal prefix that fits the remaining budget, then pads the
/// shortfall with a single trailing wait-run. The result satisfies
/// [`validate_agent_plan`] by construction (see
/// [`tests::normalized_output_always_passes_official_validation`]).
///
/// Waits are truncated to the remaining budget rather than dropped: a wait
/// run that no longer fits contributes its fitting part and ends the day.
#[must_use]
pub fn normalize_agent_plan(
    agent: &Agent,
    map: &MapGrid,
    day_steps: u32,
    actions: &[i32],
) -> Vec<i32> {
    let mut out = Vec::with_capacity(actions.len());
    let mut shadow = vec![agent.clone()];
    let mut moves = Vec::new();
    let mut events = Vec::new();
    let mut used: u32 = 0;

    for &code in actions {
        if used >= day_steps {
            break;
        }
        if code <= -1 {
            let wait = code.unsigned_abs();
            let take = wait.min(day_steps - used);
            if take > 0 {
                let encoded = i32::try_from(take).unwrap_or(i32::MAX);
                out.push(-encoded);
                used += take;
            }
            continue;
        }
        let Some(dir) = direction_for_wire(code) else { break };
        let agent_id = shadow[0].id;
        // Budget: the **origin** cell's step cost (module table). Legality
        // (bounds / impassable destination / fuel) stays with `try_move`.
        let Some(origin) = map.get(shadow[0].pos) else { break };
        let Some(step) = terrain_step_cost(origin.terrain) else { break };
        if used.saturating_add(step) > day_steps {
            break; // budget: keep the prefix, pad below
        }
        let outcome =
            try_move(&shadow[0], Action::step(agent_id, dir), map, &mut moves, &mut events);
        let MoveOutcome::Moved { to, cost } = outcome else {
            break; // the official server would reject this move: keep the prefix
        };
        out.push(code);
        used += step;
        phase_fuel(&mut shadow, &[(agent_id, cost)], &mut events);
        shadow[0].pos = to; // `try_move` is pure; the sim's apply_moves does this
    }

    if used < day_steps {
        let pad = i32::try_from(day_steps - used).unwrap_or(i32::MAX);
        out.push(-pad);
    }
    out
}

/// Normalize an entire `POST /` body for the day.
///
/// `my_agent_ids` must be in submission order (the order of `body`'s inner
/// lists). `map` and `agents` must carry the *live* view — positions, fuel
/// and road traffic overlaid from the current `GET /` payload — which is
/// what makes day ≥ 1 replays start where the agents actually stand, with
/// the fuel they actually carry and the road costs they are actually
/// charged.
///
/// # Errors
/// When `body` has a different agent count than `my_agent_ids`, or an agent
/// id is missing from `agents`.
pub fn normalize_day_plan(
    map: &MapGrid,
    agents: &[Agent],
    my_agent_ids: &[u8],
    day_steps: u32,
    body: &ActionPlanBody,
) -> Result<ActionPlanBody, String> {
    if body.len() != my_agent_ids.len() {
        return Err(format!(
            "answer body has {} agent lists, expected {}",
            body.len(),
            my_agent_ids.len()
        ));
    }
    let mut out = ActionPlanBody::with_capacity(my_agent_ids.len());
    for (idx, id) in my_agent_ids.iter().enumerate() {
        let agent = agents
            .iter()
            .find(|a| a.id == *id)
            .ok_or_else(|| format!("agent {id} missing from the live match state"))?;
        out.push(normalize_agent_plan(agent, map, day_steps, &body[idx]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cell, HexCoord, MapGrid, MapSize, MatchSetting, MatchState, TrafficState};
    use crate::solver::TeamId;
    use crate::{MatchSim, SimConfig, rules::fuel_cost};

    const FLAT: [&str; 8] = ["bbbbbbbb"; 8];

    /// Build a map from an ASCII sketch: `b` 平地, `m` 山地, `p` 池,
    /// `r` 道路(順調), `B` 道路(混雑).
    fn grid(sketch: &[&str]) -> MapGrid {
        let height = u8::try_from(sketch.len()).expect("small test map");
        let width = u8::try_from(sketch[0].len()).expect("small test map");
        let mut cells = Vec::new();
        for (row, line) in sketch.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                let terrain = match ch {
                    'b' => Terrain::Plain,
                    'm' => Terrain::Mountain,
                    'p' => Terrain::Pond,
                    'r' => Terrain::Road(TrafficState::Smooth),
                    'B' => Terrain::Road(TrafficState::Busy),
                    other => panic!("unknown terrain char {other}"),
                };
                cells.push(Cell {
                    coord: HexCoord::new(
                        i32::try_from(col).expect("col fits"),
                        i32::try_from(row).expect("row fits"),
                    ),
                    terrain,
                    spot: None,
                });
            }
        }
        MapGrid::new(MapSize::new(width, height).expect("8..=32"), cells).expect("grid builds")
    }

    /// 8x8 flat map whose cell at `(1, 0)` is `marker`.
    fn map_with(marker: char) -> MapGrid {
        let mut row = String::from("b");
        row.push(marker);
        row.push_str("bbbbbb");
        let sketch = [row.as_str(); 8];
        grid(&sketch)
    }

    fn patrol_at(col: i32, row: i32, fuel: u32) -> Agent {
        Agent::patrol_with_fuel(0, HexCoord::new(col, row), fuel)
    }

    /// Weighted sum of a wire plan, replayed from `start` (test-side
    /// reference implementation of the official rule).
    fn weighted_sum(map: &MapGrid, start: HexCoord, actions: &[i32]) -> u32 {
        let mut pos = start;
        let mut used = 0;
        for &code in actions {
            if code <= -1 {
                used += code.unsigned_abs();
                continue;
            }
            let dir = direction_for_wire(code).expect("legal code");
            // Origin cell is billed (official rule, module table).
            let cell = map.get(pos).expect("in bounds");
            used += terrain_step_cost(cell.terrain).expect("passable");
            pos = dir.step(pos);
        }
        used
    }

    /// AC (a): every shape of plan normalizes to *exactly* the weighted
    /// budget, keeping the movement prefix.
    #[test]
    fn normalization_hits_the_weighted_budget_exactly() {
        // (cell at (1,0), day, input, expected output)
        let cases: &[(char, u32, &[i32], &[i32])] = &[
            // all-wait control: unchanged.
            ('b', 20, &[-20], &[-20]),
            // plain overshoot: 4 plain moves = 8 > 6 -> keep 3 moves (6).
            ('b', 6, &[2, 2, 2, 2, -2], &[2, 2, 2]),
            // partial wait: move (2) + 4 of the 9 waits = 6.
            ('b', 6, &[2, -9], &[2, -4]),
            // wait first, then a move that fills the day exactly.
            ('b', 6, &[-4, 2, 2, 2, 2], &[-4, 2]),
            // mountain leg: the second move departs 山地 (3) -> 2 + 3 + 1 = 6.
            ('m', 6, &[2, 2, -3], &[2, 2, -1]),
            // road leg: the second move departs smooth 道路 (1) -> 2 + 1 + 3 = 6.
            ('r', 6, &[2, 2, -5], &[2, 2, -3]),
            // the second move departs a busy road (2, the traffic overlay
            // changes the budget) -> 2 + 2 + 2 = 6.
            ('B', 6, &[2, 2, -3], &[2, 2, -2]),
            // move into a pond is dropped (the official server would reject
            // the whole plan) and the day is padded with waits.
            ('p', 6, &[2, 5, -4], &[-6]),
            // off-board move (left from (0,0)) is dropped the same way.
            ('b', 6, &[5, 2, -4], &[-6]),
        ];

        for &(marker, day_steps, input, want) in cases {
            let map = if marker == 'b' { grid(&FLAT) } else { map_with(marker) };
            let agent = patrol_at(0, 0, 1_000);
            let got = normalize_agent_plan(&agent, &map, day_steps, input);
            assert_eq!(got.as_slice(), want, "marker {marker:?}, input {input:?}, day {day_steps}");
            assert_eq!(
                weighted_sum(&map, agent.pos, &got),
                day_steps,
                "weighted sum must equal day_steps for {got:?}"
            );
        }
    }

    /// Normalization is idempotent: normalizing a normalized plan is a
    /// no-op (it already satisfies the official budget).
    #[test]
    fn normalization_is_idempotent() {
        let map = grid(&FLAT);
        let agent = patrol_at(0, 0, 1_000);
        let once = normalize_agent_plan(&agent, &map, 6, &[2, 2, 2, 2, -2]);
        let twice = normalize_agent_plan(&agent, &map, 6, &once);
        assert_eq!(once, twice);
    }

    /// Whatever comes out of the normalizer passes the official validation
    /// (weighted sum exact, every move legal).
    #[test]
    fn normalized_output_always_passes_official_validation() {
        let cases: &[(char, u32, &[i32])] = &[
            ('b', 6, &[2, 2, 2, 2, -2]),
            ('b', 6, &[2, -9]),
            ('m', 6, &[2, 2, -3]),
            ('r', 6, &[2, 2, -5]),
            ('p', 6, &[2, 5, -4]),
            ('b', 6, &[-20]),
        ];
        for &(marker, day_steps, input) in cases {
            let map = if marker == 'b' { grid(&FLAT) } else { map_with(marker) };
            let agent = patrol_at(0, 0, 1_000);
            let normalized = normalize_agent_plan(&agent, &map, day_steps, input);
            validate_agent_plan(&agent, &map, day_steps, &normalized)
                .unwrap_or_else(|e| panic!("normalized {input:?} -> {normalized:?}: {e}"));
        }
    }

    /// Fuel is part of the official legality rules: a patrol that cannot pay
    /// for the next move has that move (and everything after it) dropped,
    /// rather than submitted and rejected wholesale.
    #[test]
    fn fuel_short_moves_are_dropped_not_submitted() {
        let map = map_with('r');
        // Entering the road costs 2 fuel; the patrol carries only 1.
        let broke = patrol_at(0, 0, 1);
        let normalized = normalize_agent_plan(&broke, &map, 6, &[2, 2, 2, -3]);
        assert_eq!(normalized.as_slice(), &[-6], "the unfuelable move must not be submitted");
        validate_agent_plan(&broke, &map, 6, &normalized).expect("padded plan validates");

        // With a full tank the road move is affordable: 1 (road) + 5 waits.
        let funded = patrol_at(0, 0, 20);
        // Budget now bills the origin 平地 (2) + 4 waits = 6.
        let normalized = normalize_agent_plan(&funded, &map, 6, &[2, -5]);
        assert_eq!(normalized.as_slice(), &[2, -4]);
        validate_agent_plan(&funded, &map, 6, &normalized).expect("road plan validates");
    }

    /// AC (b): for plans that end in waits — what the walk/loop candidates
    /// emit — truncate + pad is state-neutral in `MatchSim`, so the
    /// normalized plan simulates *identically* to the original: same final
    /// positions, same fuel, same step count.
    #[test]
    fn normalized_plan_simulates_identically_when_the_tail_is_waits() {
        let map = grid(&FLAT);
        let setting = setting_with(map, 6);
        let sim = MatchSim::new(SimConfig { traffic_thresholds: Default::default() });

        // Original candidate-shaped plan: 3 plain moves + 3 waits for agent 0.
        // Element sum = 6 (the engine clock), weighted sum = 3*2 + 3 = 9
        // (what the server checks) -> the normalizer trims the surplus waits.
        let original = [2_i32, 2, 2, -3];
        let normalized = normalize_agent_plan(&setting.agents[0], &setting.map, 6, &original);
        assert_eq!(normalized.as_slice(), &[2, 2, 2], "surplus waits are trimmed");

        // Engine view: the element clock wants `day_steps` actions, so the
        // normalized plan is padded back out with waits — which is exactly
        // the "pad" half of truncate+pad.
        let mut engine_view = normalized.clone();
        while engine_view.len() < 6 {
            engine_view.push(-1);
        }

        let mut from_original = MatchState::new(setting.clone()).expect("state");
        run_day(&sim, &mut from_original, &original);

        let mut from_normalized = MatchState::new(setting).expect("state");
        run_day(&sim, &mut from_normalized, &engine_view);

        assert_eq!(
            from_normalized.agents, from_original.agents,
            "truncate+pad must not change the simulated day"
        );
        assert_eq!(from_normalized.day_step, from_original.day_step);
        assert_eq!(from_normalized.illegal_action_count, from_original.illegal_action_count);
    }

    /// The complement of the property above: when the budget cuts the plan
    /// *inside* its movement prefix, the wire plan legitimately does less
    /// than the element-clock offline simulation ran — the boundary follows
    /// the official budget, and the movement prefix is preserved exactly.
    #[test]
    fn movement_prefix_is_preserved_when_the_budget_cuts_mid_move() {
        let map = grid(&FLAT);
        let agent = patrol_at(0, 0, 1_000);
        let input = [2, 2, 2, 2, -2]; // 4 moves (8 weighted) into a 6-step day
        let normalized = normalize_agent_plan(&agent, &map, 6, &input);
        assert_eq!(normalized.as_slice(), &[2, 2, 2]);
        // The kept moves are a prefix of the input's moves...
        assert_eq!(normalized.as_slice(), &input[..normalized.len()]);
        // ...and they walk the same cells the original's prefix walked.
        let mut pos = agent.pos;
        for &code in &normalized {
            pos = direction_for_wire(code).expect("legal").step(pos);
        }
        assert_eq!(pos, HexCoord::new(3, 0));
    }

    /// `Direction::ALL` (and therefore the wire codes `0..=5`) is the
    /// official order: 左上, 右上, 右, 右下, 左下, 左.
    #[test]
    fn wire_direction_codes_follow_the_official_order() {
        assert_eq!(Direction::ALL[0], Direction::UpperLeft);
        assert_eq!(Direction::ALL[1], Direction::UpperRight);
        assert_eq!(Direction::ALL[2], Direction::Right);
        assert_eq!(Direction::ALL[3], Direction::LowerRight);
        assert_eq!(Direction::ALL[4], Direction::LowerLeft);
        assert_eq!(Direction::ALL[5], Direction::Left);
        assert!(direction_for_wire(6).is_none());
        assert!(direction_for_wire(-1).is_none());
    }

    /// Step-cost table cross-check against the domain SoT
    /// (`domain::cell::MOVE_STEPS`): the engine budget helper must never
    /// drift from the shared rules table.
    #[test]
    fn step_cost_table_matches_the_domain_table() {
        use domain::cell::{Cell as DCell, TrafficState as DTraffic};
        let cases = [
            (Terrain::Plain, DCell::Plain),
            (Terrain::Mountain, DCell::Mountain),
            (Terrain::Pond, DCell::Pond),
            (Terrain::Road(TrafficState::Smooth), DCell::Road(DTraffic::Smooth)),
            (Terrain::Road(TrafficState::Busy), DCell::Road(DTraffic::Busy)),
            (Terrain::Road(TrafficState::Jammed), DCell::Road(DTraffic::Jammed)),
        ];
        for (engine, domain) in cases {
            let want = domain.move_steps();
            match terrain_step_cost(engine) {
                Some(got) => assert_eq!(got, u32::from(want), "{engine:?} step cost drifted"),
                None => assert_eq!(want, u8::MAX, "{engine:?} must be the impassable one"),
            }
        }
        // Fuel table stays the shared one too (Q&A Q11).
        assert_eq!(fuel_cost(Terrain::Plain), 1);
        assert_eq!(fuel_cost(Terrain::Mountain), 2);
        assert_eq!(fuel_cost(Terrain::Road(TrafficState::Jammed)), 2);
    }

    /// The day-level entry point keeps submission order and budgets each
    /// agent independently (three agents, different plans, one day).
    #[test]
    fn day_plan_normalization_is_per_agent() {
        let map = grid(&FLAT);
        let setting = setting_with(map, 6);
        let state = MatchState::new(setting).expect("state");

        let body: ActionPlanBody = vec![vec![2, 2, 2, 2, -2], vec![-6], vec![2, 2, -3]];
        let normalized =
            normalize_day_plan(&state.setting.map, &state.agents, &[0, 1, 2], 6, &body)
                .expect("body normalizes");
        assert_eq!(normalized[0], vec![2, 2, 2]);
        assert_eq!(normalized[1], vec![-6]);
        assert_eq!(normalized[2], vec![2, 2, -2]);
        for (idx, id) in [0, 1, 2].into_iter().enumerate() {
            let agent = state.agents.iter().find(|a| a.id == id).expect("agent");
            validate_agent_plan(agent, &state.setting.map, 6, &normalized[idx])
                .unwrap_or_else(|e| panic!("agent {id}: {e}"));
        }

        let wrong: ActionPlanBody = vec![vec![-6]];
        assert!(
            normalize_day_plan(&state.setting.map, &state.agents, &[0, 1, 2], 6, &wrong).is_err()
        );
    }

    // ------------------------------------------------------------------
    // helpers
    // ------------------------------------------------------------------

    /// A valid 3-agent all-plain setting (the engine requires 3..=8 agents);
    /// agents stand on rows 0..=2 of column 0.
    fn setting_with(map: MapGrid, day_steps: u32) -> MatchSetting {
        MatchSetting {
            match_id: domain::MatchId::new("wire-plan-test").expect("match id"),
            team: TeamId::new("wire-plan-test").expect("team id"),
            map,
            agents: vec![
                Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 1_000),
                Agent::patrol_with_fuel(1, HexCoord::new(0, 1), 1_000),
                Agent::patrol_with_fuel(2, HexCoord::new(0, 2), 1_000),
            ],
            day_steps,
            num_days: 1,
            day_steps_by_day: vec![day_steps],
        }
    }

    /// Expand a wire plan into one action per engine step and drive a full
    /// day through `MatchSim` (element clock — the offline semantics).
    /// Agent 0 follows `plan`; the others wait (well-shaped decisions).
    fn run_day(sim: &MatchSim, state: &mut MatchState, plan: &[i32]) {
        let ids = state.setting.agent_ids();
        let day_steps = state.day_steps();
        let mut expanded = Vec::with_capacity(day_steps as usize);
        for &code in plan {
            if code <= -1 {
                for _ in 0..code.unsigned_abs() {
                    expanded.push(Action::wait(0));
                }
            } else {
                expanded.push(Action::step(0, direction_for_wire(code).expect("legal")));
            }
        }
        assert_eq!(
            expanded.len(),
            usize::try_from(day_steps).expect("fits"),
            "test plan must cover the engine day: {plan:?}"
        );
        for agent0_action in expanded {
            let actions = crate::model::AgentActions::new(
                ids.iter()
                    .enumerate()
                    .map(|(idx, id)| if idx == 0 { agent0_action } else { Action::wait(*id) })
                    .collect(),
            );
            let _ = sim.step(state, &actions);
        }
    }
}
