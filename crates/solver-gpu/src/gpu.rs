//! GPU-side stub for batch shortest-path.
//!
//! This module is **only compiled when the `wgpu` feature is enabled** and
//! provides the bind-group / buffer plumbing the real compute pipeline
//! (see issue #37) will use. For now, [`solve_batch`] reports
//! [`GpuError::PipelineUnavailable`] so callers can detect "GPU requested
//! but not yet wired" without panicking.
//!
//! The shader source itself lives in [`crate::wgsl::SHORTEST`] and is
//! embedded at compile time.

use thiserror::Error;
use wgpu::util::DeviceExt;

use crate::map::Map;
use crate::wgsl::SHORTEST;
use crate::{PathResult, Query};

/// Error variants for GPU-side solver failures.
#[derive(Debug, Error)]
pub enum GpuError {
    /// The real compute pipeline is not implemented yet. Returned by the
    /// skeleton so callers can distinguish "device is fine, pipeline not
    /// wired" from "device does not exist".
    #[error("GPU shortest-path pipeline is not implemented yet (issue #37)")]
    PipelineUnavailable,
    /// Requested adapter / device failed to initialize.
    #[error("failed to request GPU device: {0}")]
    Device(String),
}

/// Stub GPU batch solver.
///
/// Always returns [`GpuError::PipelineUnavailable`] for now — the skeleton
/// only validates that the buffer layout compiles and the WGSL source
/// string is reachable from the runtime.
pub fn solve_batch(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    map: &Map,
    queries: &[Query],
) -> Result<Vec<PathResult>, GpuError> {
    let _ = device;
    let _ = map;
    let _ = queries;
    let _ = SHORTEST;
    Err(GpuError::PipelineUnavailable)
}

/// Helper used by the eventual pipeline; exposed so future integration
/// tickets can build buffers without re-implementing the layout.
///
/// Layout: row-major `u32` cost grid, zero-padded to `MAX_CELLS`.
#[must_use]
pub fn make_cost_buffer(device: &wgpu::Device, map: &Map) -> wgpu::Buffer {
    let mut costs = vec![0u32; crate::MAX_CELLS];
    for (i, cell) in map.iter().enumerate() {
        costs[i] = u32::from(cell.cost);
    }
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("solver-gpu::cost_buffer"),
        contents: bytemuck::cast_slice(&costs),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // No real GPU is required for the skeleton tests; these only assert that
    // the embedded shader source is present.
    #[test]
    fn shader_source_is_exposed() {
        assert!(!SHORTEST.is_empty());
    }

    #[test]
    fn make_cost_buffer_helper_compiles() {
        // This test does not allocate a real device (would require
        // wgpu::Device construction); it only checks the source layout.
        // Real device construction is exercised by issue #37.
    }

    // Compile-time check: `GpuError` exposes both variants used by callers.
    #[test]
    fn error_variants_are_distinct() {
        let a = GpuError::PipelineUnavailable;
        let b = GpuError::Device("test".into());
        assert_ne!(format!("{a:?}"), format!("{b:?}"));
    }
}
