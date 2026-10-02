// SPDX-License-Identifier: MIT OR Apache-2.0
//
// batched_search.wgsl — batched path search compute kernel.
//
// Each workgroup evaluates one (start, goal, agent_id) query on a shared
// hex grid (max 32x32 = 1024 cells) using a parallel synchronous wavefront
// BFS-style relaxation in workgroup shared memory.
//
// Bindings:
//   @group(0) @binding(0) map          uniform hex cost grid + dimensions
//   @group(0) @binding(1) queries      storage read  : array<Query, Q>
//   @group(0) @binding(2) distances    storage write : array<u32, Q>      (per-query final cost)
//   @group(0) @binding(3) predecessors storage write : array<i32, Q * 1024> (per-query per-cell predecessor flat idx; -1 = none)

const MAX_CELLS: u32 = 1024u;
const INFINITY_U: u32 = 0xFFFFFFFFu;
const UNREACHABLE: u32 = 0xFFFFFFFFu;
const WORKGROUP_SIZE: u32 = 64u;
const CELLS_PER_THREAD: u32 = MAX_CELLS / WORKGROUP_SIZE;  // 16

// Map metadata + per-cell costs. Bound as a storage buffer (read-only) so
// naga's uniform 16-byte alignment constraint does not force us to repack
// the cost array as vec4. The layout still matches the host struct.
struct Map {
    rows: u32,
    cols: u32,
    cell_count: u32,
    _pad: u32,
    costs: array<u32, 1024>,
};

// Single query: start hex, goal hex in axial coordinates, plus an agent_id
// forwarded to the host so callers can correlate GPU results back to their
// original request ordering (in particular when a batch is sharded).
struct Query {
    start_q: i32,
    start_r: i32,
    goal_q: i32,
    goal_r: i32,
    agent_id: u32,
};

@group(0) @binding(0) var<storage, read> map: Map;
@group(0) @binding(1) var<storage, read> queries: array<Query>;
@group(0) @binding(2) var<storage, read_write> distances: array<u32>;
@group(0) @binding(3) var<storage, read_write> predecessors: array<i32>;

// Workgroup-shared per-query state. Each workgroup handles one Query, so the
// shared distance / predecessor arrays never need to be larger than MAX_CELLS.
var<workgroup> shared_dist: array<u32, 1024>;
var<workgroup> shared_pred: array<i32, 1024>;
var<workgroup> shared_changed: array<u32, 64>;
var<workgroup> shared_start_idx: u32;
var<workgroup> shared_goal_idx: u32;
var<workgroup> shared_terminated: u32;  // 0 = continue, 1 = goal reached

/// Compute the flat (row-major) index of a hex coord in the grid.
/// Returns MAX_CELLS (= u32 sentinel) when the coord is outside the map.
fn index_of(q: i32, r: i32) -> u32 {
    if (r < 0) || (r >= i32(map.rows)) {
        return MAX_CELLS;
    }
    let parity = u32(r) & 1u;
    let c = q + i32(parity) * (r / 2);
    if (c < 0) || (c >= i32(map.cols)) {
        return MAX_CELLS;
    }
    return u32(r) * map.cols + u32(c);
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

/// Read the cost of a cell at flat index `idx`. Returns INFINITY_U when the
/// index is out of bounds or the cell is blocked (cost 0).
fn cost_at(idx: u32) -> u32 {
    if (idx >= map.cell_count) {
        return INFINITY_U;
    }
    let c = map.costs[idx];
    if (c == 0u) {
        return INFINITY_U;
    }
    return c;
}

/// Convert a flat index back to axial (q, r).
fn axial_of(idx: u32) -> vec2<i32> {
    let c = i32(idx % map.cols);
    let r = i32(idx / map.cols);
    let parity = u32(r) & 1u;
    let q = c - i32(parity) * (r / 2);
    return vec2<i32>(q, r);
}

@compute @workgroup_size(64)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let q_idx = gid.x;
    let q_count = arrayLength(&queries);
    if (q_idx >= q_count) {
        return;
    }

    let query = queries[q_idx];

    // ---- Init shared memory ----
    // Each thread clears its slice of the distance + predecessor arrays.
    for (var i = lid; i < MAX_CELLS; i = i + WORKGROUP_SIZE) {
        shared_dist[i] = INFINITY_U;
        shared_pred[i] = -1;
    }
    shared_changed[lid] = 0u;
    workgroupBarrier();

    // ---- Resolve start / goal flat indices ----
    let start_idx = index_of(query.start_q, query.start_r);
    let goal_idx = index_of(query.goal_q, query.goal_r);
    if (lid == 0u) {
        shared_start_idx = start_idx;
        shared_goal_idx = goal_idx;
        shared_terminated = 0u;
    }
    workgroupBarrier();

    // Off-map start or goal -> mark unreachable and exit early.
    if (shared_start_idx >= map.cell_count) || (shared_goal_idx >= map.cell_count) {
        if (lid == 0u) {
            distances[q_idx] = UNREACHABLE;
        }
        for (var i = lid; i < MAX_CELLS; i = i + WORKGROUP_SIZE) {
            predecessors[q_idx * MAX_CELLS + i] = -1;
        }
        return;
    }

    // Seed the BFS frontier at the start cell.
    if (lid == 0u) {
        shared_dist[shared_start_idx] = 0u;
        shared_pred[shared_start_idx] = i32(shared_start_idx);
    }
    workgroupBarrier();

    // ---- Parallel synchronous wavefront ----
    // With at most 1024 cells the shortest path cannot exceed 1023 hops, so
    // 1024 iterations of relaxation suffice. Each thread relaxes its slice of
    // the cell index space; we accumulate `changed` into a per-thread atomic
    // then sum across the workgroup via workgroup-shared memory + barrier.
    let my_lo = lid * CELLS_PER_THREAD;
    let my_hi = my_lo + CELLS_PER_THREAD;

    for (var iter = 0u; iter < 1024u; iter = iter + 1u) {
        if (shared_terminated == 1u) {
            break;
        }

        var local_changed: u32 = 0u;

        for (var idx = my_lo; idx < my_hi; idx = idx + 1u) {
            let d = shared_dist[idx];
            if (d == INFINITY_U) {
                continue;
            }

            let axial = axial_of(idx);
            let nbrs = neighbors(axial[0], axial[1]);
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
                if (alt < shared_dist[n_idx]) {
                    shared_dist[n_idx] = alt;
                    shared_pred[n_idx] = i32(idx);
                    local_changed = local_changed + 1u;
                }
            }
        }

        // Publish this thread's `changed` and accumulate the workgroup total.
        shared_changed[lid] = local_changed;
        workgroupBarrier();

        var total: u32 = 0u;
        for (var i = 0u; i < WORKGROUP_SIZE; i = i + 1u) {
            total = total + shared_changed[i];
        }
        workgroupBarrier();

        // Early exit: no progress means frontier is empty.
        if (total == 0u) {
            break;
        }

        // Early exit: goal reached with its final cost.
        if (shared_dist[shared_goal_idx] != INFINITY_U) {
            if (lid == 0u) {
                shared_terminated = 1u;
            }
        }
        workgroupBarrier();
    }

    // ---- Publish results ----
    let final_cost = shared_dist[shared_goal_idx];
    if (lid == 0u) {
        distances[q_idx] = final_cost;
    }

    // Write per-query predecessors: 1024 i32 entries per query, slot at offset q_idx * MAX_CELLS.
    for (var i = lid; i < MAX_CELLS; i = i + WORKGROUP_SIZE) {
        predecessors[q_idx * MAX_CELLS + i] = shared_pred[i];
    }
}