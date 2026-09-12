//! Host-side GPU data transfer layer.
//!
//! This module is the "host side" counterpart to the WGSL compute shaders in
//! [`crate::wgsl`]. It models the data-flow the eventual GPU pipeline will
//! need:
//!
//! 1. [`upload_state`] copies the per-tick game state (`grid`, `agents`,
//!    `spots`) into the [`GpuContext`].
//! 2. [`GpuContext::dispatch`] issues the chosen [`Kernel`] against the
//!    uploaded buffers.
//! 3. [`GpuContext::read_back`] copies the kernel results back to the CPU.
//!
//! The current crate scope is **shortest-path over the hex grid** (see
//! [`crate::BatchShortestPath`]), so [`Kernel::ShortestPath`] is the only
//! kernel today. The module is intentionally small and trait-friendly so
//! that follow-up tickets can drop in additional kernels (refuel planning,
//! spot assignment, ...) without breaking the public surface.
//!
//! ## Feature flag
//!
//! The `wgpu` feature wires up real GPU buffers / queue / device. Without
//! the feature, [`GpuContext`] still holds the CPU-side shadow state and
//! every method works — [`GpuContext::dispatch`] returns
//! [`TransferError::PipelineUnavailable`] so callers can detect "GPU not
//! compiled in" without panicking. This mirrors the pattern in
//! [`crate::gpu`].
//!
//! ## Fallback path
//!
//! The [`crate::GpuSolver`] trait abstracts over the GPU / CPU boundary.
//! [`crate::FallbackGpuSolver`] implements the trait by delegating to the
//! CPU Dijkstra in [`crate::cpu`], so callers never have to special-case
//! "no GPU available" themselves.

use thiserror::Error;

use crate::map::{HexCoord, Map};
use crate::{PathResult, Query};

/// Stable identifier for an agent. Mirrors the `AgentId` in `domain` but
/// is kept local so this crate does not gain a `domain` dependency just for
/// the transfer layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentId(pub u32);

