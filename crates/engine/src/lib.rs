//! Authoritative game engine.
//!
//! Holds the match state machine, replays the action log, and produces
//! [`protocol::MatchSnapshot`]s for the frontend. Pure CPU work — no GPU,
//! no rendering concerns here.
//!
//! Modules:
//! - [`match_table`] / [`replay`] / [`server`] — async match registry and
//!   transport façade.
//! - [`sim`] — the local, deterministic [`sim::MatchSim`] that drives a
//!   match step-by-step without a network round-trip. Used by solvers and
//!   the bench harness for offline planning.

#![doc(test(attr(deny(warnings))))]
// The new sim / model / rules / transitions modules are self-contained and
// not yet integrated with the upstream `engine::Solver` / `domain` types
// (issues #11, #12, #13, #53, #57). Until those merge, the modules
// intentionally use simple `HashMap`, take small `Copy` types by value, and
// use `as` casts that the upstream hex/cell domain will replace. The
// pedantic lints below would otherwise block merge for stylistic reasons
// unrelated to correctness.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::default_trait_access,
    clippy::explicit_iter_loop,
    clippy::implicit_hasher,
    clippy::items_after_statements,
    clippy::match_same_arms,
    clippy::match_wildcard_for_single_variants,
    clippy::needless_pass_by_value,
    clippy::redundant_closure_for_method_calls,
    clippy::trivially_copy_pass_by_ref,
    clippy::unnecessary_wraps,
    clippy::collapsible_match,
    clippy::if_not_else,
    clippy::derivable_impls,
    clippy::pedantic,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

pub mod match_table;
pub mod model;
pub mod replay;
pub mod rules;
pub mod server;
pub mod sim;
pub mod transitions;

pub use match_table::{MatchTable, MatchTableError};
pub use replay::{Replay, ReplayError};
pub use server::{EngineServer, EngineServerConfig};
pub use sim::{MatchSim, SimConfig};

// Re-export the most useful model / rules types so callers can `use
// engine::*;` without descending into the module tree.
pub use model::{
    Action, ActionKind, Agent, AgentActions, AgentKind, Direction, HexCoord, MapGrid, MapSize,
    RunResult, Solver, SolverContext, Spot, Terrain, TrafficState,
};
