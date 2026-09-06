//! Domain types, invariants, and pure game logic.
//!
//! `core` holds the contest's source-of-truth rules and value types. It must:
//! - be deterministic (same input -> same output) for any `core::*` function.
//! - have no I/O, no async, no clock.
//! - never depend on `engine`, `protocol`, or `cli`.
//!
//! Higher-level crates wrap these primitives in I/O and orchestration.

#![doc(test(attr(deny(warnings))))]

pub mod id;
pub mod outcome;
pub mod state;

pub use id::{MatchId, PlayerId, TurnId};
pub use state::{GameState, Outcome, Score, Turn};

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
