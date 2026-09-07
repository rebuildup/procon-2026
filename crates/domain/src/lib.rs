//! Domain types, invariants, and pure game logic.
//!
//! `core` holds the contest's source-of-truth rules and value types. It must:
//! - be deterministic (same input -> same output) for any `core::*` function.
//! - have no I/O, no async, no clock.
//! - never depend on `engine`, `protocol`, or `cli`.
//!
//! Higher-level crates wrap these primitives in I/O and orchestration.

#![doc(test(attr(deny(warnings))))]

pub mod agent;
pub mod cell;
pub mod coordinate;
pub mod id;
pub mod outcome;
pub mod refuel;
pub mod state;
pub mod traffic;

pub use coordinate::{Bounds, HexCoord, MapLayout, MapSize};
pub use id::{MatchId, PlayerId, TurnId};
pub use state::{GameState, Outcome, Score, Turn};
pub use agent::{
    AGENT_COUNT_MAX, AGENT_COUNT_MIN, Agent, AgentId, AgentKind, Day, DaySeconds, DaySteps,
    FUEL_UPPER_BOUND_MULTIPLIER, FuelAmount, FuelPolicy, MatchState, Patrol, Position, Supply,
    Traffic,
};
pub use cell::{
    BUSY_THRESHOLD_DEFAULT, BUSY_THRESHOLD_MAX, BUSY_THRESHOLD_MIN, Cell, FUEL_COSTS,
    JAMMED_THRESHOLD_DEFAULT, JAMMED_THRESHOLD_MAX, JAMMED_THRESHOLD_MIN, MOVE_STEPS, Series, Spot,
    TrafficState, validate_traffic_thresholds,
};
pub use traffic::{Thresholds, TrafficCell, TrafficMap};

/// Library-wide error type. Pure-domain errors only; I/O errors belong to other crates.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The supplied move or action is not legal in the given state.
    #[error("illegal move: {0}")]
    IllegalMove(String),
    /// An invariant in the supplied state was violated.
    #[error("invariant violation: {0}")]
    InvariantViolation(String),
}

/// Library-wide result alias.
pub type CoreResult<T> = Result<T, CoreError>;