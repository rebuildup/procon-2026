//! Smoke tests for the GPU-host data transfer layer.
//!
//! These cover the host-side `GpuContext` API plus the
//! `GpuSolver` / `FallbackGpuSolver` integration that #36 introduces. The
//! tests do **not** require a real GPU device — the `wgpu` feature is
//! off in the default profile and the fallback path is what we exercise.
//!
//! # Coverage
//!
//! 1. Upload + readback-style inspection round-trips data faithfully.
//! 2. [`FallbackGpuSolver::run`] returns a valid plan for a small grid.
//! 3. The [`GpuSolver`] trait is dyn-compatible (object-safe), so the
//!    engine can pick a backend at runtime.

use solver_gpu::{
    Agent, AgentId, FallbackGpuSolver, GpuContext, GpuSolver, HexCoord, Kernel, Map, PathResult,
    Query, Spot, SpotId, TransferError,
};

/// Build a small 3x3 all-cost-1 grid for the tests below.
fn three_by_three() -> Map {
    let rows = vec![vec![1u8, 1, 1], vec![1, 1, 1], vec![1, 1, 1]];
    Map::from_costs(&rows, 3).expect("3x3 map should build")
}

/// Build a 3x3 grid with the middle cell blocked so we can verify the
/// fallback routes around the obstacle.
fn three_by_three_with_wall() -> Map {
    let rows = vec![
        vec![1u8, 1, 1],
        vec![1, 0, 1], // middle cell impassable
        vec![1, 1, 1],
    ];
    Map::from_costs(&rows, 3).unwrap()
}

#[test]
fn upload_and_inspect_round_trips_data() {
    let map = three_by_three();
    let agents = vec![
        Agent::new(AgentId(0), HexCoord::new(0, 0)),
        Agent::new(AgentId(1), HexCoord::new(2, 2)),
    ];
    let spots = vec![
        Spot::new(SpotId(0), HexCoord::new(1, 1)),
        Spot::new(SpotId(1), HexCoord::new(2, 0)),
        Spot::new(SpotId(2), HexCoord::new(0, 2)),
    ];

    let mut ctx = GpuContext::new();
    assert!(!ctx.has_state(), "fresh context must have no state");
    ctx.upload_state(&map, &agents, &spots).expect("upload should succeed");

    assert!(ctx.has_state());
    assert_eq!(ctx.grid().expect("grid present").size, map.size);
    assert_eq!(ctx.agents().len(), agents.len());
    assert_eq!(ctx.spots().len(), spots.len());
    for (a, expected) in ctx.agents().iter().zip(agents.iter()) {
        assert_eq!(a, expected, "agent payload preserved across upload");
    }
    for (s, expected) in ctx.spots().iter().zip(spots.iter()) {
        assert_eq!(s, expected, "spot payload preserved across upload");
    }

    // The read_back path is intentionally unavailable on the skeleton —
    // verify it returns the documented sentinel.
    ctx.dispatch(Kernel::ShortestPath { queries: vec![] }).unwrap();
    let err = ctx.read_back().unwrap_err();
    assert!(
        matches!(err, TransferError::PipelineUnavailable(_)),
        "read_back must surface PipelineUnavailable until the GPU pipeline lands, got {err:?}"
    );
}

#[test]
fn upload_state_rejects_off_grid_positions() {
    let map = three_by_three();
    let mut ctx = GpuContext::new();
    let bad_agent = Agent::new(AgentId(0), HexCoord::new(50, 50));
    let bad_spot = Spot::new(SpotId(0), HexCoord::new(20, 0));

    let err = ctx.upload_state(&map, &[bad_agent], &[]).unwrap_err();
    assert!(matches!(err, TransferError::InvalidInput(_)));

    let err = ctx.upload_state(&map, &[], &[bad_spot]).unwrap_err();
    assert!(matches!(err, TransferError::InvalidInput(_)));
}

