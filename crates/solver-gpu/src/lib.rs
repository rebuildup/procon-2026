//! GPU-accelerated shortest-path solver skeleton.
//!
//! Provides a batch shortest-path interface that runs on the CPU today
//! (Dijkstra, [`cpu::shortest_path_batch`]) and is wired to a WGSL compute
//! shader ([`wgsl::SHORTEST`]) for future GPU execution. The shader text is
//! embedded as a `const &str` so it ships with the binary, satisfying the
//! "WGSL embedded in Rust" acceptance criterion.
//!
//! # Why this crate exists
//!
//! [`docs/contest/strategy.md §5.4`](../../../docs/contest/strategy.md) calls
//! for a parallel shortest-path primitive that can later be hoisted onto a
//! WebGPU compute shader. Issue #34 establishes the minimum viable shape —
//! types, embedded shader, CPU fallback, and a feature-gated GPU stub — so
//! that follow-up tickets (#36 path-search logic, #37 batch eval, …) can
//! plug in without restructuring the crate boundary.
//!
//! # Module layout
//!
//! - [`map`] — minimal hex map types (`HexCoord`, `MapSize`, `Cell`, `Map`)
//!   so the crate does not have to depend on `domain` while #18 is still in
//!   flight.
//! - [`cpu`] — deterministic Dijkstra reference implementation, used by the
//!   unit tests and as the canonical CPU result for future GPU parity tests.
//! - [`wgsl`] — embedded WGSL source. The text is validated at build time
//!   only via `include_str!`; full `naga` validation belongs to the GPU
//!   integration work in a later ticket.
//! - [`gpu`] — feature-gated (`wgpu`) stub exposing the buffer / bind-group
//!   plumbing the compute shader will need once a real device is wired in.
//! - [`transfer`] — host-side upload / dispatch / read-back layer plus the
//!   [`GpuSolver`] trait and [`FallbackGpuSolver`] (issue #36).
//! - [`batched`] — `BatchedSearcher`, the parallel workgroup-shared-memory
//!   path-search driver added in #35. Provides a CPU fallback so callers
//!   never see a hard GPU requirement.
//!
//! Both `wasm32` and native builds compile: `wasm32` simply disables the
//! `wgpu` feature, falling back to the CPU implementation.

#![doc(test(attr(deny(warnings))))]

pub mod cpu;
pub mod map;
pub mod transfer;
pub mod wgsl;

#[cfg(feature = "wgpu")]
pub mod batched;
#[cfg(feature = "wgpu")]
pub mod gpu;

pub use map::{Cell, HexCoord, MAX_CELLS, MAX_MAP_DIM, Map, MapError, MapSize};
pub use transfer::{Agent, AgentId, GpuContext, HexGrid, Kernel, Spot, SpotId, TransferError};

/// One source–destination pair for batch shortest-path evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Query {
    pub start: HexCoord,
    pub goal: HexCoord,
}

impl Query {
    pub const fn new(start: HexCoord, goal: HexCoord) -> Self {
        Self { start, goal }
    }
}

/// Result of evaluating a single query. `None` means the goal was not
/// reachable from the start within the map (or the start/goal was off-map).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathResult {
    pub cost: Option<u32>,
}

/// Batch of [`Query`] items with a uniform [`Map`] reference. This is the
/// shape uploaded to the GPU as a storage buffer; the CPU fallback takes the
/// same shape so parity tests can compare results directly.
#[derive(Debug, Clone)]
pub struct BatchShortestPath<'m> {
    pub map: &'m Map,
    pub queries: Vec<Query>,
}

impl<'m> BatchShortestPath<'m> {
    pub fn new(map: &'m Map, queries: Vec<Query>) -> Self {
        Self { map, queries }
    }

    /// Solve the batch on the CPU. Always available — no GPU required.
    ///
    /// This is the canonical reference for future GPU-parity tests.
    pub fn solve_cpu(&self) -> Vec<PathResult> {
        cpu::shortest_path_batch(self.map, &self.queries)
    }

    /// Solve the batch on the GPU via [`wgpu`]. Returns `Err` when the
    /// device does not expose a compatible queue (most CI / wasm32 builds).
    /// The skeleton does not yet ship a real GPU pipeline — see #37.
    #[cfg(feature = "wgpu")]
    pub fn solve_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<PathResult>, gpu::GpuError> {
        gpu::solve_batch(device, queue, self.map, &self.queries)
    }
}

