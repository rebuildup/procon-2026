// EXPECTED INTEGRATION: domain
//
// Local stub types for the solver-localsearch crate. They mirror the shape
// expected from `crates/domain` once Issues #11, #12, #13 land
// (`HexCoord`, `Spot`, `Series`, `Agent`, `DayScore`). The stub is kept
// intentionally minimal and `serde` is the only extra dependency; swapping
// these types for the real domain types is a name-for-name rename plus a
// coordinate-distance switch (`hex_distance` -> `domain::hex_distance`).
//
// Integration plan (post #11/#12/#13 merge):
//   * delete this file
//   * add `domain = { workspace = true }` to `Cargo.toml`
//   * `pub use domain::{HexCoord, Spot, Series, Agent, DayScore, Instance}`
//   * the solver, neighbors, and annealing modules keep their public API
//     unchanged (they take `&Instance` and return `Solution`)
//
// Until then, every API the rest of the crate exposes treats these stubs as
// opaque value types — the solver never inspects fields it does not need
// (e.g. cell terrain) because real map layout is the engine layer's job.

use serde::{Deserialize, Serialize};

/// Hex cell coordinate.
///
/// The contest problem fixes an even-row right-shift layout (Q1 / Q&A その 1),
/// so `(col, row)` is the canonical form. Distance is computed with
/// [`hex_distance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexCoord {
    pub col: i32,
    pub row: i32,
}

impl HexCoord {
    #[must_use]
    pub const fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }
}

/// Axial-coordinate hex distance. Matches the "even-r" offset layout.
///
/// `dr = row - row'`, `dc = col - col'`. With the even-row right-shift
/// convention the axial distance reduces to
/// `(dc - dr / 2).abs() + (dc - dr / 2 - dr).abs() + dr.abs()) / 2`
/// but the closed form below is the canonical
/// `(|x| + |y| + |z|) / 2` projection.
#[must_use]
#[allow(
    clippy::cast_sign_loss,
    reason = "hex distance is non-negative; the explicit abs() in the sum guarantees it"
)]
pub fn hex_distance(a: HexCoord, b: HexCoord) -> u32 {
    let ac = a.col - b.col;
    let ar = a.row - b.row;
    let bc = ac - (ar / 2);
    let br = bc - ar;
    let total = ac.unsigned_abs() + ar.unsigned_abs() + bc.unsigned_abs() + br.unsigned_abs();
    total / 2
}

/// Series identifier (a.k.a. "udon type"). Wrapped newtype so that
/// serialisation stays compact and `Ord`/`Hash` are derived automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Series(pub u16);

/// A spot on the map. `remaining` is the per-team remaining stock for the
/// current day; `max_stock` is the cap set at match start (`1..=agent_count`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Spot {
    pub id: u32,
    pub coord: HexCoord,
    pub series: Series,
    pub remaining: u16,
    pub max_stock: u16,
}

impl Spot {
    /// True when the spot still holds at least one ball for our team.
    #[must_use]
    pub const fn has_stock(&self) -> bool {
        self.remaining > 0
    }
}

/// Agent stub. The real domain type (post #13) carries `kind: AgentKind`
/// (Patrol / Supply). For route planning we only need a starting position,
/// a movement cost budget, and the per-step action budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Agent {
    pub id: u32,
    pub coord: HexCoord,
    /// Q14: `day_steps * 1..=3`. Treated as opaque budget by the solver —
    /// it never spends fuel, only plans the visit order.
    pub fuel_budget: u32,
    /// Per-day step budget (`day_steps`). Caps the maximum number of moves
    /// (including pickups) the agent can make.
    pub step_budget: u32,
}

/// Per-day outcome of a route. Mirrors the contest scoring priority
/// (戦略ノート §1): うどんの種類数 → 累積種類数 → 総玉数 → 累積回答時刻。
///
/// `cumulative_distinct` is only meaningful across multiple days; for a
/// single-day pass the solver treats it as a free tiebreaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DayScore {
    pub distinct_series: u32,
    pub cumulative_distinct: u32,
    pub total_balls: u32,
}

