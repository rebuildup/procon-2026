//! Local stub types used while the real `domain` crate is still landing.
//!
//! EXPECTED INTEGRATION: domain
//! ============================
//! When the upstream domain work (issues #11, #12, #13, #53, #57) merges to
//! `main`, the contents of this module will be **deleted** and the crate will
//! depend on:
//!
//! - `domain::HexCoord`     (issue #11) — axial hex coordinates.
//! - `domain::Cell`, `domain::Spot`, `domain::Series` (issue #12) — map cells,
//!   spots with stock, and udon variety series.
//! - `domain::Agent`, `domain::AgentKind` (issue #13) — agent position, fuel,
//!   role.
//! - `domain::Action`, `domain::Direction` (issue #53) — per-step agent
//!   action (`Move`, `Stay`).
//! - `engine::solver::{Solver, SolveContext, Plan}` (issue #57) — solver
//!   trait plus per-day input/output contract.
//!
//! At that point `SolveContext` is expected to be a thin read-only view built
//! on top of `protocol::MatchSnapshot` and `MatchState` from `domain`, and
//! this crate becomes a leaf that just implements the cross-day optimisation
//! pass on top of `engine`.
//!
//! Until then, this stub module keeps the crate self-contained so the
//! single-day optimality test (and any downstream integration with the
//! `solver-greedy` skeleton) does not depend on unmerged domain work.

use serde::{Deserialize, Serialize};

/// Strategy descriptor, used by the bench harness to label reports.
pub const STRATEGY_NAME: &str = "ilp-cross-phase3-multi-day";

/// Stub hex coordinate. Will be replaced by `domain::HexCoord` (issue #11).
///
/// Axial coordinates — `q` is the column, `r` is the row offset. Distance
/// between two cells is `(q1-q2, r1-r2)`-style cube rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HexCoord {
    /// Column (axial q).
    pub q: i32,
    /// Row offset (axial r).
    pub r: i32,
}

impl HexCoord {
    /// Construct a coordinate.
    #[must_use]
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// Manhattan-style hex distance. Adequate for unit tests; the real
    /// routing cost lives in issue #11.
    ///
    /// Uses cube-coordinate distance `max(|dq|, |dr|, |dq + dr|)` which is
    /// the standard axial-hex distance metric — correct for the unit-step
    /// offsets in [`Direction::offset`].
    #[must_use]
    pub fn distance(self, other: Self) -> u32 {
        let dq = (self.q - other.q).unsigned_abs();
        let dr = (self.r - other.r).unsigned_abs();
        let ds = (self.q + self.r - (other.q + other.r)).unsigned_abs();
        dq.max(dr).max(ds)
    }
}

/// Stub direction enum. Matches the wire-code ordering used by the engine
/// (`engine::solver::Direction` from issue #57):
///
/// `0` upper-left, `1` upper-right, clockwise to `5` left.
///
/// Will be replaced by `domain::Direction` (issue #53) and re-exported from
/// `engine::solver` (issue #57).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Wire code `0`.
    UpperLeft,
    /// Wire code `1`.
    UpperRight,
    /// Wire code `2`.
    Right,
    /// Wire code `3`.
    LowerRight,
    /// Wire code `4`.
    LowerLeft,
    /// Wire code `5`.
    Left,
}

impl Direction {
    /// All directions in wire-code order. Useful for exhaustive neighbour
    /// scans in the LP model.
    pub const ALL: [Self; 6] = [
        Self::UpperLeft,
        Self::UpperRight,
        Self::Right,
        Self::LowerRight,
        Self::LowerLeft,
        Self::Left,
    ];

    /// Unit-step offset along the hex grid. Standard axial hex neighbour
    /// offsets: each direction is exactly one step in cube-hex distance.
    /// The real grid walk lives in `domain` (issue #11).
    #[must_use]
    pub const fn offset(self) -> (i32, i32) {
        match self {
            Self::UpperLeft => (0, -1),
            Self::UpperRight => (1, -1),
            Self::Right => (1, 0),
            Self::LowerRight => (0, 1),
            Self::LowerLeft => (-1, 1),
            Self::Left => (-1, 0),
        }
    }
}

/// Stable identifier for an agent (positional index within a team).
///
/// Will be replaced by `engine::solver::AgentId` (issue #57). A team fields
/// 3..=8 agents, so a byte is sufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub u8);

impl AgentId {
    /// Construct an agent id.
    #[must_use]
    pub const fn new(v: u8) -> Self {
        Self(v)
    }
}

/// Stable identifier for a spot on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpotId(pub u16);

impl SpotId {
    /// Construct a spot id.
    #[must_use]
    pub const fn new(v: u16) -> Self {
        Self(v)
    }
}

/// Stable identifier for a udon variety series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeriesId(pub u16);

impl SeriesId {
    /// Construct a series id.
    #[must_use]
    pub const fn new(v: u16) -> Self {
        Self(v)
    }
}