/// Unified solver trait that hides the GPU / CPU boundary.
///
/// [`GpuSolver::run`] consumes the state already uploaded into
/// [`GpuContext`] and returns one [`PathResult`] per query in the most
/// recently dispatched [`Kernel::ShortestPath`] kernel. The trait is
/// **dyn-compatible** (`&dyn GpuSolver` works) so the engine can pick a
/// backend at runtime based on device availability.
///
/// # Examples
///
/// ```
/// use solver_gpu::{
///     FallbackGpuSolver, GpuContext, GpuSolver, HexCoord, Kernel, Map, Query,
/// };
/// let grid = vec![vec![1u8, 1, 1], vec![1, 1, 1]];
/// let map = Map::from_costs(&grid, 3).unwrap();
/// let mut ctx = GpuContext::new();
/// ctx.upload_state(&map, &[], &[]).unwrap();
/// let q = Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0));
/// ctx.dispatch(Kernel::ShortestPath { queries: vec![q] }).unwrap();
///
/// let solver: &dyn GpuSolver = &FallbackGpuSolver;
/// let results = solver.run(&ctx).unwrap();
/// assert_eq!(results.len(), 1);
/// ```
///
/// The same call signature works against a future `WgpuGpuSolver` that
/// runs the real compute pipeline; the engine does not need to branch.
pub trait GpuSolver {
    /// Execute the most-recently dispatched kernel on `ctx` and return the
    /// results. The solver may consume the CPU shadow state in `ctx`
    /// directly (CPU fallback) or schedule a GPU read-back (real backend).
    ///
    /// # Errors
    ///
    /// Returns [`TransferError::NothingToRead`] when `ctx` has not been
    /// populated with a dispatchable kernel. CPU-fallback implementations
    /// additionally reject an un-uploaded [`GpuContext`] via
    /// [`TransferError::InvalidInput`].
    fn run(&self, ctx: &GpuContext) -> Result<Vec<PathResult>, TransferError>;
}

/// CPU-backed [`GpuSolver`] implementation. Always available — used in
/// CI, `wasm32`, and any environment where a real GPU device cannot be
/// requested. Delegates directly to [`cpu::shortest_path_batch`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FallbackGpuSolver;

impl GpuSolver for FallbackGpuSolver {
    fn run(&self, ctx: &GpuContext) -> Result<Vec<PathResult>, TransferError> {
        let grid = ctx.grid().ok_or(TransferError::InvalidInput("upload_state was not called"))?;
        let kernel = ctx.last_kernel().ok_or(TransferError::NothingToRead)?;
        let queries = match kernel {
            Kernel::ShortestPath { queries } => queries.clone(),
        };
        Ok(cpu::shortest_path_batch(grid, &queries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_map() -> Map {
        let grid = vec![vec![1u8, 1, 1], vec![1, 1, 1]];
        Map::from_costs(&grid, 3).unwrap()
    }

    #[test]
    fn fallback_solver_returns_one_result_per_query() {
        let map = tiny_map();
        let q1 = Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0));
        let q2 = Query::new(HexCoord::new(0, 0), HexCoord::new(0, 0));
        let q3 = Query::new(HexCoord::new(0, 0), HexCoord::new(99, 99));

        let mut ctx = GpuContext::new();
        ctx.upload_state(&map, &[], &[]).unwrap();
        ctx.dispatch(Kernel::ShortestPath { queries: vec![q1, q2, q3] }).unwrap();

        let results = FallbackGpuSolver.run(&ctx).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].cost, cpu::shortest_path(&map, q1.start, q1.goal));
        assert_eq!(results[1].cost, Some(0));
        assert_eq!(results[2].cost, None);
    }

    #[test]
    fn fallback_solver_rejects_empty_context() {
        let ctx = GpuContext::new();
        let err = FallbackGpuSolver.run(&ctx).unwrap_err();
        assert!(matches!(err, TransferError::InvalidInput(_)));
    }

    #[test]
    fn gpu_solver_trait_is_dyn_compatible() {
        // If this compiles, the trait is dyn-compatible (`&dyn GpuSolver`
        // and `Box<dyn GpuSolver>` are both legal).
        let solver: &dyn GpuSolver = &FallbackGpuSolver;
        let map = tiny_map();
        let mut ctx = GpuContext::new();
        ctx.upload_state(&map, &[], &[]).unwrap();
        let q = Query::new(HexCoord::new(0, 0), HexCoord::new(1, 0));
        ctx.dispatch(Kernel::ShortestPath { queries: vec![q] }).unwrap();
        let results = solver.run(&ctx).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn gpu_solver_boxed_dyn_dispatch() {
        // Exercise the boxed dyn path explicitly so trait-object safety
        // is verified end-to-end.
        let solver: Box<dyn GpuSolver> = Box::new(FallbackGpuSolver);
        let map = tiny_map();
        let mut ctx = GpuContext::new();
        ctx.upload_state(&map, &[], &[]).unwrap();
        ctx.dispatch(Kernel::ShortestPath { queries: vec![] }).unwrap();
        let results = solver.run(&ctx).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn fallback_solver_matches_batch_solver_struct() {
        // Cross-check: FallbackGpuSolver and the existing
        // BatchShortestPath::solve_cpu must agree result-by-result.
        let map = tiny_map();
        let queries = vec![
            Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0)),
            Query::new(HexCoord::new(1, 0), HexCoord::new(1, 1)),
        ];
        let batch = BatchShortestPath::new(&map, queries.clone());
        let via_batch = batch.solve_cpu();

        let mut ctx = GpuContext::new();
        ctx.upload_state(&map, &[], &[]).unwrap();
        ctx.dispatch(Kernel::ShortestPath { queries }).unwrap();
        let via_fallback = FallbackGpuSolver.run(&ctx).unwrap();

        assert_eq!(via_batch, via_fallback);
    }
}