#[test]
fn fallback_solver_returns_valid_plan() {
    // Verifies the CPU-backed GpuSolver matches the canonical Dijkstra
    // for a non-trivial case (path that must detour around a blocked
    // cell) — i.e. it returns a valid plan, not just `Some(0)`.
    let map = three_by_three_with_wall();
    let queries = vec![
        Query::new(HexCoord::new(0, 0), HexCoord::new(2, 2)),
        Query::new(HexCoord::new(2, 0), HexCoord::new(0, 2)),
        Query::new(HexCoord::new(0, 0), HexCoord::new(0, 0)),
        // Off-map: must report None.
        Query::new(HexCoord::new(0, 0), HexCoord::new(99, 99)),
    ];

    let mut ctx = GpuContext::new();
    ctx.upload_state(&map, &[], &[]).unwrap();
    ctx.dispatch(Kernel::ShortestPath { queries: queries.clone() }).unwrap();

    let results: Vec<PathResult> = FallbackGpuSolver.run(&ctx).unwrap();
    assert_eq!(results.len(), queries.len());

    // Indices 0/1 must be Some with positive cost (detour around the
    // wall costs at least 3 hops for the shorter route and 4 for the
    // longer one — we assert the floor is at least 3 to allow either
    // path through the grid while still ruling out the unblocked direct
    // route of 2 hops, which would imply the wall wasn't honored).
    assert!(results[0].cost.is_some(), "valid plan: cost is Some");
    assert!(results[0].cost.expect("cost") >= 3);
    assert!(results[1].cost.is_some());
    assert!(results[1].cost.expect("cost") >= 3);

    // Self-loop costs 0.
    assert_eq!(results[2].cost, Some(0));

    // Off-map returns None.
    assert_eq!(results[3].cost, None);
}

#[test]
fn fallback_solver_matches_direct_cpu_call() {
    // Parity check: FallbackGpuSolver must agree with
    // `cpu::shortest_path_batch` row-for-row.
    let map = three_by_three();
    let queries = vec![
        Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0)),
        Query::new(HexCoord::new(0, 0), HexCoord::new(0, 2)),
        Query::new(HexCoord::new(1, 1), HexCoord::new(1, 0)),
    ];
    let expected = solver_gpu::cpu::shortest_path_batch(&map, &queries);

    let mut ctx = GpuContext::new();
    ctx.upload_state(&map, &[], &[]).unwrap();
    ctx.dispatch(Kernel::ShortestPath { queries }).unwrap();
    let got = FallbackGpuSolver.run(&ctx).unwrap();

    assert_eq!(got, expected);
}

#[test]
fn gpu_solver_trait_is_dyn_compatible() {
    // The GpuSolver trait must be object-safe: the engine needs to be
    // able to store `Box<dyn GpuSolver>` and switch backends at runtime.
    let trait_object: Box<dyn GpuSolver> = Box::new(FallbackGpuSolver);
    let map = three_by_three();
    let mut ctx = GpuContext::new();
    ctx.upload_state(&map, &[], &[]).unwrap();
    ctx.dispatch(Kernel::ShortestPath {
        queries: vec![Query::new(HexCoord::new(0, 0), HexCoord::new(2, 2))],
    })
    .unwrap();

    let results = trait_object.run(&ctx).expect("dyn dispatch should succeed");
    assert_eq!(results.len(), 1);
    assert!(results[0].cost.is_some());

    // Also verify the reference form.
    let by_ref: &dyn GpuSolver = &FallbackGpuSolver;
    let results = by_ref.run(&ctx).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn dispatch_without_upload_is_rejected() {
    let mut ctx = GpuContext::new();
    let err = ctx
        .dispatch(Kernel::ShortestPath {
            queries: vec![Query::new(HexCoord::new(0, 0), HexCoord::new(1, 0))],
        })
        .unwrap_err();
    assert!(matches!(err, TransferError::InvalidInput(_)));
}

#[test]
fn kernel_query_count_matches_input() {
    let queries = vec![
        Query::new(HexCoord::new(0, 0), HexCoord::new(1, 0)),
        Query::new(HexCoord::new(1, 0), HexCoord::new(2, 0)),
        Query::new(HexCoord::new(2, 0), HexCoord::new(0, 0)),
    ];
    let k = Kernel::ShortestPath { queries };
    assert_eq!(k.query_count(), 3);
}

#[test]
fn multiple_dispatches_replace_last_kernel() {
    // The contract is "last_kernel wins" so re-dispatching overwrites the
    // previously-recorded kernel. Verify the state machine honors that.
    let map = three_by_three();
    let mut ctx = GpuContext::new();
    ctx.upload_state(&map, &[], &[]).unwrap();

    let q1 = Query::new(HexCoord::new(0, 0), HexCoord::new(1, 0));
    let q2 = Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0));
    ctx.dispatch(Kernel::ShortestPath { queries: vec![q1] }).unwrap();
    ctx.dispatch(Kernel::ShortestPath { queries: vec![q2, q1] }).unwrap();

    assert_eq!(ctx.last_kernel().map(Kernel::query_count), Some(2));

    // And FallbackGpuSolver uses the latest kernel, not the first.
    let results = FallbackGpuSolver.run(&ctx).unwrap();
    assert_eq!(results.len(), 2);
}
