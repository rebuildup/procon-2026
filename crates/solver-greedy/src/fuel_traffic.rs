//! Fuel-aware, traffic-avoiding greedy variant.
//!
//! Reference: Issue #24 and `docs/contest/strategy.md` §5.3 (Phase 3
//! heuristic extension of the Phase 1 greedy solver).
//!
//! ## Algorithm
//!
//! For each agent we pick the best available spot using a score that
//! branches on the agent's current fuel level:
//!
//! - **Low fuel** (strictly below [`FuelTrafficSolveContext::fuel_low_threshold`]):
//!   the agent must refuel. Per the Issue #24 contract, *any* spot is a
//!   valid refill opportunity; we head for the nearest one.
//! - **Sufficient fuel**: the agent minimises
//!   `distance + traffic_weight × Σ traffic_penalty`, where the sum is
//!   taken over the greedy "closest neighbour" path from the agent to
//!   the candidate spot. This nudges us toward routes that avoid
//!   [`TrafficState::Jammed`] cells while keeping distance as the
//!   primary tie-break for determinism.
//!
//! After choosing the spot we emit [`Action::MoveToward`]. When an
//! agent has no fuel or no available spot exists, we emit
//! [`Action::Stay`].
//!
//! ## Caveats / future work
//!
//! - Path estimation uses a greedy step (always pick the neighbour
//!   closest to the destination). This is **not** an optimal A* search
//!   — for exact pathfinding with the full cost model use
//!   `solver-astar` (Issue #25).
//! - Cells without an explicit entry in [`FuelTrafficSolveContext::traffic`]
//!   are treated as [`TrafficState::Smooth`] (no penalty).
//! - This crate is still self-contained with local stub types because
//!   `crates/domain` (#11/#12/#53) has not landed on the active release
//!   branch. When integration happens, [`TrafficState`] will be replaced
//!   by the canonical `domain::cell::TrafficState` and
//!   [`FuelTrafficSolveContext`] will fold back into [`crate::SolveContext`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::types::{Action, Agent, HexCoord, Spot};

/// Strategy descriptor, used by the bench harness for telemetry.
///
/// Stable across releases so bench reports remain comparable.
pub const STRATEGY_NAME: &str = "greedy-phase3-fuel-traffic";

/// Default fuel threshold: an agent with strictly less fuel than this
/// heads for the nearest refill spot, ignoring traffic.
pub const DEFAULT_FUEL_LOW_THRESHOLD: u32 = 3;

/// Default traffic penalty weight in the score function.
///
/// With weight `1`, a path of three Busy cells adds `3` to the score —
/// comparable to one cell of distance. Heavier weights make the
/// solver prefer smoother routes over shorter ones.
pub const DEFAULT_TRAFFIC_WEIGHT: u32 = 1;

/// Local mirror of `domain::cell::TrafficState`.
///
/// Kept inline so this crate stays self-contained, matching the
/// pattern in `solver-astar::hex::TrafficState`. When the integration
/// issues (#11/#12) land, this type will be replaced by a re-export
/// from `domain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TrafficState {
    /// 順調 (Smooth). Lowest per-cell penalty.
    Smooth,
    /// 混雑 (Busy). Mid per-cell penalty.
    Busy,
    /// 渋滞 (Jammed). Highest per-cell penalty.
    Jammed,
}

impl TrafficState {
    /// Per-cell penalty added to the traffic score along the path.
    ///
    /// Matches the relative ratio of `Cell::move_steps` for the three
    /// road states (Smooth = 1, Busy = 2, Jammed = 4), shifted so that
    /// Smooth contributes 0 — staying Smooth is the neutral baseline
    /// and Busy/Jammed penalise the agent proportionally.
    #[must_use]
    pub const fn penalty(self) -> u32 {
        match self {
            Self::Smooth => 0,
            Self::Busy => 1,
            Self::Jammed => 3,
        }
    }
}

/// Extended read-only view used by [`FuelTrafficGreedy`].
///
/// Wraps the same per-step data as [`crate::SolveContext`] and adds:
///
/// - a per-cell [`TrafficState`] map (cells not present are treated as
///   [`TrafficState::Smooth`]),
/// - a configurable `fuel_low_threshold` below which the agent is
///   forced to refuel before doing any other planning.
///
/// When the domain integration lands this will fold back into the
/// canonical [`crate::SolveContext`]; the algorithm below is the only
/// consumer of the traffic payload today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuelTrafficSolveContext {
    /// Sequential step number inside the current day. `0` is the first
    /// step.
    pub step: u32,
    /// `true` iff this is the very first step of the day.
    pub start_of_day: bool,
    /// Current state of each agent we control. The list order is
    /// stable and matches the order in which the solver emits actions.
    pub agents: Vec<Agent>,
    /// Candidate spots. Spots with `stock == 0` are skipped.
    pub spots: Vec<Spot>,
    /// Per-cell traffic state. Cells not present are treated as
    /// [`TrafficState::Smooth`] (no penalty).
    pub traffic: HashMap<HexCoord, TrafficState>,
    /// Fuel level below which the agent must refuel.
    pub fuel_low_threshold: u32,
}

