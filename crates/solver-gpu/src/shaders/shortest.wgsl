// SPDX-License-Identifier: MIT OR Apache-2.0
//
// shortest.wgsl — batch shortest-path compute shader skeleton.
//
// Each invocation evaluates a single (start, goal) query on a shared
// hex grid (max 32x32 = 1024 cells) using a parallel wavefront BFS-like
// relaxation. The skeleton implements a proper synchronous wavefront over
// 1024 iterations; callers can swap in an asynchronous frontier ring
// buffer later without changing the public Rust API.
//
// Bindings:
//   @group(0) @binding(0) map         uniform hex cost grid + dimensions
//   @group(0) @binding(1) queries     storage read  : array<Query, Q>
//   @group(0) @binding(2) distances   storage write : array<u32, Q>
//   @group(0) @binding(3) reachable   storage write : array<u32, Q>  (1 = reachable)

const MAX_CELLS: u32 = 1024u;
const INFINITY_U: u32 = 0xFFFFFFFFu;
const UNREACHABLE: u32 = 0xFFFFFFFFu;

// Map metadata. Layout chosen so the struct stays 16-byte aligned.
struct Map {
    rows: u32,
    cols: u32,
    cell_count: u32,
    _pad: u32,
    // Costs are a fixed-size storage array so the shader works for any map
    // up to the 32x32 limit without dynamic array binding in WGSL.
    costs: array<u32, 1024>,
};

// Single query: start hex and goal hex in axial coordinates.
struct Query {
    start_q: i32,
    start_r: i32,
    goal_q: i32,
    goal_r: i32,
};

@group(0) @binding(0) var<uniform> map: Map;
@group(0) @binding(1) var<storage, read> queries: array<Query>;
@group(0) @binding(2) var<storage, read_write> distances: array<u32>;
@group(0) @binding(3) var<storage, read_write> reachable: array<u32>;

/// Compute the flat (row-major) index of a hex coord in the grid.
/// Returns u32::MAX when the coord is outside the map.
fn index_of(q: i32, r: i32) -> u32 {
    if (r < 0) || (r >= i32(map.rows)) {
        return MAX_CELLS;
    }
    let offset = axial_to_offset(q, r);
    let c = offset[0];
    if (c < 0) || (c >= i32(map.cols)) {
        return MAX_CELLS;
    }
    return u32(r) * map.cols + u32(c);
}

/// Convert axial (q, r) to (col, row) using even-r offset coordinates.
/// Returns ([-1,*], *) when the hex is outside the rectangular bounds.
fn axial_to_offset(q: i32, r: i32) -> vec2<i32> {
    let parity = u32(r) & 1u;
    var c = q + (i32(parity) * (r / 2));
    return vec2<i32>(c, r);
}

/// Six axial neighbors (clockwise from east).
fn neighbors(q: i32, r: i32) -> array<vec2<i32>, 6> {
    return array<vec2<i32>, 6>(
        vec2<i32>(q + 1, r),
        vec2<i32>(q + 1, r - 1),
        vec2<i32>(q,     r - 1),
        vec2<i32>(q - 1, r),
        vec2<i32>(q - 1, r + 1),
        vec2<i32>(q,     r + 1),
    );
}

/// Read the cost of a cell, returning INFINITY_U when out of bounds.
fn cost_at(idx: u32) -> u32 {
    if (idx >= map.cell_count) {
        return INFINITY_U;
    }
    let c = map.costs[idx];
    if (c == 0u) {
        // 0 means blocked/impassable; treat as infinite cost.
        return INFINITY_U;
    }
    return c;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let q_count = arrayLength(&queries);
    if (gid.x >= q_count) {
        return;
    }

    let query = queries[gid.x];

    // Per-thread distance buffer (lives in registers / private memory).
    var dist = array<u32, 1024>();
    for (var i = 0u; i < 1024u; i = i + 1u) {
        dist[i] = INFINITY_U;
    }

    let start_idx = index_of(query.start_q, query.start_r);
    let goal_idx = index_of(query.goal_q, query.goal_r);
    if (start_idx >= map.cell_count) || (goal_idx >= map.cell_count) {
        distances[gid.x] = UNREACHABLE;
        reachable[gid.x] = 0u;
        return;
    }

    dist[start_idx] = 0u;

    // Synchronous wavefront relaxation. With at most 1024 cells we know the
    // shortest path cannot exceed 1023 hops, so 1024 iterations suffice.
    for (var iter = 0u; iter < 1024u; iter = iter + 1u) {
        var changed = 0u;
        for (var idx = 0u; idx < map.cell_count; idx = idx + 1u) {
            let d = dist[idx];
            if (d == INFINITY_U) {
                continue;
            }
            let q = i32(idx % map.cols);
            let r = i32(idx / map.cols);
            // Recover axial from offset (even-r inverse).
            let parity = u32(r) & 1u;
            let aq = q - i32(parity) * (r / 2);
            let ar = r;
            let nbrs = neighbors(aq, ar);
            for (var k = 0u; k < 6u; k = k + 1u) {
                let n_idx = index_of(nbrs[k][0], nbrs[k][1]);
                if (n_idx >= map.cell_count) {
                    continue;
                }
                let step = cost_at(n_idx);
                if (step == INFINITY_U) {
                    continue;
                }
                let alt = d + step;
                if (alt < dist[n_idx]) {
                    dist[n_idx] = alt;
                    changed = changed + 1u;
                }
            }
        }
        if (changed == 0u) {
            break;
        }
    }

    let final = dist[goal_idx];
    distances[gid.x] = final;
    reachable[gid.x] = select(0u, 1u, final != UNREACHABLE);
}