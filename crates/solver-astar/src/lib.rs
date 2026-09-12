//! `solver-astar` — single-agent A* over the hex grid.
//!
//! Spec anchors:
//!
//! - 募集要項「移動と燃料」 — base step / fuel cost per terrain.
//! - Q&A Q1 — even-row right-offset layout (encoded in [`hex::Direction::step`]).
//! - Q&A Q6 — environment reflects the **previous** step's actions before
//!   the next step's evaluation; the solver treats traffic / cell state as
//!   static within a single solve.
//! - Issue #25 — fuel-aware + traffic-aware A*; 8 reachable moves
//!   (`6 hex + Wait`); ponds and out-of-bounds cells filtered at expansion.
//!
//! The crate is intentionally self-contained: it does not depend on
//! `crates/domain` or `crates/engine` because those do not yet ship on the
//! active release branch. The local stubs in [`stub`] mirror the shapes
//! expected by issue #57's `Solver` trait and will be replaced when that
//! trait lands.

#![doc(test(attr(deny(warnings))))]

pub mod cost;
pub mod hex;
pub mod map;
pub mod stub;

mod solver;
pub mod time_expanded;

pub use cost::{BaseFuelCost, BaseStepCost, CostModel, CostPolicy, DefaultCostModel};
pub use hex::{CellKind, Direction, HexCoord, MapLayout, MapSize, TrafficState};
pub use map::{HexMap, HexMapError};
pub use solver::{AstarSolver, Solver, SolverStats};
pub use stub::{Action, Path, SolveContext, SolverFailure, SolverResult};
pub use time_expanded::{
    TimeExpandedAStar, TimeExpandedNode, TimeExpandedResult, TimeExpandedSolver,
};

/// Re-export of the crate's error type. Currently a thin alias for
/// [`HexMapError`]; expand when the solver gains more failure modes that
/// need to escape through a `Result` boundary.
pub use map::HexMapError as Error;

/// Library-wide result alias.
pub type Result<T> = ::core::result::Result<T, Error>;
