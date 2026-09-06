//! CPU reference implementation of batch shortest-path on the hex map.
//!
//! Uses a binary-heap Dijkstra (O((V + E) log V)) — a single-source BFS
//! would also work when all costs are 1, but the contest map carries
//! weighted terrain costs so Dijkstra is the canonical reference. The
//! implementation is deterministic and side-effect-free so it can act as
//! the source of truth for future GPU parity tests.
//!
//! [`shortest_path_batch`] is the public entry point and is exercised by
//! the unit tests in `tests/shortest_path.rs`.

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
}
