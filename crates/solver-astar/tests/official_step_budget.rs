//! The `daySteps` budget contract, pinned against the official units.
//!
//! [`SolveContext::max_steps`] mirrors the official `daySteps` budget, and the
//! official server enforces it in **terrain-weighted steps**, not in answer
//! elements:
//!
//! ```text
//! Σ(step cost of the cell each move *departs*) <= daySteps
//! ```
//!
//! The table is 募集要項 表1 / Q&A Q11 — canonical at
//! [`domain::cell::MOVE_STEPS`], restated in the engine at
//! `engine::wire_plan::terrain_step_cost`:
//!
//! | cell | official step cost |
//! |---|---|
//! | plain | 2 |
//! | mountain | 3 |
//! | road (smooth / busy / jammed) | 1 / 2 / 4 |
//! | pond | impassable |
//!
//! Two facts are easy to get wrong, and both are pinned here:
//!
//! 1. **the table** — plain is 2 and mountain is 3, not 1 and 2;
//! 2. **the charged cell** — a move is billed the cell it **departs**, so
//!    plain → mountain costs 2 and mountain → plain costs 3.
//!
//! # Why the oracle lives here and not in the crate
//!
//! Every expected value in this file is read from [`domain::cell::MOVE_STEPS`]
//! or written out from the problem statement — never from `BaseStepCost`.
//! Asserting the crate's table against the crate's own table would pass no
//! matter how wrong the table got, which is the failure this issue is about.
//! `domain` is a **dev**-dependency for exactly this reason: the library stays
//! self-contained, as `lib.rs` documents, while the tests still check it
//! against the one canonical table.

use domain::TrafficState as DomainTraffic;
use domain::cell::{Cell, MOVE_STEPS};
use solver_astar::{
    Action, AstarSolver, BaseStepCost, CellKind, Direction, HexCoord, HexMap, Path, SolveContext,
    Solver, SolverFailure,
};

// ---------------------------------------------------------------------------
// Official table, read from the canonical source
// ---------------------------------------------------------------------------

/// The official step cost of a cell, read from [`MOVE_STEPS`].
///
/// This is the oracle every assertion below is measured against. It is
/// deliberately *not* `BaseStepCost::for_cell`: the point of the file is to
/// compare the implementation against the spec, and a shared helper would
/// make that comparison vacuous.
fn official_step_cost(cell: CellKind) -> Option<u32> {
    let domain_cell = match cell {
        CellKind::Plain => Cell::Plain,
        CellKind::Mountain => Cell::Mountain,
        CellKind::Pond => Cell::Pond,
        CellKind::Road { traffic } => Cell::Road(match traffic {
            solver_astar::TrafficState::Smooth => DomainTraffic::Smooth,
            solver_astar::TrafficState::Busy => DomainTraffic::Busy,
            solver_astar::TrafficState::Jammed => DomainTraffic::Jammed,
        }),
    };
    MOVE_STEPS
        .iter()
        .find(|(c, _)| *c == domain_cell)
        .map(|&(_, steps)| steps)
        .filter(|&steps| steps != u8::MAX)
        .map(u32::from)
}

