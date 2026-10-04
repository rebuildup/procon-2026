//! Batched path-search solver.
//!
//! [`BatchedSearcher`] evaluates a batch of `(start, goal, agent_id)` queries
//! against a [`Map`] and returns one shortest path per query. It is the
//! follow-up to the skeleton established by [`crate::gpu`] (issue #34, PR #81)
//! and wires the real WGSL compute pipeline in [`wgsl::BATCHED_SEARCH`].
//!
//! ## Design
//!
//! Each workgroup processes exactly one query in workgroup-shared memory:
//!
//! ```text
//!   workgroup_size = 64 threads
//!   per-query shared state:
//!     shared_dist[1024]   per-cell tentative distance
//!     shared_pred[1024]   per-cell predecessor flat idx
//!     shared_changed[64]  per-thread "did I improve anything?" atomic flag
//! ```
//!
//! The kernel runs a synchronous wavefront (max 1024 iterations) and writes
//! the final per-query cost + per-query per-cell predecessor arrays to
//! storage buffers. The host reconstructs the actual path list from the
//! predecessor array.
//!
//! ## CPU fallback
//!
//! GPU device creation may fail in headless / CI environments. When that
//! happens — or when the `SOLVER_GPU_DISABLE` environment flag is set — the
//! searcher transparently delegates to the deterministic Dijkstra reference
//! in [`crate::cpu`]. The fallback path is exercised by the
//! `cpu_fallback_works_when_gpu_unavailable` integration test.

use thiserror::Error;
use tracing::warn;

use crate::Query;
use crate::cpu;
use crate::map::{HexCoord, Map};
use crate::wgsl::BATCHED_SEARCH;

/// Sentinel value for "no predecessor recorded yet". Mirrors the GPU kernel.
const NO_PREDECESSOR: i32 = -1;

/// Number of cells on a single map edge (32×32 grid). Mirrors `MAX_CELLS` in
/// `batched_search.wgsl`.
const CELLS_PER_QUERY: usize = 1024;

/// Per-query byte stride in the GPU queries storage buffer. The WGSL struct
/// is 20 bytes (4 × i32 + 1 × u32); we pad to 24 so the layout matches
/// `std430`'s 8-byte alignment.
const QUERY_STRIDE: usize = 24;

/// Number of threads per workgroup. Must match `@workgroup_size(64)` in
/// `batched_search.wgsl`.
pub const DEFAULT_WORKGROUP_SIZE: u32 = 64;

/// Name of the environment variable that forces the searcher onto the CPU
/// fallback path, skipping GPU device creation entirely.
pub const CPU_FALLBACK_ENV: &str = "SOLVER_GPU_DISABLE";

/// Errors that can be returned while bringing up or running the GPU pipeline.
#[derive(Debug, Error)]
pub enum BatchedSearchError {
    #[error("no compatible GPU adapter could be acquired")]
    NoAdapter,
    #[error("failed to create GPU device: {0}")]
    Device(String),
    #[error("failed to compile WGSL shader: {0}")]
    Shader(String),
    #[error("failed to create GPU buffer: {0}")]
    Buffer(String),
    #[error("failed to read GPU buffer: {0}")]
    Readback(String),
    #[error("GPU dispatch failed: {0}")]
    Dispatch(String),
}

/// Configuration for the WGSL compute pipeline. Currently only the workgroup
/// size is exposed — it must remain at 64 to keep the wavefront's
/// `cells_per_thread` ratio (1024 / 64 = 16) integer.
#[derive(Debug, Clone, Copy)]
pub struct BatchedSearchConfig {
    pub workgroup_size: u32,
}

impl Default for BatchedSearchConfig {
    fn default() -> Self {
        Self { workgroup_size: DEFAULT_WORKGROUP_SIZE }
    }
}

/// Public entry point for batched path search.
///
/// `device` / `queue` are `wgpu::Device` / `wgpu::Queue`; this module is
/// gated on the `wgpu` feature so the type is feature-gated as well — the
/// crate compiles on `wasm32` / no-GPU targets where only the CPU fallback
/// is reachable, via [`crate::cpu::shortest_path_batch_cells`].
pub struct BatchedSearcher {
    gpu: Option<GpuBackend>,
    workgroup_size: u32,
}