impl DayScore {
    /// Compose two partial scores (e.g. for multi-agent plans).
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            distinct_series: self.distinct_series + other.distinct_series,
            cumulative_distinct: self.cumulative_distinct + other.cumulative_distinct,
            total_balls: self.total_balls + other.total_balls,
        }
    }

    /// Scalar energy for SA: distinct series is weighted to dominate so that
    /// improving `distinct_series` is always strictly better than gaining
    /// balls, matching the contest ranking order.
    #[must_use]
    pub fn energy(self) -> i64 {
        i64::from(self.distinct_series) * 1_000_000
            + i64::from(self.cumulative_distinct) * 10_000
            + i64::from(self.total_balls)
    }
}

impl PartialOrd for DayScore {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DayScore {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // Contest ranking priority (higher distinct series is better).
        self.distinct_series
            .cmp(&other.distinct_series)
            .then(self.cumulative_distinct.cmp(&other.cumulative_distinct))
            .then(self.total_balls.cmp(&other.total_balls))
    }
}

/// Per-day action emission. The real `Action` enum will replace the stub
/// once #11/#13 land; for now we only encode which spot the agent visits
/// at which step (the engine turns that into a concrete movement plan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Stay,
    Move(HexCoord),
    Take { spot: u32, ball_index: u16 },
}

/// Read-only description of the optimisation problem for one agent.
///
/// The real domain provides `MatchState`/`Instance`; the stub keeps only the
/// slice the local-search solver needs (spots + the agent's own position
/// and budgets) so the algorithm stays unit-testable in isolation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    pub spots: Vec<Spot>,
    pub agent: Agent,
}

impl Instance {
    /// Convenience constructor for tests.
    #[must_use]
    pub fn new(spots: Vec<Spot>, agent: Agent) -> Self {
        Self { spots, agent }
    }

    /// Spot count, useful for empty-route guards.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spots.len()
    }

    /// True when there are no spots to visit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }
}

/// Per-agent day plan: an ordered sequence of [`Action`]s plus the score
/// achieved by evaluating it with [`evaluate_route`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Solution {
    pub actions: Vec<Action>,
    pub score: DayScore,
}

impl Solution {
    /// Empty plan (no spots visited). Score is the zero element.
    #[must_use]
    pub fn empty() -> Self {
        Self { actions: Vec::new(), score: DayScore::default() }
    }
}

/// Compute [`DayScore`] for a route expressed as spot indices into
/// `instance.spots`.
///
/// The route is interpreted as the order the agent intends to visit each
/// spot; we collect one ball per visit, capped by `spot.remaining`. This is
/// the local stub of the engine's per-day scorer and is the function the
/// neighbor / annealing modules consume to evaluate moves.
#[must_use]
pub fn evaluate_route(route: &[u32], instance: &Instance) -> DayScore {
    if route.is_empty() {
        return DayScore::default();
    }
    let mut distinct = 0u32;
    let mut total = 0u32;
    let mut seen: u32 = 0;
    // Bit-set tracker for series count (Series is u16 so 16 bits suffice).
    let mut series_mask: u32 = 0;
    for &spot_id in route {
        let Some(spot) = instance.spots.iter().find(|s| s.id == spot_id) else {
            continue;
        };
        if spot.remaining == 0 {
            continue;
        }
        let bit = 1u32.checked_shl(u32::from(spot.series.0));
        if let Some(b) = bit {
            if series_mask & b == 0 {
                distinct += 1;
                series_mask |= b;
                seen += 1;
            }
        }
        total = total.saturating_add(u32::from(spot.remaining));
    }
    let _ = seen; // reserved for future cumulative-distinct tracking
    DayScore { distinct_series: distinct, cumulative_distinct: distinct, total_balls: total }
}