/// The official budget a path consumes: every move is billed the cell it
/// **departs**.
fn official_step_total(map: &HexMap, start: HexCoord, path: &Path) -> Option<u32> {
    let mut total = 0;
    let mut pos = start;
    for action in &path.steps {
        let origin = map.get(pos)?;
        total += official_step_cost(origin)?;
        if let Action::Move(dir) = action {
            pos = dir.step(pos);
        }
    }
    Some(total)
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Build a `width x height` map from a row-of-rows textual fixture.
///
/// Encoding: `.` plain, `M` mountain, `~` pond (impassable),
/// `s`/`b`/`j` smooth / busy / jammed road.
fn map_from_str(width: u32, height: u32, rows: &[&str]) -> HexMap {
    assert_eq!(u32::try_from(rows.len()), Ok(height), "fixture must declare {height} rows");
    let mut cells = Vec::with_capacity(width as usize * height as usize);
    for (row_idx, row) in rows.iter().enumerate() {
        assert_eq!(
            u32::try_from(row.chars().count()),
            Ok(width),
            "row {row_idx} must be exactly {width} chars wide"
        );
        for ch in row.chars() {
            cells.push(cell_from_char(ch));
        }
    }
    HexMap::new(width, height, cells).expect("fixture map must validate")
}

/// An all-plain `width x height` map.
fn plain_map(width: u32, height: u32) -> HexMap {
    let row = ".".repeat(usize::try_from(width).expect("width fits usize"));
    let rows = vec![row.as_str(); usize::try_from(height).expect("height fits usize")];
    map_from_str(width, height, &rows)
}

/// A one-row **lane**: `lane` is placed on row 0, every other cell is pond.
///
/// `HexMap` refuses any dimension outside `[8, 32]`, so a literal `4 x 1`
/// fixture cannot be built. Row 0 is walled in by ponds and the top edge,
/// which makes the lane exactly as one-dimensional as the `4 x 1` fixture was
/// meant to be: from any lane cell the only legal move is along the lane.
fn lane_map(lane: &str) -> HexMap {
    const W: u32 = 8;
    const H: u32 = 8;
    let lane_len = lane.chars().count();
    assert!(lane_len <= W as usize, "lane {lane:?} is wider than the {W}-cell board");
    let mut cells = vec![CellKind::Pond; (W * H) as usize];
    for (col, ch) in lane.chars().enumerate() {
        cells[col] = cell_from_char(ch);
    }
    HexMap::new(W, H, cells).expect("lane fixture map must validate")
}

/// Decode one fixture character. Shared by [`map_from_str`] and [`lane_map`].
fn cell_from_char(ch: char) -> CellKind {
    match ch {
        '.' => CellKind::Plain,
        'M' => CellKind::Mountain,
        '~' => CellKind::Pond,
        's' => CellKind::Road { traffic: solver_astar::TrafficState::Smooth },
        'b' => CellKind::Road { traffic: solver_astar::TrafficState::Busy },
        'j' => CellKind::Road { traffic: solver_astar::TrafficState::Jammed },
        other => panic!("unknown fixture char {other:?}"),
    }
}

/// Solve and return the path, asserting one was produced.
fn solve_path(map: &HexMap, start: HexCoord, goal: HexCoord, fuel: u32, max_steps: u32) -> Path {
    let ctx = SolveContext::new(map.clone(), start, goal, fuel, max_steps)
        .expect("start/goal must be on passable cells");
    AstarSolver::default().solve(&ctx).path.expect("solver must return a path")
}

/// Cube-coordinate hex distance, mirroring the solver's own heuristic.
fn hex_distance(a: HexCoord, b: HexCoord) -> u32 {
    let (ac, ar) = (a.col, a.row);
    let (bc, br) = (b.col, b.row);
    let ax = ac - ar.div_euclid(2);
    let ay = -ax - ar;
    let bx = bc - br.div_euclid(2);
    let by = -bx - br;
    let dx = ax - bx;
    let dy = ay - by;
    let dz = -dx - dy;
    (dx.unsigned_abs() + dy.unsigned_abs() + dz.unsigned_abs()) / 2
}

// ---------------------------------------------------------------------------
// 1. The declared table is the official one
// ---------------------------------------------------------------------------

/// `BaseStepCost` is a restatement of a published spec, so it is pinned
/// against that spec. A well-meaning edit that "optimises" a weight fails
/// here instead of silently redefining what `max_steps` means.
#[test]
fn declared_step_costs_match_the_official_table() {
    for cell in [
        CellKind::Plain,
        CellKind::Mountain,
        CellKind::Road { traffic: solver_astar::TrafficState::Smooth },
        CellKind::Road { traffic: solver_astar::TrafficState::Busy },
        CellKind::Road { traffic: solver_astar::TrafficState::Jammed },
    ] {
        assert_eq!(
            BaseStepCost::for_cell(cell).map(|c| c.0),
            official_step_cost(cell),
            "{cell:?} must cost what domain::cell::MOVE_STEPS says it costs"
        );
    }
    assert_eq!(
        BaseStepCost::for_cell(CellKind::Pond),
        None,
        "pond is impassable, not merely expensive"
    );
}

// ---------------------------------------------------------------------------
// 2. The budget is enforced, in official units
// ---------------------------------------------------------------------------

/// A path that overruns `max_steps` in official units must be **rejected**,
/// not merely de-ranked.
///
/// Four plain moves east is 4 moves, but the official budget charges 2 steps
/// for each, so it costs 8. With `max_steps = 6` no path is feasible and the
/// solver must refuse. A budget used only for tie-breaking would hand this
/// back, which is the failure mode this test exists to catch.
#[test]
fn a_path_over_the_official_budget_is_rejected() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(4, 4);

    let ctx = SolveContext::new(map.clone(), start, goal, 100, 6).unwrap();
    let result = AstarSolver::default().solve(&ctx);

    let Some(path) = result.path else {
        assert_eq!(result.failure, Some(SolverFailure::ExceedsStepBudget { min_cost: 8 }));
        return;
    };
    let cost = official_step_total(&map, start, &path).expect("path stays on the map");
    panic!("BUG: returned a {cost}-step path under a 6-step budget");
}

