//! Local stub types — `HexCoord`, `Spot`, `Series`, `Agent`, `Action`,
//! `SolveRequest`, `SolveResponse`.
//!
//! EXPECTED INTEGRATION: domain
//! -----------------------------
//! These types are intentionally minimal placeholders. Once the `domain` crate
//! (Issue #11) lands, every type in this module is expected to be replaced by
//! the canonical domain definition. Until then, this crate compiles and tests
//! in isolation against stable local types so the clustering / assignment
//! logic can mature ahead of integration. See Issue #57 for the integration
//! sequence.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Axial cube-style hex coordinate (q, r). The third cube axis is reconstructed
/// as `s = -q - r` so we can keep the public surface 2-tuple while still using
/// cube-distance arithmetic internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexCoord {
    pub q: i32,
    pub r: i32,
}

impl HexCoord {
    /// Construct an axial coordinate. The cube `s` axis is implicit: `s = -q - r`.
    #[must_use]
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// Number of hex steps between this coordinate and `other` on a cube grid.
    #[must_use]
    pub fn distance(self, other: Self) -> u32 {
        let dq = (self.q - other.q).abs();
        let dr = (self.r - other.r).abs();
        let ds = ((self.q + self.r) - (other.q + other.r)).abs();
        let half = (dq + dr + ds) / 2;
        half.unsigned_abs()
    }
}

impl fmt::Display for HexCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{})", self.q, self.r)
    }
}

/// Series identifier (e.g. an udon flavour family). Equal ids mean the spots
/// are part of the same series.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

/// Spot identifier — stable within a single `SolveRequest`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpotId(pub String);

impl SpotId {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SpotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A spot on the map.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Spot {
    pub id: SpotId,
    pub coord: HexCoord,
    pub series: Series,
    /// Initial inventory (number of claimable balls the spot starts with).
    pub initial_stock: u32,
}

/// Stable patrol-agent identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub String);

impl AgentId {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A patrol agent: position + per-day mobility budget (used to dimension
/// clustering, not to plan routes here).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub coord: HexCoord,
    /// Approximate per-day patrol radius in hex steps. Informational — the
    /// solver does not currently enforce it.
    pub patrol_radius: u32,
}

/// Discrete per-agent atomic action. Concrete engine payload will land with #18.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// Move to an adjacent hex. Caller is responsible for pathfinding.
    Move { to: HexCoord },
    /// Acquire one ball from the spot at `coord`.
    Claim { spot: SpotId, coord: HexCoord },
    /// Remain in place (e.g. recover fuel).
    Stay { coord: HexCoord },
}

impl Action {
    /// Hex coordinate this action ultimately resolves at (post-move for `Move`,
    /// spot coordinate for `Claim`, current for `Stay`).
    #[must_use]
    pub fn target_coord(&self) -> HexCoord {
        match *self {
            Action::Move { to } => to,
            Action::Claim { coord, .. } | Action::Stay { coord } => coord,
        }
    }
}

/// Per-agent planned itinerary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPlan {
    pub agent: AgentId,
    pub actions: Vec<Action>,
}

impl AgentPlan {
    #[must_use]
    pub fn new(agent: AgentId) -> Self {
        Self { agent, actions: Vec::new() }
    }
}

/// Solver input — the static map snapshot plus the team's agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveRequest {
    pub spots: Vec<Spot>,
    pub agents: Vec<Agent>,
}

/// Solver output — one plan per cluster, every spot reachable from an agent.
///
/// Cluster ids are opaque and stable per request; callers should treat them as
/// identifiers, not indices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveResponse {
    /// Sorted by `cluster_id` for deterministic output.
    pub plans: Vec<AgentPlan>,
    /// Number of distinct series touched across all plans. Exposed so callers
    /// can verify the variety-first heuristic is doing its job.
    pub variety: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_distance_is_symmetric_and_satisfies_triangle() {
        let a = HexCoord::new(0, 0);
        let b = HexCoord::new(3, -1);
        let c = HexCoord::new(7, -3);
        assert_eq!(a.distance(a), 0);
        assert_eq!(a.distance(b), b.distance(a));
        assert!(a.distance(c) <= a.distance(b) + b.distance(c));
    }

    #[test]
    fn action_target_coord_matches_variant() {
        let stay = Action::Stay { coord: HexCoord::new(1, 2) };
        let mv = Action::Move { to: HexCoord::new(3, 4) };
        let claim = Action::Claim { spot: SpotId::new("s"), coord: HexCoord::new(5, 6) };
        assert_eq!(stay.target_coord(), HexCoord::new(1, 2));
        assert_eq!(mv.target_coord(), HexCoord::new(3, 4));
        assert_eq!(claim.target_coord(), HexCoord::new(5, 6));
    }
}
