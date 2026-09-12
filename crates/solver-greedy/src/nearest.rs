//! Nearest-spot, multi-series greedy solver.
//!
//! Companion to the variety-first [`GreedySolver`](crate::GreedySolver).
//! Whereas the Phase 1 skeleton picks the closest spot globally and only
//! uses variety as a tie-break, this variant commits each agent to a
//! single series and only competes against spots in that series. Series
//! assignment is done round-robin over the distinct series present in the
//! candidate pool, which lets a small fleet of agents spread out across
//! many series in parallel — the key to a high
//! `distinct_series_today` score.
//!
//! ## Algorithm
//!
//! 1. Collect the distinct `series_id`s from the spots with `stock > 0`,
//!    sort ascending (so the decision is fully deterministic / replayable).
//! 2. Assign each agent to one series by index modulo the number of
//!    distinct series: `agent[i] -> series_ids[i % series_count]`. This
//!    is the "rotation across series" mentioned in issue #23.
//! 3. For each agent, scan the spots in the assigned series and pick the
//!    one with the minimum hex distance, breaking ties by `spot_id`
//!    ascending.
//! 4. When [`NearestSpotGreedy::fuel_aware`] is `true`, skip spots whose
//!    distance exceeds the agent's remaining fuel — this is the
//!    "fuel-aware variant" requested by the issue's acceptance criteria.
//! 5. Emit `MoveToward { agent, spot }` for the chosen spot, or
//!    `Stay { agent }` if no spot in the assigned series is reachable
//!    (no fuel, no available stock, or fuel-aware budget exhausted).
//!
//! ## Caveats
//!
//! - Like [`crate::GreedySolver`], we do **not** claim the chosen spot;
//!   bookkeeping is the engine's responsibility (#13).
//! - Series assignment is purely positional and resets every step. A
//!   future revision could persist a per-agent series assignment across
//!   steps via `ctx.start_of_day` + a shadow map, but for now the solver
//!   stays stateless and replay-deterministic.

use tracing::trace;

use crate::Solver;
use crate::types::{Action, Agent, SolveContext, Spot};

/// Strategy descriptor, used by the bench harness (issue #20) to label
/// reports. Stable for telemetry.
pub const STRATEGY_NAME: &str = "greedy-nearest-spot-multi-series";

/// Nearest-spot greedy with round-robin multi-series assignment.
///
/// See [module docs](self) for the full algorithm. Construct with
/// [`NearestSpotGreedy::new`] for the basic variant or
/// [`NearestSpotGreedy::fuel_aware`] for the fuel-budgeted variant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NearestSpotGreedy {
    fuel_aware: bool,
}

impl NearestSpotGreedy {
    /// Construct the basic nearest-spot / multi-series solver.
    ///
    /// The basic variant ignores remaining fuel when picking a spot — use
    /// [`NearestSpotGreedy::fuel_aware`] when you want the agent to skip
    /// spots it cannot possibly reach.
    #[must_use]
    pub const fn new() -> Self {
        Self { fuel_aware: false }
    }

    /// Construct the fuel-aware variant.
    ///
    /// With `fuel_aware = true`, an agent will only pick a spot whose hex
    /// distance is within its remaining fuel. Spots farther away are
    /// skipped and the agent emits `Stay` when no in-budget spot exists
    /// in its assigned series.
    #[must_use]
    pub const fn fuel_aware() -> Self {
        Self { fuel_aware: true }
    }

    /// Whether this solver instance filters spots by remaining fuel.
    #[must_use]
    pub const fn is_fuel_aware(&self) -> bool {
        self.fuel_aware
    }

    /// Choose the nearest available spot in `series` for `agent`.
    ///
    /// Returns `None` when no spot matches (no available stock, wrong
    /// series, or — in fuel-aware mode — no spot within the agent's
    /// remaining fuel).
    fn choose_spot<'a>(
        agent: &Agent,
        spots: &'a [Spot],
        series: u16,
        fuel_aware: bool,
    ) -> Option<&'a Spot> {
        if !agent.can_move() {
            return None;
        }
        spots
            .iter()
            .filter(|s| s.is_available() && s.series_id == series)
            .filter(|s| !fuel_aware || agent.coord.distance(s.coord) <= agent.fuel)
            .min_by_key(|s| (agent.coord.distance(s.coord), s.id.0))
    }
}

