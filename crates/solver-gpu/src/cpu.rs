//! CPU reference implementation of batch shortest-path on the hex map.
//!
//! Uses a binary-heap Dijkstra (O((V + E) log V)) — a single-source BFS
//! would also work when all costs are 1, but the contest map carries
//! weighted terrain costs so Dijkstra is the canonical reference. The
//! implementation is deterministic and side-effect-free so it can act as
//! the source of truth for future GPU parity tests.
//!
//! [`shortest_path_batch`] is the public entry point and is exercised by
//! the unit tests in `tests/shortest_path.rs`. The path-returning variants
//! ([`shortest_path_batch_paths`], [`shortest_path_with_predecessors`])
//! back the CPU fallback of the batched searcher added in #35.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use tracing::trace;

use crate::map::{HexCoord, Map};
use crate::{PathResult, Query};

/// Maximum representable cost on the map. The skeleton encodes per-cell
/// cost as `u8`, so the worst-case total path cost across `MAX_CELLS` hops
/// fits comfortably in `u32`.
const INFINITY_U: u32 = u32::MAX;

/// Compute shortest-path costs for a batch of queries against `map`.
///
/// Order of results matches the order of `queries`. `PathResult::cost` is
/// `None` when start or goal is off-map, or when the goal is unreachable
/// from the start (e.g. a blocked cell separates them).
pub fn shortest_path_batch(map: &Map, queries: &[Query]) -> Vec<PathResult> {
    queries.iter().map(|q| PathResult { cost: single(map, q.start, q.goal) }).collect()
}

/// Compute the single shortest-path cost from `start` to `goal`.
///
/// Returns `None` when either coordinate is off-map, when start equals goal
/// (cost 0 — but [`Some(0)`]), or when the goal is unreachable.
pub fn shortest_path(map: &Map, start: HexCoord, goal: HexCoord) -> Option<u32> {
    single(map, start, goal)
}

/// Compute the shortest-path **list of cells** from `start` to `goal`.
///
/// The returned vector lists coordinates in walk order, starting at
/// `start` and ending at `goal`. An empty vector signals unreachable / off-map.
/// When `start == goal` the returned path is exactly `[start]`.
pub fn shortest_path_cells(map: &Map, start: HexCoord, goal: HexCoord) -> Vec<HexCoord> {
    let Some(pred) = shortest_path_with_predecessors(map, start) else {
        return Vec::new();
    };
    reconstruct_path(map, start, goal, &pred)
}

/// Compute the shortest-path predecessor for every cell reachable from
/// `start`. `pred[i]` is the flat index of the cell whose edge improved the
/// best-known distance to `i`; `pred[i] == i` for the start cell. Cells
/// that are unreachable or off-map have `pred[i] == usize::MAX`.
///
/// Returns `None` if the map is empty or `start` is off-map.
pub fn shortest_path_with_predecessors(map: &Map, start: HexCoord) -> Option<Vec<usize>> {
    let n = map.len();
    if n == 0 {
        return None;
    }
    let start_idx = map.hex_index(start)?;

    let mut dist: Vec<u32> = vec![INFINITY_U; n];
    let mut pred: Vec<usize> = vec![usize::MAX; n];
    let mut heap: BinaryHeap<Reverse<(u32, usize)>> = BinaryHeap::with_capacity(n);

    dist[start_idx] = 0;
    pred[start_idx] = start_idx;
    heap.push(Reverse((0, start_idx)));

    while let Some(Reverse((d, u))) = heap.pop() {
        if d > dist[u] {
            continue;
        }
        let u_coord = flat_to_axial(u, map);
        for nbr in u_coord.neighbors() {
            let Some(v_idx) = map.hex_index(nbr) else {
                continue;
            };
            let Some(cell) = map.get(nbr) else {
                continue;
            };
            if cell.cost == 0 {
                continue; // impassable
            }
            let v_cost = u32::from(cell.cost);
            let alt = d.saturating_add(v_cost);
            if alt < dist[v_idx] {
                dist[v_idx] = alt;
                pred[v_idx] = u;
                heap.push(Reverse((alt, v_idx)));
            }
        }
    }
    Some(pred)
}

/// Batch variant of [`shortest_path_cells`]. Returns one path vector per
/// query, in the same order. Empty vectors denote unreachable / off-map.
pub fn shortest_path_batch_cells(map: &Map, queries: &[Query]) -> Vec<Vec<HexCoord>> {
    queries.iter().map(|q| shortest_path_cells(map, q.start, q.goal)).collect()
}

fn single(map: &Map, start: HexCoord, goal: HexCoord) -> Option<u32> {
    let n = map.len();
    if n == 0 {
        return None;
    }
    let start_idx = map.hex_index(start)?;
    let goal_idx = map.hex_index(goal)?;

    let mut dist: Vec<u32> = vec![INFINITY_U; n];
    let mut heap: BinaryHeap<Reverse<(u32, usize)>> = BinaryHeap::with_capacity(n);

    dist[start_idx] = 0;
    heap.push(Reverse((0, start_idx)));

    while let Some(Reverse((d, u))) = heap.pop() {
        if d > dist[u] {
            continue;
        }
        if u == goal_idx {
            trace!(cost = d, "dijkstra: reached goal");
            return Some(d);
        }
        let u_coord = flat_to_axial(u, map);
        for nbr in u_coord.neighbors() {
            let Some(v_idx) = map.hex_index(nbr) else {
                continue;
            };
            let Some(cell) = map.get(nbr) else {
                continue;
            };
            if cell.cost == 0 {
                continue; // impassable
            }
            let v_cost = u32::from(cell.cost);
            let alt = d.saturating_add(v_cost);
            if alt < dist[v_idx] {
                dist[v_idx] = alt;
                heap.push(Reverse((alt, v_idx)));
            }
        }
    }

    None
}