/// Map cell classification. Used to model "pond" cells (off-limits) and
/// "spot" cells (target cells). Plain cells are walkable and contain nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellKind {
    /// Walkable, no spot.
    Plain,
    /// Walkable, holds a spot (and possibly a udon).
    Spot(SpotId),
    /// Off-limits — an agent must never enter this cell.
    Pond,
}

/// Spot payload — coordinates, udon stock, and series variety.
///
/// Will be replaced by `domain::Spot` (issue #12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    /// Stable identifier.
    pub id: SpotId,
    /// Cell coordinate.
    pub coord: HexCoord,
    /// Series (variety) this spot belongs to.
    pub series_id: SeriesId,
    /// Stock left on day 0 — additional days may add fresh stock via the
    /// `stock_per_day` table on [`SolveContext`].
    pub initial_stock: u32,
}

impl Spot {
    /// A spot is "available" iff it still has stock for the given day.
    #[must_use]
    pub fn is_available(&self, day: u32, ctx: &SolveContext) -> bool {
        self.stock_for(day, ctx) > 0
    }

    /// Stock available on the given day. Defaults to `initial_stock`; if the
    /// context lists a per-day override, that wins.
    #[must_use]
    pub fn stock_for(&self, day: u32, ctx: &SolveContext) -> u32 {
        ctx.stock_per_day
            .iter()
            .find(|(d, sid, _)| *d == day && *sid == self.id)
            .map_or(self.initial_stock, |(_, _, q)| *q)
    }
}

/// Stub agent payload — coordinate and remaining fuel for the day.
///
/// Will be replaced by `domain::Agent` (issue #13). At that point the fuel
/// cap moves into `domain::FuelPolicy` (Patrol = `day_steps * 3` per Q14,
/// Supply = no cap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// Stable identifier.
    pub id: AgentId,
    /// Starting cell coordinate (before day 0).
    pub coord: HexCoord,
    /// Fuel remaining for the current day (relevant only to day 0).
    pub fuel: u32,
}

impl Agent {
    /// Whether the agent has enough fuel to move at all.
    #[must_use]
    pub const fn can_move(&self) -> bool {
        self.fuel > 0
    }
}

/// Stub per-step action. Will be replaced by `domain::Action` (issue #53)
/// and re-exported from `engine::solver` (issue #57).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Stay in place for one step.
    Stay {
        /// Acting agent.
        agent: AgentId,
    },
    /// Move one cell towards `direction`.
    Move {
        /// Acting agent.
        agent: AgentId,
        /// Direction to walk.
        direction: Direction,
    },
}

impl Action {
    /// The agent that emits this action.
    #[must_use]
    pub fn agent(self) -> AgentId {
        match self {
            Self::Stay { agent } | Self::Move { agent, .. } => agent,
        }
    }

    /// Whether this action consumes no movement (and therefore no fuel).
    #[must_use]
    pub const fn is_wait(self) -> bool {
        matches!(self, Self::Stay { .. })
    }
}

/// Per-step world view used by the cross-day optimiser.
///
/// Mirrors the eventual `engine::solver::SolveContext` (issue #57), but
/// embeds enough static day-by-day information (per-day step budget, fuel
/// caps, spot stock refreshes) to let the LP model reason across days.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveContext {
    /// Total days in the match (`days.len()` == `days` count).
    pub days: Vec<DaySpec>,
    /// Static set of cells in the map (with pond / spot classifications).
    pub cells: Vec<(HexCoord, CellKind)>,
    /// All spots reachable on day 0 (and after via per-day overrides).
    pub spots: Vec<Spot>,
    /// Per-day stock overrides — `(day, spot_id, quantity)`. Empty means
    /// every spot keeps its `initial_stock` on every day.
    pub stock_per_day: Vec<(u32, SpotId, u32)>,
    /// Our agents at the start of day 0. Positions / fuels for later days
    /// are derived from the previous day's plan.
    pub agents: Vec<Agent>,
}

impl SolveContext {
    /// Construct a context for tests. Convenience wrapper — production
    /// callers should assemble it via the real `engine::MatchSnapshot`.
    #[must_use]
    pub fn for_test(
        days: Vec<DaySpec>,
        cells: Vec<(HexCoord, CellKind)>,
        spots: Vec<Spot>,
        agents: Vec<Agent>,
    ) -> Self {
        Self { days, cells, spots, stock_per_day: Vec::new(), agents }
    }

    /// Number of days in this match.
    #[must_use]
    pub fn day_count(&self) -> usize {
        self.days.len()
    }

    /// Number of steps planned for the given day. Clamps to `0` if the day
    /// is out of range.
    #[must_use]
    pub fn day_steps(&self, day: u32) -> u32 {
        self.days.get(day as usize).map_or(0, |d| d.steps)
    }