impl Solver for NearestSpotGreedy {
    fn name(&self) -> &'static str {
        STRATEGY_NAME
    }

    fn decide(&self, ctx: &SolveContext) -> Vec<Action> {
        // Step 1: collect distinct series from *all* spots, not just
        // available ones. Including empty series keeps the round-robin
        // assignment stable when one series has been exhausted (so an
        // agent does not silently flip to a different series the moment
        // its assigned series runs out).
        let mut series_ids: Vec<u16> = ctx.spots.iter().map(|s| s.series_id).collect();
        series_ids.sort_unstable();
        series_ids.dedup();

        if series_ids.is_empty() {
            // No available spots anywhere — every agent stays.
            return ctx.agents.iter().map(|a| Action::Stay { agent: a.id }).collect();
        }

        let series_count = series_ids.len();
        let fuel_aware = self.fuel_aware;

        ctx.agents
            .iter()
            .enumerate()
            .map(|(idx, agent)| {
                // Step 2: round-robin series assignment.
                let assigned = series_ids[idx % series_count];
                // Step 3-5: pick, then emit.
                let chosen = Self::choose_spot(agent, &ctx.spots, assigned, fuel_aware);
                if let Some(spot) = chosen {
                    trace!(
                        agent = agent.id.0,
                        spot = spot.id.0,
                        series = spot.series_id,
                        assigned_series = assigned,
                        "nearest: chose spot"
                    );
                    Action::MoveToward { agent: agent.id, spot: spot.id }
                } else {
                    trace!(
                        agent = agent.id.0,
                        assigned_series = assigned,
                        "nearest: no reachable spot in assigned series"
                    );
                    Action::Stay { agent: agent.id }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, HexCoord, SpotId};

    fn spot(id: u16, q: i32, r: i32, series: u16, stock: u32) -> Spot {
        Spot { id: SpotId(id), coord: HexCoord::new(q, r), series_id: series, stock }
    }

    fn agent(id: u8, q: i32, r: i32, fuel: u32) -> Agent {
        Agent { id: AgentId(id), coord: HexCoord::new(q, r), fuel }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(NearestSpotGreedy::new().name(), STRATEGY_NAME);
        assert_eq!(NearestSpotGreedy::fuel_aware().name(), STRATEGY_NAME);
    }

    #[test]
    fn empty_world_emits_stay_for_every_agent() {
        let ctx = SolveContext::for_test(0, vec![agent(0, 0, 0, 10), agent(1, 1, 0, 10)], vec![]);
        let actions = NearestSpotGreedy::new().decide(&ctx);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::Stay { .. }));
        assert!(matches!(actions[1], Action::Stay { .. }));
    }

    #[test]
    fn no_fuel_means_stay() {
        let ctx = SolveContext::for_test(0, vec![agent(0, 0, 0, 0)], vec![spot(0, 1, 0, 0, 1)]);
        let actions = NearestSpotGreedy::new().decide(&ctx);
        assert!(matches!(actions[0], Action::Stay { .. }));
    }

    #[test]
    fn single_agent_single_series_picks_nearest() {
        // One agent, all spots in series 0. Closest spot is s1 at distance 1.
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![
                spot(0, 5, 5, 0, 1), // far
                spot(1, 1, 0, 0, 1), // closest, series 0
                spot(2, 2, 2, 0, 1), // medium
            ],
        );
        let actions = NearestSpotGreedy::new().decide(&ctx);
        assert_eq!(actions[0], Action::MoveToward { agent: AgentId(0), spot: SpotId(1) });
    }

    #[test]
    fn nearest_in_series_ignores_other_series() {
        // Closer spots exist in series 1 but agent is assigned to series 0,
        // so the agent must pick the nearest *series 0* spot regardless.
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![
                spot(0, 1, 0, 1, 1), // closest overall but series 1 (ignored)
                spot(1, 5, 0, 0, 1), // series 0, far
                spot(2, 3, 0, 0, 1), // series 0, medium
            ],
        );
        let actions = NearestSpotGreedy::new().decide(&ctx);
        assert_eq!(actions[0], Action::MoveToward { agent: AgentId(0), spot: SpotId(2) });
    }

    #[test]
    fn no_available_stock_in_assigned_series_means_stay() {
        // Only series-0 spot exists but its stock is 0. The agent must
        // stay even though other series have stock available.
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![
                spot(0, 1, 0, 0, 0), // series 0 but empty
                spot(1, 0, 1, 1, 1), // series 1, available (different series)
            ],
        );
        let actions = NearestSpotGreedy::new().decide(&ctx);
        assert!(matches!(actions[0], Action::Stay { .. }));
    }

    #[test]
    fn deterministic_tie_break_by_spot_id() {
        // Two series-0 spots at the same distance. Pick the lower spot_id.
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![spot(5, 1, 0, 0, 1), spot(2, 0, 1, 0, 1)],
        );
        let actions = NearestSpotGreedy::new().decide(&ctx);
        assert_eq!(actions[0], Action::MoveToward { agent: AgentId(0), spot: SpotId(2) });
    }
}
