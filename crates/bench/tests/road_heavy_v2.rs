//! Regression tests for the rebuilt road corpus case (Issue #187).
//!
//! `road-heavy-v2` exists so road-aware / traffic-aware candidates can be
//! measured on a topology where road routing actually matters. These tests
//! pin the three properties the fixture is supposed to guarantee, against
//! the *real* cost semantics — no hand-written numbers:
//!
//! 1. **Threshold → state transition.** The fixture's own
//!    `busyThreshold=1` / `jammedThreshold=2` (read through
//!    `CaseSpec::sim_config`, not re-typed) flip a corridor road cell
//!    `Smooth → Busy` at the day-0 boundary and `Busy → Jammed` at the
//!    day-1 boundary of the real 3-day match, driven through
//!    `engine::MatchSim`. A single-day fixture can never show this, because
//!    `classify_traffic` combines today's and yesterday's counts.
//! 2. **Road-aware vs road-agnostic difference.** On this topology the
//!    west-east road corridor beats the plain row-3 detour in move-steps
//!    while the road is `Smooth` and loses once it is `Busy`/`Jammed`
//!    (`domain::cell` step costs), while always costing more fuel
//!    (`engine::rules::fuel_cost`). Both routes are validated for adjacency
//!    and terrain against the actual fixture map.
//! 3. **Identity separation.** The legacy `fixture:road-heavy` case keeps
//!    its pre-#187 terrain (Road 4 / Pond 54) and a truthful `medium`
//!    category; the new case carries a distinct id and the `road_heavy`
//!    category. Neither can be confused with the other.

use bench::MatchFixture;
use bench::case::CaseSpec;
use bench::fixture::{FixtureCategory, MatchFixtureLoader};
use domain::{Cell as DomainCell, PlayerId, TrafficState as DomainTraffic};
use engine::model::{
    Action, AgentActions, Direction, HexCoord, MapGrid, MatchState, Terrain,
    TrafficState as EngineTraffic,
};
use engine::rules::{TrafficThresholds, fuel_cost};
use engine::transitions::classify_traffic;
use engine::{MatchSim, SimConfig};

const CASE: &str = "road-heavy-v2";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load(name: &str) -> MatchFixture {
    MatchFixtureLoader::new().load_bundled(name).expect("bundled fixture loads")
}

fn case(name: &str) -> CaseSpec {
    CaseSpec::from_bundled_fixture(name).expect("fixture becomes a case")
}

fn team() -> PlayerId {
    PlayerId::new("player-a").expect("valid team id")
}

fn terrain_at(map: &MapGrid, col: i32, row: i32) -> Terrain {
    map.get(HexCoord::new(col, row)).expect("route cell in bounds").terrain
}

/// Map an engine [`Terrain`] onto the domain cost table's [`DomainCell`].
fn domain_cell(terrain: Terrain) -> DomainCell {
    match terrain {
        Terrain::Plain => DomainCell::Plain,
        Terrain::Mountain => DomainCell::Mountain,
        Terrain::Pond => DomainCell::Pond,
        Terrain::Road(state) => DomainCell::Road(match state {
            EngineTraffic::Smooth => DomainTraffic::Smooth,
            EngineTraffic::Busy => DomainTraffic::Busy,
            EngineTraffic::Jammed => DomainTraffic::Jammed,
        }),
    }
}

/// Sum of `domain::cell` move-steps for entering each cell of `route`
/// (the starting cell is not entered, so it is not counted). Road cells are
/// evaluated in the given [`DomainTraffic`] state — that is exactly what
/// "the road became Busy" means for a planner reading the map on day 1.
fn route_move_steps(map: &MapGrid, route: &[(i32, i32)], road_traffic: DomainTraffic) -> u32 {
    route
        .iter()
        .map(|&(col, row)| {
            let terrain = terrain_at(map, col, row);
            let cell = match terrain {
                Terrain::Road(_) => DomainCell::Road(road_traffic),
                other => domain_cell(other),
            };
            u32::from(cell.move_steps())
        })
        .sum()
}

/// Sum of `engine::rules::fuel_cost` for entering each cell of `route`.
fn route_fuel(map: &MapGrid, route: &[(i32, i32)]) -> u32 {
    route.iter().map(|&(col, row)| fuel_cost(terrain_at(map, col, row))).sum()
}

