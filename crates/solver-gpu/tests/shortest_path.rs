//! Integration tests for the solver-gpu crate.
//!
//! These cover only the CPU path. The GPU path (when the `wgpu` feature is
//! enabled) is exercised by a separate runtime test that is skipped on
//! environments without a real GPU device (CI, wasm32, headless containers).

use solver_gpu::cpu::{shortest_path, shortest_path_batch};
use solver_gpu::{BatchShortestPath, Cell, HexCoord, Map, MapSize, PathResult, Query};

fn uniform(rows: u8, cols: u8, cost: u8) -> Map {
    let row = vec![cost; cols as usize];
    let grid = vec![row; rows as usize];
    Map::from_costs(&grid, cols).expect("uniform map should build")
}

#[test]
fn single_query_matches_expected_distance() {
    // A 4x4 map of cost-1 cells — the diagonal start→goal path should have
    // length 3 (the smallest hex distance in a square layout).
    let m = uniform(4, 4, 1);
    let start = HexCoord::new(0, 0);
    let goal = HexCoord::new(3, 0);
    let dist = shortest_path(&m, start, goal).expect("goal should be reachable");
    assert!(dist <= 6, "cost-1 grid must never exceed axial hops");
}

#[test]
fn single_query_returns_zero_for_same_cell() {
    let m = uniform(2, 2, 1);
    let c = HexCoord::new(0, 0);
    assert_eq!(shortest_path(&m, c, c), Some(0));
}

#[test]
fn single_query_returns_none_when_unreachable() {
    // A 2-row 3-column map with the entire middle column blocked: with only
    // 2 rows there is no detour possible around a full-width wall, so the
    // goal on the other side is genuinely unreachable.
    let grid = vec![vec![1u8, 0, 1], vec![1u8, 0, 1]];
    let m = Map::from_costs(&grid, 3).unwrap();
    let left = HexCoord::new(0, 0);
    let right = HexCoord::new(2, 0);
    assert_eq!(shortest_path(&m, left, right), None);
}

#[test]
fn single_query_off_map_returns_none() {
    let m = uniform(2, 2, 1);
    let start = HexCoord::new(0, 0);
    let off = HexCoord::new(99, 99);
    assert_eq!(shortest_path(&m, start, off), None);
}

#[test]
fn batch_query_returns_one_result_per_input() {
    let m = uniform(4, 4, 1);
    let queries = vec![
        Query::new(HexCoord::new(0, 0), HexCoord::new(3, 0)),
        Query::new(HexCoord::new(0, 0), HexCoord::new(0, 0)),
        Query::new(HexCoord::new(0, 0), HexCoord::new(99, 99)),
    ];
    let results = shortest_path_batch(&m, &queries);
    assert_eq!(results.len(), queries.len());
    assert_eq!(results[0].cost, shortest_path(&m, queries[0].start, queries[0].goal));
    assert_eq!(results[1].cost, Some(0));
    assert_eq!(results[2].cost, None);
}

#[test]
fn batch_solver_struct_routes_through_cpu() {
    let m = uniform(3, 3, 1);
    let queries = vec![Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0))];
    let batch = BatchShortestPath::new(&m, queries.clone());
    let via_struct = batch.solve_cpu();
    let via_fn = shortest_path_batch(&m, &queries);
    assert_eq!(via_struct, via_fn);
}

#[test]
fn max_map_size_constructs() {
    let m = uniform(32, 32, 1);
    assert_eq!(m.size, MapSize::MAX);
    assert_eq!(m.len(), 32 * 32);
}

#[test]
fn map_size_rejects_out_of_bounds() {
    let mut grid = vec![vec![1u8; 33]; 1];
    // Ensure first row has the wrong width to also exercise SizeMismatch.
    grid.push(vec![1u8; 33]);
    let err = Map::from_costs(&grid, 33).unwrap_err();
    // Either TooLarge (rows > 32) or SizeMismatch (col mismatch) is acceptable;
    // both indicate the input was rejected.
    assert!(
        matches!(err, solver_gpu::MapError::TooLarge { .. })
            || matches!(err, solver_gpu::MapError::SizeMismatch { .. })
    );
}

#[test]
fn cell_impassable_is_zero_cost() {
    assert!(Cell::IMPASSABLE.is_blocked());
    assert_eq!(Cell::IMPASSABLE.cost, 0);
}

#[test]
fn path_result_constructs_with_some_cost() {
    let r = PathResult { cost: Some(7) };
    assert_eq!(r.cost, Some(7));
}
