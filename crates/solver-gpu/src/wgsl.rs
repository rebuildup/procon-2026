//! Embedded WGSL compute shaders.
//!
//! The shader text is included at compile time via [`include_str!`] so the
//! final binary carries it as a `&'static str`. Validation against the
//! `naga` validator happens in the GPU integration work (follow-up ticket
//! #37); for the skeleton we only check that the file is non-empty and
//! round-trips through `include_str!`.

/// Embedded source of `shaders/shortest.wgsl`.
pub const SHORTEST: &str = include_str!("shaders/shortest.wgsl");

/// Embedded source of `shaders/batched_search.wgsl` — the workgroup-shared
/// parallel BFS kernel used by [`crate::batched::BatchedSearcher`].
pub const BATCHED_SEARCH: &str = include_str!("shaders/batched_search.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_source_is_non_empty() {
        assert!(!SHORTEST.is_empty(), "embedded WGSL must not be empty");
        assert!(!BATCHED_SEARCH.is_empty(), "embedded batched_search.wgsl must not be empty");
    }

    #[test]
    fn shader_contains_compute_entry_point() {
        assert!(SHORTEST.contains("@compute"), "shortest.wgsl must declare a @compute entry point");
        assert!(
            BATCHED_SEARCH.contains("@compute"),
            "batched_search.wgsl must declare a @compute entry point"
        );
    }

    #[test]
    fn shader_references_required_bindings() {
        for needle in ["@group(0) @binding(0)", "@group(0) @binding(1)"] {
            assert!(SHORTEST.contains(needle), "shortest.wgsl must declare binding {needle}");
        }
        // batched_search.wgsl uses binding 0 (map uniform) + 1 (queries) + 2/3 (results)
        for needle in [
            "@group(0) @binding(0)",
            "@group(0) @binding(1)",
            "@group(0) @binding(2)",
            "@group(0) @binding(3)",
        ] {
            assert!(
                BATCHED_SEARCH.contains(needle),
                "batched_search.wgsl must declare binding {needle}"
            );
        }
    }

    #[test]
    fn batched_shader_uses_workgroup_shared_memory() {
        // The kernel is required to use workgroup-shared memory so multiple
        // threads can cooperate on a single BFS frontier.
        assert!(
            BATCHED_SEARCH.contains("var<workgroup>"),
            "batched_search.wgsl must declare at least one var<workgroup> buffer"
        );
    }
}