/// Assert `route` is walkable step-by-step under the engine's hex geometry
/// (each consecutive pair differs by exactly one `Direction`), starting at
/// `start`.
fn assert_adjacent(start: (i32, i32), route: &[(i32, i32)], label: &str) {
    let mut cur = start;
    for &next in route {
        assert!(
            Direction::ALL
                .iter()
                .any(|d| d.step(HexCoord::new(cur.0, cur.1)) == HexCoord::new(next.0, next.1)),
            "{label}: {cur:?} -> {next:?} is not one hex step"
        );
        assert!(
            next.0 >= 0 && next.1 >= 0 && next.0 < 16 && next.1 < 16,
            "{label}: {next:?} out of bounds"
        );
        cur = next;
    }
}

/// Drive one full day of the match with agent 0 following `moves` (one entry
/// per step; `None` = wait) and every other agent waiting. Asserts the day
/// boundary fired exactly when the fixture's `daySteps` ran out — that is
/// when `phase_traffic_update` reclassifies the roads.
fn drive_day(state: &mut MatchState, sim: MatchSim, moves: &[Option<Direction>]) {
    let agent_ids = state.setting.agent_ids();
    let day = state.day;
    let day_steps = state.day_steps();
    assert!(
        u32::try_from(moves.len()).expect("move count fits u32") <= day_steps,
        "plan must fit the day"
    );
    for step in 0..day_steps {
        let dir = moves.get(usize::try_from(step).expect("step fits usize")).copied().flatten();
        let mut actions = Vec::with_capacity(agent_ids.len());
        for id in &agent_ids {
            actions.push(match (*id, dir) {
                (0, Some(d)) => Action::step(0, d),
                (0, None) => Action::wait(0),
                (other, _) => Action::wait(other),
            });
        }
        sim.step(state, &AgentActions::new(actions));
    }
    assert_eq!(
        state.day,
        day + 1,
        "day {day} must end after exactly {day_steps} steps (traffic update runs here)"
    );
}

// ---------------------------------------------------------------------------
// 1. The fixture's own thresholds flip a corridor cell across day boundaries
// ---------------------------------------------------------------------------

#[test]
fn fixture_thresholds_flip_corridor_state_across_days() {
    let c = case(CASE);
    let sim_config: SimConfig = c.sim_config().expect("fixture thresholds are Q13-legal");
    let thresholds = sim_config.traffic_thresholds;

    // The real fixture values, read from the case — not re-typed here.
    assert_eq!(c.setting.busy_threshold, 1, "fixture busyThreshold");
    assert_eq!(c.setting.jammed_threshold, 2, "fixture jammedThreshold");
    assert_eq!((thresholds.busy_threshold, thresholds.jammed_threshold), (1, 2));
    assert_ne!(
        thresholds,
        TrafficThresholds::default(),
        "the fixture must declare its own thresholds, or this test proves nothing"
    );

    // Same counts, different thresholds: the fixture's thresholds are what
    // promote the cell — the simulator default (2/5) would leave it Smooth.
    assert_eq!(classify_traffic(thresholds, 1, 0), EngineTraffic::Busy);
    assert_eq!(classify_traffic(TrafficThresholds::default(), 1, 0), EngineTraffic::Smooth);
    assert_eq!(classify_traffic(thresholds, 2, 1), EngineTraffic::Jammed);
    assert_eq!(
        classify_traffic(TrafficThresholds::default(), 2, 1),
        EngineTraffic::Busy,
        "the simulator default (2/5) would never jam this cell on these counts"
    );

    // Multi-day case: a single-day fixture can never observe a transition,
    // because `phase_traffic_update` combines today + yesterday (Q&A Q13).
    assert!(c.setting.day_steps.len() >= 2, "the case must run at least 2 days");
    assert_eq!(c.setting.day_seconds.len(), c.setting.day_steps.len(), "day budgets align");

    // Wire the case through the same conversion the evaluator uses.
    let setting = c.to_engine_setting(&team()).expect("engine::from_wire accepts the case");
    let mut state = MatchState::new(setting).expect("validated setting");
    let sim = MatchSim::new(sim_config);

    // Wire cell code 3 always enters as Smooth.
    assert_eq!(terrain_at(&state.setting.map, 1, 4), Terrain::Road(EngineTraffic::Smooth));

    // Day 0: agent 0 (start (1,3)) steps down onto the corridor and walks
    // east to (13,4) — crossing corridor cell (1,4) exactly once.
    let day0: Vec<Option<Direction>> = std::iter::once(Some(Direction::LowerRight))
        .chain((0..12).map(|_| Some(Direction::Right)))
        .collect();
    drive_day(&mut state, sim, &day0);
    assert_eq!(state.agents[0].pos, HexCoord::new(13, 4), "agent 0 walked the corridor");

    // End of day 0: 1 traversal >= busyThreshold 1 -> Busy on day 1.
    assert_eq!(
        terrain_at(&state.setting.map, 1, 4),
        Terrain::Road(EngineTraffic::Busy),
        "day-0 traffic must promote the corridor cell for day 1"
    );
    assert_eq!(
        terrain_at(&state.setting.map, 5, 4),
        Terrain::Road(EngineTraffic::Busy),
        "every corridor cell used once that day is Busy"
    );

    // Day 1: agent 0 walks west back to (1,4), steps off, and steps back on
    // — two traversals of (1,4) today against yesterday's one.
    let day1: Vec<Option<Direction>> = (0..12)
        .map(|_| Some(Direction::Left))
        .chain([Some(Direction::UpperLeft), Some(Direction::LowerRight)])
        .collect();
    drive_day(&mut state, sim, &day1);

    // max(today=2, yesterday=1) >= jammedThreshold 2 -> Jammed on day 2.
    assert_eq!(
        terrain_at(&state.setting.map, 1, 4),
        Terrain::Road(EngineTraffic::Jammed),
        "day-1 traffic must jam the corridor cell for day 2"
    );
    // Cells used once per day stay at the busy level — the jam is specific
    // to the over-used cell, which is what makes rerouting a real choice.
    assert_eq!(terrain_at(&state.setting.map, 5, 4), Terrain::Road(EngineTraffic::Busy));
}