/// Cheap, in-crate greedy initialiser: at each step pick the nearest spot
/// with remaining stock, breaking ties by lower series id. This is the
/// baseline the local search must match or beat.
///
/// Returns a route as a `Vec<u32>` of spot ids. The caller is expected to
/// convert it into a [`Solution`] via [`evaluate_route`] + a final
/// movement-trace expansion; we keep it as ids because the engine layer
/// is responsible for concrete step-by-step actions.
#[must_use]
pub fn greedy_route(instance: &Instance) -> Vec<u32> {
    let mut route: Vec<u32> = Vec::with_capacity(instance.spots.len());
    let mut remaining: Vec<Spot> = instance.spots.clone();
    let mut cursor = instance.agent.coord;
    let mut budget = instance.agent.step_budget;
    while let Some((idx, _)) = remaining
        .iter()
        .enumerate()
        .filter(|(_, s)| s.has_stock())
        .min_by_key(|(_, s)| (hex_distance(cursor, s.coord), s.series.0))
    {
        if budget == 0 {
            break;
        }
        let spot = remaining[idx];
        // Travel cost is approximated by hex distance; the real engine
        // accounts for terrain. We charge at least 1 step per visit.
        let cost = hex_distance(cursor, spot.coord).max(1);
        if cost > budget {
            break;
        }
        budget -= cost;
        cursor = spot.coord;
        route.push(spot.id);
        remaining[idx].remaining = 0;
    }
    route
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_distance_zero_for_same_cell() {
        let c = HexCoord::new(3, 4);
        assert_eq!(hex_distance(c, c), 0);
    }

    #[test]
    fn hex_distance_is_symmetric() {
        let a = HexCoord::new(0, 0);
        let b = HexCoord::new(3, -2);
        assert_eq!(hex_distance(a, b), hex_distance(b, a));
    }

    #[test]
    fn day_score_orders_by_distinct_series_first() {
        let lo = DayScore { distinct_series: 1, cumulative_distinct: 0, total_balls: 999 };
        let hi = DayScore { distinct_series: 2, cumulative_distinct: 0, total_balls: 0 };
        assert!(hi > lo);
    }

    #[test]
    fn day_score_breaks_ties_by_total_balls() {
        let a = DayScore { distinct_series: 2, cumulative_distinct: 2, total_balls: 3 };
        let b = DayScore { distinct_series: 2, cumulative_distinct: 2, total_balls: 5 };
        assert!(b > a);
    }

    #[test]
    fn greedy_visits_nearest_first() {
        // Two spots of different series, one closer.
        let s1 = Spot {
            id: 1,
            coord: HexCoord::new(0, 1),
            series: Series(0),
            remaining: 1,
            max_stock: 1,
        };
        let s2 = Spot {
            id: 2,
            coord: HexCoord::new(0, 5),
            series: Series(1),
            remaining: 1,
            max_stock: 1,
        };
        let agent = Agent { id: 0, coord: HexCoord::new(0, 0), fuel_budget: 100, step_budget: 10 };
        let instance = Instance::new(vec![s2, s1], agent);
        let route = greedy_route(&instance);
        assert_eq!(route, vec![1, 2]);
    }

    #[test]
    fn evaluate_route_counts_distinct_series_and_balls() {
        let s1 = Spot {
            id: 1,
            coord: HexCoord::new(0, 1),
            series: Series(0),
            remaining: 2,
            max_stock: 2,
        };
        let s2 = Spot {
            id: 2,
            coord: HexCoord::new(0, 2),
            series: Series(0),
            remaining: 1,
            max_stock: 1,
        };
        let s3 = Spot {
            id: 3,
            coord: HexCoord::new(0, 3),
            series: Series(1),
            remaining: 1,
            max_stock: 1,
        };
        let agent = Agent { id: 0, coord: HexCoord::new(0, 0), fuel_budget: 100, step_budget: 20 };
        let instance = Instance::new(vec![s1, s2, s3], agent);
        let score = evaluate_route(&[1, 2, 3], &instance);
        assert_eq!(score.distinct_series, 2);
        assert_eq!(score.total_balls, 4);
    }
}
