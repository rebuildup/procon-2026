//! Library API for `procon-cli`.
//!
//! The CLI binary (`src/main.rs`) is the primary user-facing surface, but
//! integration tests under `tests/` and future internal consumers need to
//! reach the per-match orchestration code (`coordinator_loop`,
//! `solver_adapter`, `openapi_fixture`, …) without going through the
//! `procon` subcommand parser. This crate root re-exports those modules as
//! `pub` so they are visible to external test crates.
//!
//! The binary keeps its own private copies of the same modules; the lib and
//! bin are independent crate roots that compile the same `.rs` files into
//! distinct targets. This is by design — the binary is a thin argv parser
//! around the library surface, but it does not depend on the library at
//! link time.

pub mod action_adapter;
pub mod arena;
pub mod arena_import;
pub mod candidate_bridge;
pub mod coordinator_loop;
pub mod http_log;
pub mod initial_strategy;
pub mod match_record;
pub mod monitor;
pub mod openapi_fixture;
pub mod protocol_to_engine;
pub mod solver_adapter;
pub mod tournament;