    /// Fuel cap an agent may consume on the given day. Defaults to
    /// `steps * 3` per the Patrol policy in `domain::FuelPolicy` (Q14).
    #[must_use]
    pub fn fuel_cap(&self, day: u32) -> u32 {
        self.days
            .get(day as usize)
            .map_or(0, |d| d.fuel_cap_override.unwrap_or(d.steps.saturating_mul(3)))
    }
}

/// Per-day parameters. Will be replaced by `domain::MatchState::day(day)`
/// (issue #13) once the match-state types land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaySpec {
    /// Number of steps in this day (`daySteps[day]`).
    pub steps: u32,
    /// Wall-clock budget for this day (`daySeconds[day]`), used as the
    /// response-time penalty term in the objective. `None` means "unknown".
    pub seconds_budget: Option<u32>,
    /// Override for the agent fuel cap. `None` falls back to `steps * 3`.
    pub fuel_cap_override: Option<u32>,
}

impl DaySpec {
    /// A plain day spec: `steps` with the default Patrol fuel cap.
    #[must_use]
    pub const fn new(steps: u32) -> Self {
        Self { steps, seconds_budget: None, fuel_cap_override: None }
    }
}

/// Cross-day solver input bundle.
///
/// Bundles the per-day [`SolveContext`]s and the solver trait contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Step-major plan, one entry per (day, step, agent) row.
    pub actions: Vec<Action>,
    /// Free-form solver telemetry (objective value, LP status, fallback
    /// path, …).
    pub metadata: serde_json::Value,
}

impl Plan {
    /// Wrap a plan with empty metadata.
    #[must_use]
    pub fn new(actions: Vec<Action>) -> Self {
        Self { actions, metadata: serde_json::Value::Null }
    }

    /// Attach telemetry to a plan.
    #[must_use]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Whether nothing was planned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Number of planned actions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_distance_is_zero_on_same_cell() {
        let c = HexCoord::new(0, 0);
        assert_eq!(c.distance(c), 0);
    }

    #[test]
    fn hex_distance_is_one_for_adjacent_cells() {
        // Right is (+1, 0) in axial coords — distance 1.
        let c = HexCoord::new(0, 0);
        assert_eq!(c.distance(HexCoord::new(1, 0)), 1);
        assert_eq!(c.distance(HexCoord::new(-1, 0)), 1);
        assert_eq!(c.distance(HexCoord::new(1, -1)), 1);
        assert_eq!(c.distance(HexCoord::new(0, 1)), 1);
    }

    #[test]
    fn direction_offset_round_trips_wire_order() {
        // First three wire codes are the "upper" arc: UpperLeft, UpperRight,
        // Right — each is a unit axial hex step. Offsets must match the
        // engine's neighbour table (issue #57).
        assert_eq!(Direction::UpperLeft.offset(), (0, -1));
        assert_eq!(Direction::UpperRight.offset(), (1, -1));
        assert_eq!(Direction::Right.offset(), (1, 0));
    }

    #[test]
    fn spot_stock_falls_back_to_initial_when_no_override() {
        let spot = Spot {
            id: SpotId(7),
            coord: HexCoord::new(0, 0),
            series_id: SeriesId(2),
            initial_stock: 3,
        };
        let ctx =
            SolveContext::for_test(vec![DaySpec::new(10)], vec![], vec![spot.clone()], vec![]);
        assert_eq!(spot.stock_for(0, &ctx), 3);
        assert_eq!(spot.stock_for(99, &ctx), 3, "unlisted day falls back to initial");
    }

    #[test]
    fn spot_stock_override_wins_per_day() {
        let spot = Spot {
            id: SpotId(0),
            coord: HexCoord::new(0, 0),
            series_id: SeriesId(0),
            initial_stock: 2,
        };
        let ctx = SolveContext {
            days: vec![DaySpec::new(10), DaySpec::new(10)],
            cells: vec![],
            spots: vec![spot],
            stock_per_day: vec![(1, SpotId(0), 5)],
            agents: vec![],
        };
        assert_eq!(ctx.spots[0].stock_for(0, &ctx), 2, "no override on day 0");
        assert_eq!(ctx.spots[0].stock_for(1, &ctx), 5, "override applies on day 1");
    }

    #[test]
    fn fuel_cap_defaults_to_three_times_steps() {
        let ctx = SolveContext::for_test(vec![DaySpec::new(40)], vec![], vec![], vec![]);
        assert_eq!(ctx.fuel_cap(0), 120, "Patrol default: 3 * steps (Q14)");
        assert_eq!(ctx.fuel_cap(99), 0, "out-of-range day returns 0");
    }

    #[test]
    fn fuel_cap_override_wins() {
        let ctx = SolveContext::for_test(
            vec![DaySpec { steps: 40, seconds_budget: None, fuel_cap_override: Some(500) }],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(ctx.fuel_cap(0), 500);
    }
}
