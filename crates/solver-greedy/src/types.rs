//! Local stub types used while the real domain (`#11`, `#12`, `#13`, `#53`) is
//! still landing on `main`.
//!
//! EXPECTED INTEGRATION INTERFACE
//! =============================
//! When the upstream issues merge, the contents of this module will be
//! **deleted** and the crate will depend on:
//!
//! - `domain::HexCoord`     (issue #11) — axial hex coordinates.
//! - `domain::Cell`, `domain::Spot`, `domain::Series` (issue #12) — map cells,
//!   spots with stock, and udon variety series.
//! - `domain::Agent`        (issue #13) — agent state (position, fuel, role).
//! - `domain::Action`       (issue #53) — per-step agent action (`Move`,
//!   `Stay`, `Pick`).
//!
//! At that point `SolveContext` is expected to be a thin read-only view built
//! from `engine::GameState` + a `&DayPlan`-style helper, and the
//! `Solver` trait (defined in `lib.rs`) will live in `crates/bench` (issue #20).
//!
//! Until then this file lets the crate compile, link, and be exercised by
//! unit tests. **Do not** treat the shape of any item in this module as
//! stable — the names and fields below are deliberately minimal so that the
//! eventual swap is a pure rename / move.

use serde::{Deserialize, Serialize};

/// Stub axial hex coordinate. Will be replaced by `domain::HexCoord`
/// (issue #11).
///
/// The field is named to match the planned `q` / `r` convention but the
/// struct itself is intentionally not `Copy` so accidental cloning is loud
/// during the early stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HexCoord {
    /// Column (q) of the axial coordinate.
    pub q: i32,
    /// Row (r) of the axial coordinate.
    pub r: i32,
}

impl HexCoord {
    /// Construct a new coordinate.
    #[must_use]
    pub fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// Chebyshev-style distance on the hex lattice (axial hex distance).
    ///
    /// Equal to the canonical axial formula
    /// `(|q| + |q + r| + |r|) / 2`. It is symmetric and integer-valued.
    ///
    /// This will be removed once `domain::HexCoord::distance` lands.
    #[must_use]
    pub fn distance(self, other: HexCoord) -> u32 {
        let dq = (self.q - other.q).abs();
        let dr = (self.r - other.r).abs();
        let ds = (self.q + self.r - other.q - other.r).abs();
        // saturating cast: distance is non-negative and small for contest maps
        ((dq + dr + ds) / 2).try_into().unwrap_or(u32::MAX)
    }
}

/// Stub spot identifier. Will be replaced by `domain::SpotId` once #12 lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SpotId(pub u16);

impl SpotId {
    /// Construct a spot id.
    #[must_use]
    pub const fn new(v: u16) -> Self {
        Self(v)
    }
}

/// Stub agent identifier. Will be replaced by `domain::AgentId` once #13 lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AgentId(pub u8);

impl AgentId {
    /// Construct an agent id.
    #[must_use]
    pub const fn new(v: u8) -> Self {
        Self(v)
    }
}

/// Stub spot payload — coordinates, udon stock, and series variety.
///
/// Will be replaced by `domain::Spot` (#12). The `series_id` field encodes
/// which udon series the spot produces, used by the variety-first tie-break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    /// Stable identifier.
    pub id: SpotId,
    /// Cell coordinate.
    pub coord: HexCoord,
    /// Series (variety) this spot belongs to. Used for tie-break ordering.
    pub series_id: u16,
    /// Stock left. When this is `0` the spot is "taken" for the day.
    pub stock: u32,
}

impl Spot {
    /// A spot is "available" iff it still has stock for us.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        self.stock > 0
    }
}

/// Stub agent payload — coordinate and remaining fuel for the day.
///
/// Will be replaced by `domain::Agent` (#13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// Stable identifier.
    pub id: AgentId,
    /// Current cell coordinate.
    pub coord: HexCoord,
    /// Fuel remaining for the day. When `0` the agent cannot move further.
    pub fuel: u32,
}

impl Agent {
    /// Whether the agent has enough fuel to move at all.
    #[must_use]
    pub const fn can_move(&self) -> bool {
        self.fuel > 0
    }
}

/// Stub per-step action. Will be replaced by `domain::Action` (#53).
///
/// Currently models a single-agent step: move toward a target spot or stay
/// in place. Higher-level actions (pick, refill, etc.) will be added by #53.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Stay in place for one step.
    Stay {
        /// Acting agent.
        agent: AgentId,
    },
    /// Move toward the given target spot. The exact step path is computed by
    /// the engine when #11/#12/#53 merge — here the solver just emits the
    /// destination spot.
    MoveToward {
        /// Acting agent.
        agent: AgentId,
        /// Destination spot.
        spot: SpotId,
    },
}

impl Action {
    /// The agent that emits this action.
    #[must_use]
    pub fn agent(&self) -> AgentId {
        match self {
            Self::Stay { agent } | Self::MoveToward { agent, .. } => *agent,
        }
    }
}

/// Read-only view of the world as seen by a solver on a single step.
///
/// This is the stub `SolveContext` promised by issue #20. It is intentionally
/// minimal: agent states, the candidate spots, and a marker for whether the
/// step is at the start of the day (so the solver may reset its internal
/// book-keeping).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveContext {
    /// Sequential step number inside the current day. `0` is the first step.
    pub step: u32,
    /// True iff this is the very first step of the day. Solvers may use this
    /// to clear per-day caches.
    pub start_of_day: bool,
    /// Current state of each agent we control. The list order is stable and
    /// matches the order in which the solver should emit actions.
    pub agents: Vec<Agent>,
    /// Candidate spots with current stock. The solver must skip any spot
    /// whose `stock == 0` (already taken today).
    pub spots: Vec<Spot>,
}

impl SolveContext {
    /// Construct a context for tests.
    #[must_use]
    pub fn for_test(step: u32, agents: Vec<Agent>, spots: Vec<Spot>) -> Self {
        Self { step, start_of_day: step == 0, agents, spots }
    }
}