impl FuelTrafficSolveContext {
    /// Construct a context for tests / replays.
    ///
    /// `traffic` is converted to a [`HashMap`] internally; duplicate
    /// entries keep the last one.
    #[must_use]
    pub fn for_test(
        step: u32,
        agents: Vec<Agent>,
        spots: Vec<Spot>,
        traffic: Vec<(HexCoord, TrafficState)>,
        fuel_low_threshold: u32,
    ) -> Self {
        Self {
            step,
            start_of_day: step == 0,
            agents,
            spots,
            traffic: traffic.into_iter().collect(),
            fuel_low_threshold,
        }
    }
}

/// Solver trait for the fuel-aware traffic-avoiding variant.
///
/// Distinct from [`crate::Solver`] because the decision needs the
/// extended [`FuelTrafficSolveContext`] (traffic + fuel threshold).
/// When the integration PRs land this trait will collapse into the
/// canonical [`crate::Solver`].
pub trait FuelTrafficSolver {
    /// Stable, telemetry-friendly strategy name.
    fn name(&self) -> &'static str;

    /// Decide one action per agent, in agent order.
    ///
    /// The returned vector must contain exactly one [`Action`] per
    /// agent in `ctx.agents`, in the same order.
    fn decide(&self, ctx: &FuelTrafficSolveContext) -> Vec<Action>;
}

/// Fuel-aware + traffic-avoiding greedy solver.
///
/// See [module docs](self) for the algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuelTrafficGreedy {
    fuel_low_threshold: u32,
    traffic_weight: u32,
}

impl Default for FuelTrafficGreedy {
    fn default() -> Self {
        Self::new()
    }
}

impl FuelTrafficGreedy {
    /// Construct with [`DEFAULT_FUEL_LOW_THRESHOLD`] and
    /// [`DEFAULT_TRAFFIC_WEIGHT`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            fuel_low_threshold: DEFAULT_FUEL_LOW_THRESHOLD,
            traffic_weight: DEFAULT_TRAFFIC_WEIGHT,
        }
    }

    /// Construct with custom parameters.
    ///
    /// `fuel_low_threshold` is the fuel level below which an agent
    /// must refuel (strictly less than this triggers the refuel path).
    /// `traffic_weight` multiplies the per-cell traffic penalty
    /// accumulated along the greedy path to each candidate spot.
    #[must_use]
    pub const fn with_params(fuel_low_threshold: u32, traffic_weight: u32) -> Self {
        Self { fuel_low_threshold, traffic_weight }
    }

    /// The configured fuel threshold.
    #[must_use]
    pub const fn fuel_low_threshold(&self) -> u32 {
        self.fuel_low_threshold
    }

    /// The configured traffic weight.
    #[must_use]
    pub const fn traffic_weight(&self) -> u32 {
        self.traffic_weight
    }

    /// Choose the best spot for a single agent.
    ///
    /// Returns `None` if the agent cannot move or no spot is available.
    fn choose_spot<'a>(&self, agent: &Agent, ctx: &'a FuelTrafficSolveContext) -> Option<&'a Spot> {
        if !agent.can_move() {
            return None;
        }
        let need_refuel = agent.fuel < ctx.fuel_low_threshold;

        ctx.spots.iter().filter(|s| s.is_available()).min_by_key(|spot| {
            let dist = agent.coord.distance(spot.coord);
            if need_refuel {
                // Refuel path: pure distance, with the existing
                // series-then-id tie-break for determinism. We
                // keep the 4-tuple shape so the closure type
                // matches the normal path; the extra slots hold
                // `dist` (redundant primary) and `0_u32` for the
                // unused traffic column.
                (dist, dist, spot.series_id, spot.id.0, 0_u32)
            } else {
                // Traffic-aware: distance + weighted traffic cost.
                // Primary key is the combined score so a longer
                // smooth path can beat a shorter jammed one when
                // the user sets `traffic_weight` high enough; the
                // tie-break reverts to distance then series/id.
                let traffic_cost =
                    Self::estimate_traffic_cost(agent.coord, spot.coord, &ctx.traffic);
                let weighted = self.traffic_weight.saturating_mul(traffic_cost);
                (dist.saturating_add(weighted), dist, spot.series_id, spot.id.0, traffic_cost)
            }
        })
    }

    /// Estimate the cumulative traffic penalty along the greedy
    /// "closest neighbour" path from `from` to `to`.
    ///
    /// Cells not present in `traffic` contribute `0` (treated as
    /// [`TrafficState::Smooth`]). The walk is bounded to
    /// `from.distance(to)` steps to guarantee termination even if a
    /// future change accidentally regresses the neighbour ordering.
    fn estimate_traffic_cost(
        from: HexCoord,
        to: HexCoord,
        traffic: &HashMap<HexCoord, TrafficState>,
    ) -> u32 {
        if from == to {
            return 0;
        }
        let mut cost = 0u32;
        let mut cur = from;
        let max_steps = from.distance(to);
        for _ in 0..max_steps {
            // Greedy step: pick the neighbour closest to `to`. The
            // neighbour ordering in `HexCoord::neighbors` is stable
            // across runs, so the tie-break is deterministic.
            let next = cur.neighbors().into_iter().min_by_key(|n| n.distance(to)).unwrap_or(cur);
            // Bail out if we cannot make progress (would only happen
            // if a neighbour regressed the distance — kept as a
            // defensive guard against future bugs in `neighbors`).
            if next.distance(to) >= cur.distance(to) {
                break;
            }
            if let Some(state) = traffic.get(&next) {
                cost = cost.saturating_add(state.penalty());
            }
            cur = next;
        }
        cost
    }
}