/// A path that overruns the budget by **exactly one step** is still rejected.
///
/// 4 plain moves cost exactly 8, so a 7-step budget is one short. This is the
/// off-by-one a `<` vs `<=` slip in the budget check would let through.
#[test]
fn a_path_one_step_over_the_official_budget_is_rejected() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(4, 4);

    let ctx = SolveContext::new(map.clone(), start, goal, 100, 7).unwrap();
    let result = AstarSolver::default().solve(&ctx);

    let Some(path) = result.path else {
        return;
    };
    let cost = official_step_total(&map, start, &path).expect("path stays on the map");
    panic!("BUG: returned a {cost}-step path under a 7-step budget (off-by-one)");
}

/// A path landing **exactly at** the budget is accepted.
///
/// The boundary is `<=`, not `<`. An 8-step path under an 8-step budget is
/// feasible and the official server accepts it.
#[test]
fn a_path_exactly_at_the_official_budget_is_accepted() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(4, 4);

    let path = solve_path(&map, start, goal, 100, 8);
    assert_eq!(
        official_step_total(&map, start, &path),
        Some(8),
        "4 plain moves, each departing a plain cell, cost exactly 8 official steps"
    );
}

// ---------------------------------------------------------------------------
// 3. Origin charging — a move is billed the cell it leaves
// ---------------------------------------------------------------------------

/// A move is billed the cell it **departs**, not the cell it enters.
///
/// Departing plain and arriving on a mountain costs 2, not 3. Under
/// destination billing this would be 3, and the reverse direction below
/// would cost the same — the rule would be untestable.
#[test]
fn a_move_is_billed_the_cell_it_departs() {
    let map = lane_map(".M..");

    let path = solve_path(&map, HexCoord::new(0, 0), HexCoord::new(1, 0), 100, 100);
    assert_eq!(path.steps, vec![Action::Move(Direction::East)]);
    assert_eq!(
        official_step_total(&map, HexCoord::new(0, 0), &path),
        Some(2),
        "departing plain costs 2 even when arriving on a mountain"
    );
}

/// The mirror: departing a mountain costs 3 even when the destination is plain.
///
/// Together with the test above this pins the rule from both sides — a
/// destination-billed implementation charges 2 here too, making the two
/// directions symmetric and the origin rule indistinguishable from it.
#[test]
fn departing_a_mountain_costs_three_even_onto_plain() {
    let map = lane_map(".M..");

    let path = solve_path(&map, HexCoord::new(1, 0), HexCoord::new(0, 0), 100, 100);
    assert_eq!(path.steps, vec![Action::Move(Direction::West)]);
    assert_eq!(
        official_step_total(&map, HexCoord::new(1, 0), &path),
        Some(3),
        "departing a mountain costs 3 even when arriving on plain"
    );
}

/// The **budget check** is origin-based, not just the path accounting.
///
/// Departing the mountain is 3 official steps, so a 2-step budget must refuse
/// the move even though the destination is a plain cell worth exactly 2.
#[test]
fn the_budget_check_bills_the_origin_cell() {
    let map = lane_map(".M..");

    let ctx = SolveContext::new(map, HexCoord::new(1, 0), HexCoord::new(0, 0), 100, 2).unwrap();
    let result = AstarSolver::default().solve(&ctx);

    let Some(path) = result.path else {
        return;
    };
    panic!(
        "BUG: a 3-step move (mountain origin) was accepted under a 2-step budget; \
         got {:?}",
        path.steps
    );
}

