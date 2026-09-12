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
//!
//! Both `wasm32` and native builds compile: `wasm32` simply disables the
//! `wgpu` feature, falling back to the CPU implementation.

#![doc(test(attr(deny(warnings))))]

pub mod cpu;
pub mod map;
pub mod wgsl;

#[cfg(feature = "wgpu")]
pub mod gpu;

pub use map::{Cell, HexCoord, MAX_CELLS, MAX_MAP_DIM, Map, MapError, MapSize};

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