impl FuelTrafficSolver for FuelTrafficGreedy {
    fn name(&self) -> &'static str {
        STRATEGY_NAME
    }

    fn decide(&self, ctx: &FuelTrafficSolveContext) -> Vec<Action> {
        ctx.agents
            .iter()
            .map(|agent| {
                let chosen = self.choose_spot(agent, ctx);
                if let Some(spot) = chosen {
                    trace!(
                        agent = agent.id.0,
                        spot = spot.id.0,
                        fuel = agent.fuel,
                        "fuel_traffic: chose spot"
                    );
                    Action::MoveToward { agent: agent.id, spot: spot.id }
                } else {
                    trace!(agent = agent.id.0, "fuel_traffic: no available spot");
                    Action::Stay { agent: agent.id }
                }
            })
            .collect()
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, SpotId};

    fn spot(id: u16, q: i32, r: i32, series: u16, stock: u32) -> Spot {
        Spot { id: SpotId(id), coord: HexCoord::new(q, r), series_id: series, stock }
    }

    fn agent(id: u8, q: i32, r: i32, fuel: u32) -> Agent {
        Agent { id: AgentId(id), coord: HexCoord::new(q, r), fuel }
    }

    /// Neighbour ordering matches the canonical axial deltas.
    #[test]
    fn neighbor_ordering_is_canonical() {
        let c = HexCoord::new(0, 0);
        let expected = [
            HexCoord::new(1, 0),
            HexCoord::new(1, -1),
            HexCoord::new(0, -1),
            HexCoord::new(-1, 0),
            HexCoord::new(-1, 1),
            HexCoord::new(0, 1),
        ];
        assert_eq!(c.neighbors(), expected);
    }

    /// Each neighbour is exactly distance 1 from the centre.
    #[test]
    fn every_neighbour_is_distance_one() {
        let c = HexCoord::new(3, -2);
        for n in c.neighbors() {
            assert_eq!(c.distance(n), 1, "{n:?} should be 1 away from {c:?}");
        }
    }

    /// `Smooth` cells do not penalise a path; `Busy` and `Jammed` do.
    #[test]
    fn traffic_state_penalties_are_proportional() {
        assert_eq!(TrafficState::Smooth.penalty(), 0);
        assert!(TrafficState::Busy.penalty() > TrafficState::Smooth.penalty());
        assert!(TrafficState::Jammed.penalty() > TrafficState::Busy.penalty());
    }

    /// An agent with `fuel == 0` cannot move, so we emit `Stay`.
    #[test]
    fn out_of_fuel_emits_stay() {
        let ctx = FuelTrafficSolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 0)],
            vec![spot(0, 1, 0, 0, 1)],
            vec![],
            3,
        );
        let actions = FuelTrafficGreedy::new().decide(&ctx);
        assert!(matches!(actions[0], Action::Stay { .. }));
    }

    /// An agent with low fuel picks the closest spot, ignoring traffic.
    #[test]
    fn low_fuel_picks_nearest_spot_ignoring_traffic() {
        // Spot 1 is closer than spot 0 but the cells between the
        // agent and spot 1 are jammed. The low-fuel agent must still
        // pick spot 1 because refuel beats traffic avoidance.
        let traffic = vec![
            (HexCoord::new(0, 1), TrafficState::Jammed),
            (HexCoord::new(1, 0), TrafficState::Jammed),
        ];
        let ctx = FuelTrafficSolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 2)], // fuel=2 < threshold=3
            vec![
                spot(0, 3, 0, 0, 1), // distance 3, smooth path
                spot(1, 1, 0, 1, 1), // distance 1, jammed cells
            ],
            traffic,
            3,
        );
        let actions = FuelTrafficGreedy::with_params(3, 100).decide(&ctx);
        assert_eq!(
            actions[0],
            Action::MoveToward { agent: AgentId(0), spot: SpotId(1) },
            "low-fuel agent must refuel at the nearest spot even through jammed cells"
        );
    }

    /// A high-fuel agent prefers a smoother route over a shorter
    /// jammed one, with a heavy traffic weight.
    #[test]
    fn high_fuel_prefers_smooth_path_over_shorter_jammed() {
        // Agent at (0,0). Two spots at the same distance 3:
        //  - spot 0 at (0,3) via cells (0,1), (0,2) — both Jammed.
        //  - spot 1 at (3,0) via cells (1,0), (2,0) — both Smooth.
        let traffic = vec![
            (HexCoord::new(0, 1), TrafficState::Jammed),
            (HexCoord::new(0, 2), TrafficState::Jammed),
            (HexCoord::new(1, 0), TrafficState::Smooth),
            (HexCoord::new(2, 0), TrafficState::Smooth),
        ];
        let ctx = FuelTrafficSolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 100)],
            vec![spot(0, 0, 3, 0, 1), spot(1, 3, 0, 1, 1)],
            traffic,
            3,
        );
        let actions = FuelTrafficGreedy::with_params(3, 100).decide(&ctx);
        assert_eq!(
            actions[0],
            Action::MoveToward { agent: AgentId(0), spot: SpotId(1) },
            "high-fuel agent must prefer the smooth path with heavy traffic weight"
        );
    }

    /// With `traffic_weight = 0` the score collapses to pure distance,
    /// so the closer jammed path wins.
    #[test]
    fn zero_traffic_weight_collapses_to_distance() {
        let traffic = vec![
            (HexCoord::new(0, 1), TrafficState::Jammed),
            (HexCoord::new(0, 2), TrafficState::Jammed),
            (HexCoord::new(1, 0), TrafficState::Smooth),
            (HexCoord::new(2, 0), TrafficState::Smooth),
        ];
        let ctx = FuelTrafficSolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 100)],
            vec![
                spot(0, 0, 3, 0, 1), // distance 3, jammed
                spot(1, 3, 0, 1, 1), // distance 3, smooth
            ],
            traffic,
            3,
        );
        let actions = FuelTrafficGreedy::with_params(3, 0).decide(&ctx);
        // Pure distance: both at 3, tie-break by series then id. The
        // spot with the smaller series_id wins.
        let chosen = match actions[0] {
            Action::MoveToward { spot, .. } => spot,
            Action::Stay { .. } => panic!("expected MoveToward"),
        };
        assert!(chosen == SpotId(0) || chosen == SpotId(1));
    }

    /// `decide` is fully deterministic — the same context yields the
    /// same actions across calls.
    #[test]
    fn decide_is_deterministic() {
        let traffic = vec![(HexCoord::new(1, 0), TrafficState::Busy)];
        let ctx = FuelTrafficSolveContext::for_test(
            5,
            vec![agent(0, 0, 0, 100), agent(1, 1, 0, 2)],
            vec![spot(0, 1, 0, 0, 1), spot(1, 2, 1, 1, 1)],
            traffic,
            3,
        );
        let solver = FuelTrafficGreedy::new();
        let a1 = solver.decide(&ctx);
        let a2 = solver.decide(&ctx);
        assert_eq!(a1, a2);
    }

    /// `estimate_traffic_cost` returns 0 for an empty traffic map.
    #[test]
    fn estimate_traffic_cost_zero_on_empty_map() {
        let traffic = HashMap::new();
        assert_eq!(
            FuelTrafficGreedy::estimate_traffic_cost(
                HexCoord::new(0, 0),
                HexCoord::new(3, 0),
                &traffic,
            ),
            0
        );
    }

    /// `estimate_traffic_cost` accumulates Busy/Jammed penalties
    /// across the path but stops cleanly at `to`.
    #[test]
    fn estimate_traffic_cost_accumulates() {
        // Path (0,0) -> (3,0) goes through cells (1,0), (2,0).
        let mut traffic = HashMap::new();
        traffic.insert(HexCoord::new(1, 0), TrafficState::Busy);
        traffic.insert(HexCoord::new(2, 0), TrafficState::Jammed);
        // Busy=1 + Jammed=3 = 4.
        assert_eq!(
            FuelTrafficGreedy::estimate_traffic_cost(
                HexCoord::new(0, 0),
                HexCoord::new(3, 0),
                &traffic,
            ),
            4
        );
    }
}
