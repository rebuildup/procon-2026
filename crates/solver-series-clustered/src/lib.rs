//! `solver-series-clustered` — series-clustering + greedy agent-assignment.
//!
//! Implementation of Issue #28 / Phase 2 from
//! `docs/contest/strategy.md` §5.3. The crate has three public surfaces:
//!
//! - [`cluster`]: Union-Find over same-series, spatially-close spots, with
//!   centroid + bounding-radius metadata per cluster.
//! - [`assignment`]: greedy mapping of clusters to patrol agents, breaking
//!   ties variety-first.
//! - [`solver`]: `Solver` trait stub + [`SeriesClusteredSolver`] that wires
//!   the two together and emits per-agent `Action` itineraries.
//!
//! Types under [`types`] are local stubs pending `domain` (#11) + `engine`
//! integration (#57). The `EXPECTED INTEGRATION: domain` note in `types.rs`
//! is the source of truth on the swap surface.

#![doc(test(attr(deny(warnings))))]

pub mod assignment;
pub mod cluster;
pub mod solver;
pub mod types;

pub use assignment::{Assignment, AssignmentConfig, ClusterPlan, assign_clusters};
pub use cluster::{Cluster, ClusterConfig, ClusterId, Clustering, UnionFind};
pub use solver::{SeriesClusteredSolver, Solver};
pub use types::{
    Action, Agent, AgentId, AgentPlan, HexCoord, Series, SolveRequest, SolveResponse, Spot, SpotId,
};
