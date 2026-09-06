//! Authoritative game engine.
//!
//! Holds the match state machine, replays the action log, and produces
//! [`protocol::MatchSnapshot`]s for the frontend. Pure CPU work — no GPU,
//! no rendering concerns here.
//!
//! It also owns the [`solver`] contract: every `solver-*` crate implements
//! [`Solver`], so the engine can drive any strategy through `dyn Solver`
//! without depending on it.

#![doc(test(attr(deny(warnings))))]

pub mod match_table;
pub mod replay;
pub mod server;
pub mod solver;

pub use match_table::{MatchTable, MatchTableError};
pub use replay::{Replay, ReplayError};
pub use server::{EngineServer, EngineServerConfig};
pub use solver::{
    Action, ActionKind, AgentId, Direction, Solver, SolverContext, SolverDecision, TeamId,
};
