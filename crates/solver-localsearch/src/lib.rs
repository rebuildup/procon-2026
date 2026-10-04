//! `solver-localsearch` — local-search solver for the procon-2026 contest.
//!
//! Provides a simulated-annealing driver that improves an initial route
//! using three neighbor operators (swap, 2-opt, or-opt). The crate is
//! self-contained today; it uses local stub types in [`types`] that will
//! be replaced by `crates/domain` once Issues #11, #12, #13 merge.
//!
//! # Quick start
//!
//! ```rust
//! use solver_localsearch::{
//!     Agent, HexCoord, Instance, LocalSearchConfig, LocalSearchSolver, Series, Solver, Spot,
//! };
//!
//! let instance = Instance::new(
//!     vec![
//!         Spot { id: 1, coord: HexCoord::new(0, 1), series: Series(0), remaining: 1, max_stock: 1 },
//!         Spot { id: 2, coord: HexCoord::new(0, 2), series: Series(1), remaining: 1, max_stock: 1 },
//!     ],
//!     Agent { id: 0, coord: HexCoord::new(0, 0), fuel_budget: 100, step_budget: 50 },
//! );
//! let solver = LocalSearchSolver::new(LocalSearchConfig::deterministic(200, 5.0, 0.01, 42));
//! let sol = solver.decide(&instance);
//! assert!(sol.score.distinct_series >= 1);
//! ```
//!
//! # Modules
//!
//! * [`types`] — local-stub domain types (`HexCoord`, `Spot`, `Series`,
//!   `Agent`, `Action`, `DayScore`, `Instance`, `Solution`) plus the
//!   in-crate greedy initialiser and the [`evaluate_route`] scorer.
//! * [`neighbors`] — swap / 2-opt / or-opt primitives and the
//!   [`neighbors::Rng`] wrapper.
//! * [`annealing`] — geometric temperature [`Schedule`] and the
//!   Metropolis acceptance test.
//! * [`solver`] — [`LocalSearchSolver`] (implements [`Solver`]) and its
//!   [`LocalSearchConfig`].
//! * [`simulated_annealing`] — [`SimulatedAnnealing`] driver with
//!   geometric cooling and a restart policy, suitable for problems where
//!   the simpler [`LocalSearchSolver`]'s schedule is too rigid.

#![doc(test(attr(deny(warnings))))]

pub mod annealing;
pub mod neighbors;
pub mod simulated_annealing;
pub mod solver;
pub mod types;

pub use annealing::{Delta, Schedule, accept, accept_score};
pub use neighbors::{Move, NeighborhoodRng, Segment, apply, inverse, propose};
pub use simulated_annealing::{SimulatedAnnealing, SimulatedAnnealingResult};
pub use solver::{LocalSearchConfig, LocalSearchSolver, Solver};
pub use types::{
    Action, Agent, DayScore, HexCoord, Instance, Series, Solution, Spot, evaluate_route,
    greedy_route, hex_distance,
};

/// Library-wide error type. Currently a placeholder — every public API
/// returns `Solution` directly. Once `crates/domain` lands and the
/// adapter layer is wired in, this will carry domain-level failures.
#[derive(Debug, thiserror::Error)]
pub enum SolverError {
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Library-wide result alias.
pub type SolverResult<T> = Result<T, SolverError>;

#[cfg(test)]
mod smoke_tests {
    use super::*;

    #[test]
    fn re_exports_are_consistent() {
        // A trivial compile-time check that the surface matches the docs.
        let _: fn(HexCoord, HexCoord) -> u32 = hex_distance;
        let _: fn(&[u32], &Instance) -> DayScore = evaluate_route;
        let _: fn(&Instance) -> Vec<u32> = greedy_route;
    }
}