// ---------------------------------------------------------------------------
// 4. The flat / mountain distinction
// ---------------------------------------------------------------------------

/// A mountain crossing is charged 3 per step, plain 2 — same geometry,
/// different price, because the price follows the cell being left.
#[test]
fn mountain_costs_more_than_plain_for_the_same_move() {
    let plain_lane = lane_map("....");
    let mountain_lane = lane_map("M...");

    let start = HexCoord::new(0, 0);
    let goal = HexCoord::new(1, 0);
    let plain = solve_path(&plain_lane, start, goal, 100, 100);
    let mountain = solve_path(&mountain_lane, start, goal, 100, 100);

    assert_eq!(official_step_total(&plain_lane, start, &plain), Some(2));
    assert_eq!(official_step_total(&mountain_lane, start, &mountain), Some(3));
    assert_eq!(plain.steps.len(), mountain.steps.len(), "same geometry, one move each");
}

/// Roads keep the official 1 / 2 / 4 spread, which is the one part of the old
/// table that was already right — pinned so aligning plain/mountain does not
/// quietly disturb it.
#[test]
fn road_costs_keep_the_official_spread() {
    let map = lane_map("sbj.");
    for (col, want) in [(0, 1), (1, 2), (2, 4)] {
        let start = HexCoord::new(col, 0);
        let goal = HexCoord::new(col + 1, 0);
        let path = solve_path(&map, start, goal, 100, 100);
        assert_eq!(
            official_step_total(&map, start, &path),
            Some(want),
            "departing the road at col {col} costs {want}"
        );
    }
}

/// A pond is impassable, not merely expensive.
///
/// The official table encodes pond as `u8::MAX`, the sentinel meaning "you
/// may not be here" — a move billed at that cell is not a very expensive
/// move, it is an illegal one. So the budget must never *spend* its way past
/// a pond, and a budget large enough to pay for any path still must not
/// produce one that walks into the water.
#[test]
fn a_pond_is_impassable_rather_than_merely_expensive() {
    // Lane: plain, plain, pond, plain. The only way from col 0 to col 3 is
    // straight through the pond at col 2, which the solver must refuse
    // however generous the budget.
    let map = lane_map("..~.");

    let ctx = SolveContext::new(map.clone(), HexCoord::new(0, 0), HexCoord::new(3, 0), 100, 100)
        .expect("start and goal are both plain");
    let result = AstarSolver::default().solve(&ctx);
    assert!(
        result.path.is_none(),
        "a pond must be impassable regardless of budget, got {:?}",
        result.path.map(|p| p.steps)
    );

    // And the pond itself has no step cost at all — the official table
    // returns the impassable sentinel rather than a number to add up.
    assert_eq!(official_step_cost(CellKind::Pond), None);
}

// ---------------------------------------------------------------------------
// 5. The heuristic stays a lower bound in official units
// ---------------------------------------------------------------------------

/// The pruning heuristic counts remaining *moves*, which lower-bounds
/// remaining official steps because no move ever costs less than 1.
///
/// A* discards a node when `steps_so_far + h > max_steps`, which is only sound
/// while `h` cannot overstate what is left to pay. This asserts that on a
/// real returned path, every prefix obeys `spent + remaining_moves <=
/// max_steps` — the prune never cut a continuation the budget could have
/// still paid for, and `h` is never an over-estimate.
#[test]
fn the_prune_never_discards_a_prefix_the_budget_could_still_pay_for() {
    let map = plain_map(8, 8);
    let start = HexCoord::new(0, 4);
    let goal = HexCoord::new(4, 4);
    let max_steps = 8;

    let path = solve_path(&map, start, goal, 100, max_steps);
    let mut pos = start;
    let mut spent = 0;
    for (i, action) in path.steps.iter().enumerate() {
        if let Action::Move(dir) = action {
            let origin = map.get(pos).expect("prefix stays on the map");
            spent += official_step_cost(origin).expect("no pond under the path");
            pos = dir.step(pos);
        }
        let remaining = hex_distance(pos, goal);
        assert!(
            spent + remaining <= max_steps,
            "prefix {i} at {pos:?} has spent {spent} with {remaining} moves left, \
             which already exceeds the {max_steps}-step budget"
        );
    }
    assert_eq!(spent, max_steps, "the path lands exactly at the budget");
}
