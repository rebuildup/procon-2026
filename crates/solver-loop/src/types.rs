//! Minimal domain stubs for the iterative-improvement loop solver.
//!
//! EXPECTED INTEGRATION: domain — once #57 lands, these stubs will be replaced by
//! the real `domain::*` types. Until then, the solver-loop crate is self-contained
//! so it can be iterated on without blocking on domain integration.
//!
//! The shapes here are deliberately narrow: only what `LoopSolver` and the
//! 2-opt / 3-opt primitives need to operate. Keep the surface tiny.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Coordinate on a 2D hex grid.
///
/// Uses axial coordinates `(q, r)` — the standard offset-free representation
/// for pointy-top or flat-top hexes. Conversion to cube / offset coordinates
/// is the caller's responsibility once the real map geometry is fixed.
///
/// EXPECTED INTEGRATION: domain — `domain::HexCoord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HexCoord {
    pub q: i32,
    pub r: i32,
}

impl HexCoord {
    #[must_use]
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// Cube-coordinate distance between two hexes.
    /// Formula: `(|dq| + |dr| + |dq+dr|) / 2`.
    #[must_use]
    pub fn distance(self, other: Self) -> u32 {
        let dq = (self.q - other.q).abs();
        let dr = (self.r - other.r).abs();
        let ds = (self.q + self.r - other.q - other.r).abs();
        // dq + dr + ds is always even, integer divide is exact.
        u32::try_from((dq + dr + ds) / 2).unwrap_or(0)
    }
}

impl fmt::Display for HexCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.q, self.r)
    }
}

/// Terrain that affects per-edge movement cost on the hex grid.
///
/// EXPECTED INTEGRATION: domain — `domain::Terrain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Terrain {
    Plain,
    Road,
    Mountain,
}

/// Identifier for a noodle *series* (うどん系列).
///
/// EXPECTED INTEGRATION: domain — `domain::SeriesId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Series(pub String);

impl Series {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Series {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A spot on the map where a noodle ball can be picked up.
///
/// EXPECTED INTEGRATION: domain — `domain::Spot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub id: u32,
    pub coord: HexCoord,
    pub series: Series,
    /// Initial stock (balls of noodles available at this spot).
    pub stock: u32,
}

/// An agent that visits spots on the map.
///
/// EXPECTED INTEGRATION: domain — `domain::Agent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Agent {
    pub id: u32,
    pub start: HexCoord,
}

/// A single action in the agent's daily plan.
///
/// EXPECTED INTEGRATION: domain — `domain::Action`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// Move to an adjacent hex (or skip step if cost is 0).
    Move { to: HexCoord },
    /// Pick up a noodle ball at the current spot.
    PickUp { spot_id: u32 },
    /// Wait at the current position.
    Wait,
}