struct GpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl Default for BatchedSearcher {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchedSearcher {
    /// Try to construct a searcher that uses the GPU pipeline.
    ///
    /// Falls back to a CPU-only searcher when:
    ///
    /// * the `SOLVER_GPU_DISABLE` env flag is set to a non-empty value, or
    /// * GPU device creation fails for any reason.
    ///
    /// Callers that need to know whether GPU acceleration is active can use
    /// [`BatchedSearcher::is_gpu_active`].
    pub fn new() -> Self {
        Self::with_config(BatchedSearchConfig::default())
    }

    /// Variant of [`Self::new`] that exposes the workgroup-size knob.
    ///
    /// The accepted range is intentionally narrow (must be 64 today) so the
    /// shared-memory layout in `batched_search.wgsl` does not need to be
    /// regenerated at runtime. Any value other than 64 forces the CPU
    /// fallback with a warning.
    pub fn with_config(cfg: BatchedSearchConfig) -> Self {
        if cpu_forced() {
            return Self::cpu_only_inner(cfg.workgroup_size);
        }
        match try_build_gpu(cfg) {
            Ok((device, queue, pipeline, bind_group_layout)) => Self {
                gpu: Some(GpuBackend { device, queue, pipeline, bind_group_layout }),
                workgroup_size: cfg.workgroup_size,
            },
            Err(err) => {
                warn!(error = %err, "batched searcher: GPU bring-up failed, using CPU fallback");
                Self::cpu_only_inner(cfg.workgroup_size)
            }
        }
    }

    /// Construct a searcher that always routes through the CPU fallback.
    /// No adapter or device is required for this backend.
    pub fn cpu_only() -> Self {
        Self::cpu_only_inner(DEFAULT_WORKGROUP_SIZE)
    }

    fn cpu_only_inner(workgroup_size: u32) -> Self {
        Self { gpu: None, workgroup_size }
    }

    /// `true` when the GPU pipeline is wired and will be used by
    /// [`Self::search_batch`]. Always `false` when the searcher was built
    /// via [`Self::cpu_only`] or when `SOLVER_GPU_DISABLE` is set.
    #[allow(clippy::unused_self)]
    pub fn is_gpu_active(&self) -> bool {
        self.gpu.is_some() && !cpu_forced()
    }

    /// Public workgroup size used to size the dispatch.
    #[must_use]
    pub fn workgroup_size(&self) -> u32 {
        self.workgroup_size
    }