/// Reconstruct the start->goal path from a predecessor array.
/// Returns an empty vector when the goal is unreachable.
fn reconstruct_path(map: &Map, start: HexCoord, goal: HexCoord, pred: &[usize]) -> Vec<HexCoord> {
    let Some(start_idx) = map.hex_index(start) else {
        return Vec::new();
    };
    let Some(goal_idx) = map.hex_index(goal) else {
        return Vec::new();
    };
    if pred.get(goal_idx).copied().unwrap_or(usize::MAX) == usize::MAX {
        return Vec::new();
    }
    if start_idx == goal_idx {
        return vec![start];
    }

    let mut path: Vec<HexCoord> = Vec::new();
    let mut cur = goal_idx;
    let mut guard = 0usize;
    let limit = pred.len() + 1;
    while cur != start_idx {
        path.push(flat_to_axial(cur, map));
        let prev = pred[cur];
        if prev == usize::MAX || prev == cur {
            // Cycle or missing link — abort.
            return Vec::new();
        }
        cur = prev;
        guard += 1;
        if guard > limit {
            return Vec::new();
        }
    }
    path.push(flat_to_axial(start_idx, map));
    path.reverse();
    path
}

/// Reverse of `axial_to_offset` for the in-memory flat indexing. Pulled out
/// of the hot loop so we can keep `single` readable.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
fn flat_to_axial(idx: usize, map: &Map) -> HexCoord {
    let cols = usize::from(map.size.cols);
    let row = i32::try_from(idx / cols).expect("map row index fits in i32 (rows <= 32)");
    let col = i32::try_from(idx % cols).expect("map col index fits in i32 (cols <= 32)");
    let parity = (row as u32) & 1;
    let q = col.wrapping_sub((parity as i32).wrapping_mul(row / 2));
    HexCoord::new(q, row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform_map(rows: u8, cols: u8, cost: u8) -> Map {
        let mut grid = Vec::with_capacity(rows as usize);
        let row = vec![cost; cols as usize];
        for _ in 0..rows {
            grid.push(row.clone());
        }
        Map::from_costs(&grid, cols).expect("uniform map should build")
    }

    #[test]
    fn zero_distance_when_start_equals_goal() {
        let m = uniform_map(4, 4, 1);
        let c = HexCoord::new(0, 0);
        assert_eq!(shortest_path(&m, c, c), Some(0));
    }

    #[test]
    fn unreachable_when_target_off_map() {
        let m = uniform_map(2, 2, 1);
        let c = HexCoord::new(0, 0);
        assert_eq!(shortest_path(&m, c, HexCoord::new(99, 0)), None);
    }

    #[test]
    fn blocked_cells_force_a_detour() {
        // A wall of zero-cost cells in the middle column forces the path
        // to detour via row 1 / row 3. With all other cells at cost 1 the
        // direct axial distance is 4 (col 0 -> col 4); the detour through
        // row 1 takes 6 hops. We assert the path is at least 5 (>= direct).
        let mut grid = vec![vec![1u8; 5]; 5];
        for row in &mut grid {
            row[2] = 0;
        }
        let m = Map::from_costs(&grid, 5).unwrap();
        let left = HexCoord::new(0, 2);
        let right = HexCoord::new(4, 2);
        let dist =
            shortest_path(&m, left, right).expect("goal should be reachable around the wall");
        assert!(dist >= 5, "cost {dist} should be at least 5 due to detour");
    }

    #[test]
    fn path_cells_starts_at_start_ends_at_goal() {
        let m = uniform_map(4, 4, 1);
        let start = HexCoord::new(0, 0);
        let goal = HexCoord::new(3, 0);
        let path = shortest_path_cells(&m, start, goal);
        assert!(!path.is_empty(), "path must not be empty for reachable goal");
        assert_eq!(path.first().copied(), Some(start));
        assert_eq!(path.last().copied(), Some(goal));
    }

    #[test]
    fn path_cells_single_cell_when_start_equals_goal() {
        let m = uniform_map(2, 2, 1);
        let c = HexCoord::new(0, 0);
        assert_eq!(shortest_path_cells(&m, c, c), vec![c]);
    }

    #[test]
    fn path_cells_empty_when_unreachable() {
        let grid = vec![vec![1u8, 0, 1], vec![1u8, 0, 1]];
        let m = Map::from_costs(&grid, 3).unwrap();
        let path = shortest_path_cells(&m, HexCoord::new(0, 0), HexCoord::new(2, 0));
        assert!(path.is_empty(), "wall-blocked goal must produce empty path");
    }

    #[test]
    fn batch_cells_returns_one_path_per_query() {
        let m = uniform_map(3, 3, 1);
        let queries = vec![
            Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0)),
            Query::new(HexCoord::new(0, 0), HexCoord::new(0, 0)),
        ];
        let paths = shortest_path_batch_cells(&m, &queries);
        assert_eq!(paths.len(), queries.len());
        assert_eq!(paths[0].first().copied(), Some(queries[0].start));
        assert_eq!(paths[1], vec![queries[1].start]);
    }
}