// ---------------------------------------------------------------------------
// 2. Road-aware vs road-agnostic is observable on this topology
// ---------------------------------------------------------------------------

#[test]
fn corridor_beats_plain_detour_under_smooth_and_inverts_when_busy() {
    let c = case(CASE);
    let setting = c.to_engine_setting(&team()).expect("engine::from_wire accepts the case");

    // Pin the endpoints to the fixture itself: agent 0 starts at (1,3) and
    // the showcase spot sits at (13,3). If either moves, the route constants
    // below must be redesigned — that is the point of asserting them here.
    let width = i32::from(setting.map.size.width);
    let agent0 = c.setting.agents[0];
    assert_eq!((agent0 % width, agent0 / width), (1, 3), "agent 0 start moved");
    let showcase_pos = 3 * width + 13;
    assert!(
        c.setting.spots.iter().any(|s| s.pos == showcase_pos),
        "showcase spot at (13,3) missing"
    );

    // Road corridor: down onto row 4 at (1,4), east to (13,4), up to the
    // spot at (13,3) — 14 entered cells (13 road + 1 plain).
    let corridor: Vec<(i32, i32)> =
        (1..=13).map(|x| (x, 4)).chain(std::iter::once((13, 3))).collect();
    // Plain detour: straight east along row 3 — 12 entered cells, all plain.
    let detour: Vec<(i32, i32)> = (2..=13).map(|x| (x, 3)).collect();

    // Both routes are real walks on this map, starting from agent 0.
    assert_adjacent((1, 3), &corridor, "corridor");
    assert_adjacent((1, 3), &detour, "detour");
    assert!(
        corridor[..corridor.len() - 1]
            .iter()
            .all(|&(x, y)| matches!(terrain_at(&setting.map, x, y), Terrain::Road(_))),
        "corridor spine must be road"
    );
    assert!(
        detour.iter().all(|&(x, y)| terrain_at(&setting.map, x, y) == Terrain::Plain),
        "the row-3 alternative must be plain (no free road discounts)"
    );
    assert_eq!(terrain_at(&setting.map, 13, 3), Terrain::Plain, "spot cell is plain");

    // --- move-step cost (domain::cell table: Plain 2, Road 1/2/4) ---------
    let smooth = route_move_steps(&setting.map, &corridor, DomainTraffic::Smooth);
    let detour_steps = route_move_steps(&setting.map, &detour, DomainTraffic::Smooth);
    let busy = route_move_steps(&setting.map, &corridor, DomainTraffic::Busy);
    let jammed = route_move_steps(&setting.map, &corridor, DomainTraffic::Jammed);

    assert_eq!((smooth, detour_steps), (15, 24), "Smooth corridor vs plain detour");
    assert!(
        smooth < detour_steps,
        "road-aware routing must win on this corridor while the road is Smooth"
    );
    assert_eq!(busy, 28, "Busy corridor: 13 road cells at 2 + 1 plain at 2");
    assert!(busy > detour_steps, "the road advantage must invert once the corridor is Busy");
    assert!(jammed > busy, "Jammed (4/step) must be strictly worse than Busy (2/step)");
    // The plain detour does not care about traffic: road-blind planners that
    // only count cells behave the same on day 0 and day 2.
    assert_eq!(
        route_move_steps(&setting.map, &detour, DomainTraffic::Jammed),
        detour_steps,
        "detour has no road cells, so traffic cannot change its cost"
    );

    // --- fuel (engine rules: Plain 1, Road 2 in every traffic state) ------
    let corridor_fuel = route_fuel(&setting.map, &corridor);
    let detour_fuel = route_fuel(&setting.map, &detour);
    assert_eq!((corridor_fuel, detour_fuel), (27, 12));
    assert!(
        corridor_fuel > detour_fuel,
        "the corridor trades fuel for time — a fuel-blind 'road is always fast' \
         planner pays for it"
    );
    for state in [EngineTraffic::Smooth, EngineTraffic::Busy, EngineTraffic::Jammed] {
        assert_eq!(fuel_cost(Terrain::Road(state)), 2, "road fuel cost is traffic-independent");
    }

    // The inversion is driven by the fixture's own thresholds: one day of
    // corridor use is already enough to take the cell from Smooth (15-step
    // win) to Busy (28-step loss).
    let thresholds = c.sim_config().expect("thresholds").traffic_thresholds;
    assert_eq!(
        classify_traffic(thresholds, 1, 0),
        EngineTraffic::Busy,
        "fixture thresholds turn day-0 corridor use into the day-1 inversion"
    );
}