    /// Run a batch of `(start, goal, agent_id)` queries against `map` and
    /// return the shortest path for each query, in the same order as
    /// `queries`. Empty vectors are returned for queries whose goal is
    /// unreachable from the start (or whose endpoints lie off-map).
    pub fn search_batch(&self, map: &Map, queries: &[Query]) -> Vec<Vec<HexCoord>> {
        if !self.is_gpu_active() || queries.is_empty() {
            return cpu::shortest_path_batch_cells(map, queries);
        }
        match dispatch_gpu(self.gpu.as_ref().expect("active GPU backend"), map, queries) {
            Ok(paths) => paths,
            Err(err) => {
                warn!(error = %err, "batched searcher: GPU dispatch failed, falling back to CPU");
                cpu::shortest_path_batch_cells(map, queries)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GPU bring-up
// ---------------------------------------------------------------------------

fn try_build_gpu(
    cfg: BatchedSearchConfig,
) -> Result<
    (wgpu::Device, wgpu::Queue, wgpu::ComputePipeline, wgpu::BindGroupLayout),
    BatchedSearchError,
> {
    if cfg.workgroup_size != DEFAULT_WORKGROUP_SIZE {
        return Err(BatchedSearchError::Shader(format!(
            "workgroup_size must be {DEFAULT_WORKGROUP_SIZE} (got {})",
            cfg.workgroup_size
        )));
    }

    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        force_fallback_adapter: true, // CI / wasm32 fallback paths must succeed.
        compatible_surface: None,
    }))
    .ok_or(BatchedSearchError::NoAdapter)?;

    let (device, queue) = block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("solver-gpu::batched"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::default(),
        },
        None,
    ))
    .map_err(|e| BatchedSearchError::Device(e.to_string()))?;

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("solver-gpu::batched_search"),
        source: wgpu::ShaderSource::Wgsl(BATCHED_SEARCH.into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("solver-gpu::batched_search.bgl"),
        entries: &[
            map_bind_entry(),
            queries_bind_entry(),
            distances_bind_entry(),
            predecessors_bind_entry(),
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("solver-gpu::batched_search.pl"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("solver-gpu::batched_search.pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    Ok((device, queue, pipeline, bind_group_layout))
}

fn map_bind_entry() -> wgpu::BindGroupLayoutEntry {
    // Storage (read-only) rather than Uniform so the per-cell cost array does
    // not need to be repacked to vec4<u32> to satisfy naga's 16-byte
    // alignment requirement for uniform buffers.
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn queries_bind_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn distances_bind_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn predecessors_bind_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 3,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

// ---------------------------------------------------------------------------
// GPU dispatch
// ---------------------------------------------------------------------------

fn dispatch_gpu(
    searcher: &GpuBackend,
    map: &Map,
    queries: &[Query],
) -> Result<Vec<Vec<HexCoord>>, BatchedSearchError> {
    use wgpu::util::DeviceExt;

    let q_count = queries.len();
    if q_count == 0 {
        return Ok(Vec::new());
    }

    // ---- Pack the map uniform buffer ----
    // Layout: rows, cols, cell_count, _pad, costs[1024].
    let mut map_bytes = vec![0u8; 16 + CELLS_PER_QUERY * 4];
    map_bytes[0..4].copy_from_slice(&map.size.rows.to_le_bytes());
    map_bytes[4..8].copy_from_slice(&map.size.cols.to_le_bytes());
    let cell_count = u32::try_from(map.len()).expect("map size fits in u32");
    map_bytes[8..12].copy_from_slice(&cell_count.to_le_bytes());
    // bytes 12..16 already zero (pad)
    for (i, cell) in map.iter().take(CELLS_PER_QUERY).enumerate() {
        let cost = u32::from(cell.cost);
        let off = 16 + i * 4;
        map_bytes[off..off + 4].copy_from_slice(&cost.to_le_bytes());
    }
    let map_buf = searcher.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("solver-gpu::batched.map"),
        contents: &map_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    // ---- Pack queries storage buffer ----
    // WGSL: { start_q, start_r, goal_q, goal_r : i32, agent_id : u32 } = 20 bytes.
    // We append 4 bytes of trailing zero-padding per query so each entry
    // sits on an 8-byte boundary, matching the std430-style layout that
    // wgpu's automatic alignment tracking expects.
    let mut query_bytes = vec![0u8; q_count * QUERY_STRIDE];
    for (i, q) in queries.iter().enumerate() {
        let agent_id = u32::try_from(i).expect("batch size fits in u32");
        write_query(
            &mut query_bytes[i * QUERY_STRIDE..],
            q.start.q,
            q.start.r,
            q.goal.q,
            q.goal.r,
            agent_id,
        );
    }
    let queries_buf = searcher.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("solver-gpu::batched.queries"),
        contents: &query_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    // ---- Allocate output buffers ----
    let distances_size = (q_count * std::mem::size_of::<u32>()) as u64;
    let distances_buf = searcher.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("solver-gpu::batched.distances"),
        size: distances_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let predecessors_size = (q_count * CELLS_PER_QUERY * std::mem::size_of::<i32>()) as u64;
    let predecessors_buf = searcher.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("solver-gpu::batched.predecessors"),
        size: predecessors_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    // ---- Staging buffers for readback ----
    let distances_stage = searcher.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("solver-gpu::batched.distances.stage"),
        size: distances_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let predecessors_stage = searcher.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("solver-gpu::batched.predecessors.stage"),
        size: predecessors_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // ---- Bind group + dispatch ----
    let bind_group = searcher.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("solver-gpu::batched.bg"),
        layout: &searcher.bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: map_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: queries_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: distances_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: predecessors_buf.as_entire_binding() },
        ],
    });

    let mut encoder = searcher.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("solver-gpu::batched.encoder"),
    });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("solver-gpu::batched.cpass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&searcher.pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        let dispatch_x = u32::try_from(q_count).expect("batch size fits in u32");
        cpass.dispatch_workgroups(dispatch_x, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&distances_buf, 0, &distances_stage, 0, distances_size);
    encoder.copy_buffer_to_buffer(&predecessors_buf, 0, &predecessors_stage, 0, predecessors_size);

    let _submit_index = searcher.queue.submit(std::iter::once(encoder.finish()));
    let distances_slice = distances_stage.slice(..);
    let predecessors_slice = predecessors_stage.slice(..);
    let distances_map = poll_buffer_map(&searcher.device, &distances_slice)?;
    let predecessors_map = poll_buffer_map(&searcher.device, &predecessors_slice)?;
    Ok(reconstruct_paths(map, queries, &distances_map, &predecessors_map))
}

