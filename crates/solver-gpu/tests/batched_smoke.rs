//! Integration tests for the batched path-search solver.
//!
//! These tests cover the CPU fallback path so they always pass in CI even
//! when no real GPU device is available. The GPU pipeline is exercised by
//! the in-module tests under `src/batched.rs` when the `wgpu` feature is
//! enabled.
//!
//! The CPU fallback path is exposed by [`solver_gpu::cpu::shortest_path_batch_cells`],
//! which is the deterministic Dijkstra reference used by the searcher when
//! `SOLVER_GPU_DISABLE=1` is set in the environment or when GPU device
//! creation fails.

use solver_gpu::cpu::shortest_path_batch_cells;
use solver_gpu::{HexCoord, Map, Query};

fn uniform(rows: u8, cols: u8, cost: u8) -> Map {
    let row = vec![cost; cols as usize];
    let grid = vec![row; rows as usize];
    Map::from_costs(&grid, cols).expect("uniform map should build")
}

#[test]
fn single_query_case_produces_correct_path() {
    let map = uniform(4, 4, 1);
    let start = HexCoord::new(0, 0);
    let goal = HexCoord::new(3, 0);

    let paths = shortest_path_batch_cells(&map, &[Query::new(start, goal)]);
    assert_eq!(paths.len(), 1);
    let path = &paths[0];
    assert!(!path.is_empty(), "reachable goal must yield a non-empty path");
    assert_eq!(path.first().copied(), Some(start), "path must start at start");
    assert_eq!(path.last().copied(), Some(goal), "path must end at goal");

    // Every consecutive pair must be a hex neighbor (consistency check).
    for w in path.windows(2) {
        let diff = (w[0].q - w[1].q, w[0].r - w[1].r);
        let is_neighbor = HexCoord::NEIGHBORS.iter().any(|n| n.q == diff.0 && n.r == diff.1);
        assert!(is_neighbor, "step {w:?} -> {:?} is not a hex neighbor", w[1]);
    }
}

#[test]
fn batch_of_four_produces_four_paths() {
    let map = uniform(4, 4, 1);
    // All axial coords below must lie inside the 4x4 map. Note that
    // HexCoord::new(q, r) maps to offset col = q + parity*(r/2), so for
    // row 3 (parity 1) only q in [-1, 2] fits.
    let queries = vec![
        Query::new(HexCoord::new(0, 0), HexCoord::new(3, 0)),
        Query::new(HexCoord::new(0, 0), HexCoord::new(0, 0)),
        Query::new(HexCoord::new(1, 1), HexCoord::new(2, 2)),
        Query::new(HexCoord::new(2, 3), HexCoord::new(0, 0)),
    ];

    let paths = shortest_path_batch_cells(&map, &queries);

    assert_eq!(paths.len(), 4, "batch of 4 must produce 4 results");
    for (path, query) in paths.iter().zip(&queries) {
        assert!(!path.is_empty(), "each reachable query must yield a path");
        assert_eq!(path.first().copied(), Some(query.start));
        assert_eq!(path.last().copied(), Some(query.goal));
    }

    // The second query has start == goal, so its path is a single cell.
    assert_eq!(paths[1], vec![queries[1].start]);
}

#[test]
fn cpu_fallback_works_when_gpu_unavailable() {
    // The CPU fallback is the deterministic Dijkstra in `cpu.rs`. Even when
    // no GPU device is available (CI, wasm32, no adapter), this path must
    // produce correct results. We verify the public CPU surface here; the
    // `BatchedSearcher` delegates to the same function when its `wgpu`
    // bring-up fails or when `SOLVER_GPU_DISABLE=1` is set in the environment.
    let map = uniform(3, 3, 1);
    let queries = vec![
        Query::new(HexCoord::new(0, 0), HexCoord::new(2, 1)),
        // off-map goal must produce an empty path, not a panic
        Query::new(HexCoord::new(0, 0), HexCoord::new(99, 99)),
    ];
    let paths = shortest_path_batch_cells(&map, &queries);

    assert_eq!(paths.len(), 2);
    assert!(!paths[0].is_empty(), "reachable goal must yield a path");
    assert!(paths[1].is_empty(), "off-map goal must yield an empty path");
}

#[test]
fn empty_batch_returns_empty_paths() {
    let map = uniform(2, 2, 1);
    let paths = shortest_path_batch_cells(&map, &[]);
    assert!(paths.is_empty(), "empty batch must return empty Vec");
}

#[test]
fn unreachable_goal_produces_empty_path() {
    let grid = vec![vec![1u8, 0, 1], vec![1u8, 0, 1]];
    let map = Map::from_costs(&grid, 3).unwrap();
    let paths =
        shortest_path_batch_cells(&map, &[Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0))]);
    assert_eq!(paths.len(), 1);
    assert!(paths[0].is_empty(), "wall-blocked goal must yield empty path");
}