// ---------------------------------------------------------------------------
// 3. Identity: legacy case keeps its content, v2 is a distinct case
// ---------------------------------------------------------------------------

#[test]
fn legacy_and_v2_cases_keep_distinct_identity() {
    let legacy = case("road-heavy");
    let v2 = case(CASE);
    assert_eq!(legacy.case_id, "fixture:road-heavy");
    assert_eq!(v2.case_id, "fixture:road-heavy-v2");
    assert_ne!(legacy.case_id, v2.case_id, "old and new evidence must not share an id");

    let legacy_fx = load("road-heavy");
    let v2_fx = load(CASE);
    assert!(matches!(legacy_fx.category, FixtureCategory::Medium), "legacy relabeled honestly");
    assert!(matches!(v2_fx.category, FixtureCategory::RoadHeavy), "v2 owns the category");

    // The legacy body is untouched pre-#187 content: Road 4 / Pond 54 of 256.
    assert_eq!(legacy_fx.cell_type_count(protocol::match_config::CellType::TYPE_3), 4);
    assert_eq!(legacy_fx.cell_type_count(protocol::match_config::CellType::TYPE_2), 54);
    // ...while v2 is a genuine, validated road network (>= 15% floor).
    let roads = v2_fx.cell_type_count(protocol::match_config::CellType::TYPE_3);
    let total = u64::try_from(v2_fx.cell_count()).expect("cell count fits u64");
    assert!(
        u64::try_from(roads).expect("road count fits u64") * 100 >= total * 15,
        "v2 must clear the road_heavy density floor, got {roads}/{total}"
    );
}

// ---------------------------------------------------------------------------
// 4. No agent or spot sits on impassable terrain in the new case
// ---------------------------------------------------------------------------

#[test]
fn road_heavy_v2_positions_avoid_impassable_terrain() {
    let fx = load(CASE);
    let width = fx.width();
    let terrain = |pos: i32| {
        let row = usize::try_from(pos / width).expect("row is non-negative");
        let col = usize::try_from(pos % width).expect("col is non-negative");
        fx.setting.map.cells[row][col]
    };
    for (i, pos) in fx.setting.agents.iter().enumerate() {
        assert_ne!(
            terrain(*pos),
            protocol::match_config::CellType::TYPE_2,
            "agent #{i} starts on impassable Pond"
        );
    }
    for (i, spot) in fx.setting.spots.iter().enumerate() {
        assert_ne!(
            terrain(spot.pos),
            protocol::match_config::CellType::TYPE_2,
            "spot #{i} sits on impassable Pond"
        );
    }
    // Spots live on plain cells so road state can never shadow a pickup.
    for (i, spot) in fx.setting.spots.iter().enumerate() {
        assert_eq!(
            terrain(spot.pos),
            protocol::match_config::CellType::TYPE_0,
            "spot #{i} should sit on plain terrain"
        );
    }
}