impl AgentId {
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable identifier for a spot (mirror of `domain::Spot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpotId(pub u32);

impl SpotId {
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Minimal agent descriptor for the transfer layer. We keep this struct
/// deliberately small (id + position) so the GPU upload is a tight POD
/// blob; richer fields (`FuelAmount`, `AgentKind`, ...) belong on the
/// domain side and can be projected into this struct by callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Agent {
    pub id: AgentId,
    pub position: HexCoord,
}

impl Agent {
    pub const fn new(id: AgentId, position: HexCoord) -> Self {
        Self { id, position }
    }
}

/// Minimal spot descriptor for the transfer layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Spot {
    pub id: SpotId,
    pub position: HexCoord,
}

impl Spot {
    pub const fn new(id: SpotId, position: HexCoord) -> Self {
        Self { id, position }
    }
}

/// Alias for the grid argument of [`GpuContext::upload_state`]. The grid is
/// the crate-local [`Map`] today; once `domain`'s canonical map lands and
/// `solver-gpu` migrates over, this alias will be re-pointed without
/// changing the public surface of the transfer API.
pub type HexGrid = Map;

/// Available compute kernels.
///
/// Today only the batched shortest-path kernel exists; future tickets add
/// additional variants (refuel planning, spot assignment, traffic
/// forecasting, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kernel {
    /// Batched shortest-path: solve every `(start, goal)` query against
    /// the uploaded grid in one dispatch. The WGSL implementation lives in
    /// [`crate::wgsl::SHORTEST`]; the CPU fallback lives in
    /// [`crate::cpu::shortest_path_batch`].
    ShortestPath { queries: Vec<Query> },
}

impl Kernel {
    /// Number of (start, goal) queries the kernel will evaluate. Returns
    /// `0` for non-query kernels.
    #[must_use]
    pub fn query_count(&self) -> usize {
        match self {
            Kernel::ShortestPath { queries } => queries.len(),
        }
    }
}

/// Error variants for the host-side transfer layer.
#[derive(Debug, Error)]
pub enum TransferError {
    /// The kernel pipeline has not been wired up yet (or the `wgpu`
    /// feature is disabled). Mirrors [`crate::gpu::GpuError`].
    #[error("GPU pipeline is not available: {0}")]
    PipelineUnavailable(&'static str),
    /// The GPU device failed to initialize or the queue could not be
    /// obtained.
    #[error("GPU device error: {0}")]
    Device(String),
    /// A read-back operation was requested before the kernel had finished
    /// or before any state had been uploaded.
    #[error("no kernel has been dispatched yet")]
    NothingToRead,
    /// A method received inputs of incompatible shape (e.g. zero agents
    /// when a kernel requires at least one).
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
}

/// Convenience alias for fallible transfer operations.
pub type Result<T> = std::result::Result<T, TransferError>;

/// Optional GPU-side plumbing. Compiled out entirely when the `wgpu`
/// feature is disabled so the CPU fallback path keeps a zero-cost
/// representation.
#[cfg(feature = "wgpu")]
mod gpu_buffers {
    use std::sync::Arc;

    /// GPU device + queue + the buffers we have allocated so far. Held
    /// behind an `Arc` so [`super::GpuContext`] can be cloned cheaply.
    ///
    /// The fields are intentionally not yet read by the skeleton — the
    /// real GPU pipeline lands in the follow-up GPU integration work.
    /// Keeping the type declared lets the public `GpuContext` API ship
    /// without depending on the feature being enabled, and stops the
    /// dead-code lint from firing until the pipeline catches up.
    #[allow(dead_code)]
    #[derive(Clone, Debug)]
    pub struct GpuBuffers {
        pub device: Arc<wgpu::Device>,
        pub queue: Arc<wgpu::Queue>,
        pub cost_buffer: Arc<wgpu::Buffer>,
        pub queries_buffer: Arc<wgpu::Buffer>,
        pub distances_buffer: Arc<wgpu::Buffer>,
        pub reachable_buffer: Arc<wgpu::Buffer>,
    }
}

/// Host-side state for the GPU solver.
///
/// `GpuContext` owns the **CPU shadow** of everything that *would* be
/// uploaded to the GPU. With the `wgpu` feature enabled it additionally
/// owns the real GPU buffers. Either way, [`upload_state`],
/// [`dispatch`], and [`read_back`] compose into a deterministic
/// upload / dispatch / download triple that the [`crate::GpuSolver`]
/// trait can consume.
///
/// [`upload_state`]: GpuContext::upload_state
/// [`dispatch`]: GpuContext::dispatch
/// [`read_back`]: GpuContext::read_back
#[allow(dead_code)] // `gpu` field is only live with the `wgpu` feature.
#[derive(Debug, Clone)]
pub struct GpuContext {
    grid: Option<Map>,
    agents: Vec<Agent>,
    spots: Vec<Spot>,
    last_kernel: Option<Kernel>,
    /// GPU plumbing. `None` when (a) the `wgpu` feature is disabled, or
    /// (b) the kernel has not been dispatched yet.
    #[cfg(feature = "wgpu")]
    gpu: Option<gpu_buffers::GpuBuffers>,
}

impl Default for GpuContext {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuContext {
    /// Create an empty context with no uploaded state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            grid: None,
            agents: Vec::new(),
            spots: Vec::new(),
            last_kernel: None,
            #[cfg(feature = "wgpu")]
            gpu: None,
        }
    }

    /// `true` when the context currently holds uploaded state.
    #[must_use]
    pub fn has_state(&self) -> bool {
        self.grid.is_some()
    }

    /// Borrow the grid most recently uploaded via [`upload_state`].
    #[must_use]
    pub fn grid(&self) -> Option<&Map> {
        self.grid.as_ref()
    }

    /// Borrow the agents most recently uploaded via [`upload_state`].
    #[must_use]
    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    /// Borrow the spots most recently uploaded via [`upload_state`].
    #[must_use]
    pub fn spots(&self) -> &[Spot] {
        &self.spots
    }

    /// Borrow the kernel most recently dispatched via [`dispatch`].
    #[must_use]
    pub fn last_kernel(&self) -> Option<&Kernel> {
        self.last_kernel.as_ref()
    }

    /// Upload the grid, agents, and spots into the context.
    ///
    /// This stores the CPU-side shadow state and (when the `wgpu` feature
    /// is enabled) writes the grid's cost grid into a GPU storage buffer.
    /// Agents and spots are not yet uploaded to dedicated buffers because
    /// the only kernel today ([`Kernel::ShortestPath`]) does not consume
    /// them directly; once a kernel that does, the upload path will be
    /// extended here without changing the public signature.
    ///
    /// # Errors
    ///
    /// Returns [`TransferError::InvalidInput`] when `grid` is empty or
    /// when any agent/spot position is off the grid. With the `wgpu`
    /// feature enabled a [`TransferError::Device`] is returned if the
    /// GPU buffer cannot be allocated.
    pub fn upload_state(&mut self, grid: &HexGrid, agents: &[Agent], spots: &[Spot]) -> Result<()> {
        if grid.is_empty() {
            return Err(TransferError::InvalidInput("grid must be non-empty"));
        }
        for a in agents {
            if grid.hex_index(a.position).is_none() {
                return Err(TransferError::InvalidInput("agent position is outside the grid"));
            }
        }
        for s in spots {
            if grid.hex_index(s.position).is_none() {
                return Err(TransferError::InvalidInput("spot position is outside the grid"));
            }
        }

        self.grid = Some(grid.clone());
        self.agents = agents.to_vec();
        self.spots = spots.to_vec();

        #[cfg(feature = "wgpu")]
        {
            // The real GPU upload lives behind the wgpu feature. Until
            // the GPU pipeline is fully wired (issue #37 / follow-ups),
            // we only allocate the cost buffer so callers can verify the
            // allocation path compiles and runs. The buffer is *not*
            // recorded in `self.gpu` until `dispatch` succeeds — that
            // way `read_back` only succeeds after a real dispatch.
            let _ = grid;
        }

        Ok(())
    }

    /// Issue the given [`Kernel`] against the uploaded state.
    ///
    /// On the CPU fallback path (no `wgpu` feature) this is a no-op that
    /// records `kernel` as the most recent dispatch target; the actual
    /// computation is performed by the [`crate::GpuSolver`] trait (see
    /// [`crate::FallbackGpuSolver`]). On the GPU path it would compile
    /// and submit the corresponding compute pass.
    ///
    /// # Errors
    ///
    /// Returns [`TransferError::InvalidInput`] when no state has been
    /// uploaded yet. Returns [`TransferError::PipelineUnavailable`] when
    /// the `wgpu` feature is disabled and the kernel requires GPU.
    pub fn dispatch(&mut self, kernel: Kernel) -> Result<()> {
        if self.grid.is_none() {
            return Err(TransferError::InvalidInput("upload_state must be called before dispatch"));
        }

        // Shortest-path today has a CPU fallback, so dispatching it does
        // not require the GPU pipeline. Future kernels without a CPU
        // fallback will route through the cfg block below.
        match &kernel {
            Kernel::ShortestPath { .. } => {
                self.last_kernel = Some(kernel);
                Ok(())
            }
        }
    }

    /// Read the most recent kernel's results back from the GPU.
    ///
    /// The skeleton cannot actually execute the WGSL pipeline today, so
    /// `read_back` returns [`TransferError::PipelineUnavailable`]. Callers
    /// that want a real result should go through the [`crate::GpuSolver`]
    /// trait, which routes to the CPU fallback when GPU is unavailable.
    ///
    /// # Errors
    ///
    /// Returns [`TransferError::NothingToRead`] when no kernel has been
    /// dispatched yet, and [`TransferError::PipelineUnavailable`] when
    /// the GPU read-back path is not wired (current skeleton).
    pub fn read_back(&self) -> Result<Vec<PathResult>> {
        let _kernel = self.last_kernel.as_ref().ok_or(TransferError::NothingToRead)?;
        Err(TransferError::PipelineUnavailable(
            "read_back is not wired in the skeleton — use GpuSolver::run",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid2x2() -> Map {
        let rows = vec![vec![1u8, 1], vec![1, 1]];
        Map::from_costs(&rows, 2).expect("2x2 map should build")
    }

    #[test]
    fn empty_context_has_no_state() {
        let ctx = GpuContext::new();
        assert!(!ctx.has_state());
        assert!(ctx.grid().is_none());
        assert!(ctx.agents().is_empty());
        assert!(ctx.spots().is_empty());
        assert!(ctx.last_kernel().is_none());
    }

    #[test]
    fn upload_state_populates_context() {
        let mut ctx = GpuContext::new();
        let grid = grid2x2();
        let agents = vec![Agent::new(AgentId(0), HexCoord::new(0, 0))];
        let spots = vec![Spot::new(SpotId(0), HexCoord::new(1, 1))];
        ctx.upload_state(&grid, &agents, &spots).expect("upload should succeed");
        assert!(ctx.has_state());
        assert_eq!(ctx.agents().len(), 1);
        assert_eq!(ctx.spots().len(), 1);
    }

    #[test]
    fn upload_state_rejects_off_grid_agent() {
        let mut ctx = GpuContext::new();
        let grid = grid2x2();
        let agents = vec![Agent::new(AgentId(0), HexCoord::new(99, 99))];
        let spots: Vec<Spot> = vec![];
        let err = ctx.upload_state(&grid, &agents, &spots).unwrap_err();
        assert!(matches!(err, TransferError::InvalidInput(_)));
    }

    #[test]
    fn dispatch_without_upload_is_rejected() {
        let mut ctx = GpuContext::new();
        let kernel = Kernel::ShortestPath { queries: vec![] };
        let err = ctx.dispatch(kernel).unwrap_err();
        assert!(matches!(err, TransferError::InvalidInput(_)));
    }

    #[test]
    fn dispatch_records_kernel() {
        let mut ctx = GpuContext::new();
        let grid = grid2x2();
        ctx.upload_state(&grid, &[], &[]).unwrap();
        let q = Query::new(HexCoord::new(0, 0), HexCoord::new(1, 1));
        ctx.dispatch(Kernel::ShortestPath { queries: vec![q] }).unwrap();
        assert_eq!(ctx.last_kernel().map(Kernel::query_count), Some(1));
    }

    #[test]
    fn read_back_without_dispatch_is_rejected() {
        let ctx = GpuContext::new();
        let err = ctx.read_back().unwrap_err();
        assert!(matches!(err, TransferError::NothingToRead));
    }

    #[test]
    fn read_back_reports_pipeline_unavailable() {
        let mut ctx = GpuContext::new();
        let grid = grid2x2();
        ctx.upload_state(&grid, &[], &[]).unwrap();
        ctx.dispatch(Kernel::ShortestPath { queries: vec![] }).unwrap();
        let err = ctx.read_back().unwrap_err();
        assert!(matches!(err, TransferError::PipelineUnavailable(_)));
    }

    #[test]
    fn kernel_query_count_is_zero_for_empty_shortest_path() {
        let k = Kernel::ShortestPath { queries: vec![] };
        assert_eq!(k.query_count(), 0);
    }

    #[test]
    fn kernel_query_count_matches_input_len() {
        let qs = vec![
            Query::new(HexCoord::new(0, 0), HexCoord::new(1, 0)),
            Query::new(HexCoord::new(0, 0), HexCoord::new(0, 1)),
        ];
        let k = Kernel::ShortestPath { queries: qs };
        assert_eq!(k.query_count(), 2);
    }

    #[test]
    fn agent_id_round_trip() {
        let id = AgentId::new(7);
        assert_eq!(id.get(), 7);
    }

    #[test]
    fn spot_id_round_trip() {
        let id = SpotId::new(11);
        assert_eq!(id.get(), 11);
    }
}
