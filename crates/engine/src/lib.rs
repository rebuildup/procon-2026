//! Authoritative game engine.
//!
//! Holds the match state machine, replays the action log, and produces
//! [`protocol::MatchSnapshot`]s for the frontend. Pure CPU work — no GPU,
//! no rendering concerns here.

#![doc(test(attr(deny(warnings))))]

pub mod match_table;
pub mod replay;
pub mod server;

pub use match_table::{MatchTable, MatchTableError};
pub use replay::{Replay, ReplayError};
pub use server::{EngineServer, EngineServerConfig};