fn write_query(
    out: &mut [u8],
    start_q: i32,
    start_r: i32,
    goal_q: i32,
    goal_r: i32,
    agent_id: u32,
) {
    debug_assert!(out.len() >= QUERY_STRIDE, "query slice must be at least QUERY_STRIDE bytes");
    out[0..4].copy_from_slice(&start_q.to_le_bytes());
    out[4..8].copy_from_slice(&start_r.to_le_bytes());
    out[8..12].copy_from_slice(&goal_q.to_le_bytes());
    out[12..16].copy_from_slice(&goal_r.to_le_bytes());
    out[16..20].copy_from_slice(&agent_id.to_le_bytes());
    // bytes 20..QUERY_STRIDE are zero (pad).
}

fn poll_buffer_map(
    device: &wgpu::Device,
    slice: &wgpu::BufferSlice<'_>,
) -> Result<Vec<u8>, BatchedSearchError> {
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|e| BatchedSearchError::Readback(e.to_string()))?
        .map_err(|e| BatchedSearchError::Readback(e.to_string()))?;
    Ok(slice.get_mapped_range().to_vec())
}

fn reconstruct_paths(
    map: &Map,
    queries: &[Query],
    distances_bytes: &[u8],
    predecessors_bytes: &[u8],
) -> Vec<Vec<HexCoord>> {
    let q_count = queries.len();
    let mut distances = vec![0u32; q_count];
    for (i, slot) in distances.iter_mut().enumerate() {
        let off = i * 4;
        *slot = u32::from_le_bytes(distances_bytes[off..off + 4].try_into().unwrap_or([0; 4]));
    }

    let mut predecessors = vec![NO_PREDECESSOR; q_count * CELLS_PER_QUERY];
    for (i, slot) in predecessors.iter_mut().enumerate() {
        let off = i * 4;
        *slot = i32::from_le_bytes(predecessors_bytes[off..off + 4].try_into().unwrap_or([0; 4]));
    }

    queries
        .iter()
        .enumerate()
        .map(|(qi, q)| {
            let final_cost = distances[qi];
            if final_cost == u32::MAX {
                return Vec::new();
            }
            reconstruct_one_path(map, q.start, q.goal, &predecessors[qi * CELLS_PER_QUERY..])
        })
        .collect()
}

#[allow(clippy::cast_sign_loss)]
fn reconstruct_one_path(map: &Map, start: HexCoord, goal: HexCoord, pred: &[i32]) -> Vec<HexCoord> {
    let Some(start_idx) = map.hex_index(start) else {
        return Vec::new();
    };
    let Some(goal_idx) = map.hex_index(goal) else {
        return Vec::new();
    };
    if pred.get(goal_idx).copied().unwrap_or(NO_PREDECESSOR) == NO_PREDECESSOR {
        return Vec::new();
    }
    if start_idx == goal_idx {
        return vec![start];
    }
    let mut path = Vec::new();
    let mut cur = goal_idx;
    let mut guard = 0usize;
    let limit = pred.len() + 1;
    while cur != start_idx {
        path.push(flat_to_axial(cur, map));
        let prev = pred[cur];
        if prev < 0 {
            return Vec::new();
        }
        let prev_u = prev as usize;
        if prev_u == cur {
            return Vec::new();
        }
        cur = prev_u;
        guard += 1;
        if guard > limit {
            return Vec::new();
        }
    }
    path.push(flat_to_axial(start_idx, map));
    path.reverse();
    path
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
fn flat_to_axial(idx: usize, map: &Map) -> HexCoord {
    let cols = usize::from(map.size.cols);
    let row = i32::try_from(idx / cols).expect("map row index fits in i32 (rows <= 32)");
    let col = i32::try_from(idx % cols).expect("map col index fits in i32 (cols <= 32)");
    let parity = (row as u32) & 1;
    let q = col.wrapping_sub((parity as i32).wrapping_mul(row / 2));
    HexCoord::new(q, row)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `true` when the `SOLVER_GPU_DISABLE` env flag is set to any non-empty value.
fn cpu_forced() -> bool {
    std::env::var(CPU_FALLBACK_ENV)
        .is_ok_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
}

/// Minimal "block on a future until it resolves" helper.
///
/// We do not pull in `pollster` / `tokio` / `futures` to keep the
/// optional-dependency surface narrow — wgpu's adapter / device requests are
/// effectively synchronous once the instance is initialized, so a no-op
/// waker + thread-yield loop is sufficient.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    let waker = std::task::Waker::noop();
    let mut ctx = Context::from_waker(waker);
    let mut fut: Pin<Box<F>> = Box::pin(fut);
    loop {
        match fut.as_mut().poll(&mut ctx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Sanity tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_only_and_failed_bringup_never_require_an_adapter() {
        let map = Map::from_costs(&[vec![1, 1, 1]], 3).unwrap();
        let queries = [Query::new(HexCoord::new(0, 0), HexCoord::new(2, 0))];
        let expected = cpu::shortest_path_batch_cells(&map, &queries);
        for searcher in [
            BatchedSearcher::cpu_only(),
            BatchedSearcher::with_config(BatchedSearchConfig { workgroup_size: 0 }),
        ] {
            assert!(!searcher.is_gpu_active());
            assert_eq!(searcher.search_batch(&map, &queries), expected);
        }
    }

    #[test]
    fn cpu_forced_only_when_env_set() {
        // We mutate process-wide env here. Pin the unsafe to this single
        // test by scoping it with explicit restore at the end. The runtime
        // is single-threaded for `#[test]` setup.
        #[allow(unsafe_code)]
        fn set(v: Option<&str>) {
            // SAFETY: see test-level doc comment.
            unsafe {
                match v {
                    Some(s) => std::env::set_var(CPU_FALLBACK_ENV, s),
                    None => std::env::remove_var(CPU_FALLBACK_ENV),
                }
            }
        }

        let prev = std::env::var(CPU_FALLBACK_ENV).ok();
        set(None);
        assert!(!cpu_forced(), "must be false when env is unset");
        set(Some("1"));
        assert!(cpu_forced(), "must be true when env=1");
        set(Some("false"));
        assert!(!cpu_forced(), "must be false when env=false");
        set(prev.as_deref());
    }

    #[test]
    fn default_workgroup_size_matches_wgsl() {
        // The shader hard-codes `@workgroup_size(64)`. If this drifts, the
        // wavefront's `cells_per_thread = 1024 / 64` invariant breaks.
        assert_eq!(DEFAULT_WORKGROUP_SIZE, 64);
        assert!(BATCHED_SEARCH.contains("@workgroup_size(64)"));
    }

    #[test]
    fn query_stride_is_multiple_of_8() {
        // The WGSL struct is 20 bytes (4 × i32 + 1 × u32) and we pad to 24
        // so it sits on an 8-byte boundary for wgpu's std430-style storage
        // alignment. Anything else risks subtle miscompiles on some drivers.
        const { assert!(QUERY_STRIDE >= 20) };
        assert_eq!(QUERY_STRIDE % 8, 0);
    }
}
