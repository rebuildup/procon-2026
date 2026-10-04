//! Greedy solver — closest-spot, variety-first tie-break.
//!
//! Reference: `docs/contest/strategy.md` §5.3 ("Phase 1（短期・予選）: 貪欲法").
//!
//! ## Algorithm
//!
//! For each agent (in order) we pick the **best available spot** using a
//! lexicographic key:
//!
//! 1. `(distance, )` — the closest unclaimed spot wins.
//! 2. **variety first tie-break**: if multiple spots are tied at the same
//!    distance, we prefer the spot whose series is currently *least*
//!    represented among the spots already claimed by *us* (this agent's
//!    team) in `ctx`. This nudges us toward a higher
//!    `distinct_series_today` count, which is the top scoring criterion.
//! 3. Tie-break again by `series_id` (stable) and finally by `spot_id` so
//!    that the decision is fully deterministic for tests / replays.
//!
//! After choosing the spot we emit `MoveToward { agent, spot }`. When an
//! agent has no fuel or no available spot exists, we emit `Stay`.
//!
//! ## Caveats / future work
//!
//! - The current implementation assumes an agent can reach any spot in one
//!   step (the stub `Action::MoveToward` is target-only). When #11/#12/#53
//!   land this loop will be replaced by a step-budget-aware planner.
//! - We do **not** claim the chosen spot in `ctx` — claim bookkeeping is
//!   the engine's job (issue #13). The solver just emits intents.
//! - Path-finding and traffic-aware routing live in #23/#24 (Phase 2 / 3).

use tracing::trace;

use crate::types::{Action, Agent, SolveContext, Spot};
use crate::{STRATEGY_NAME, Solver};

/// Greedy solver — closest-spot, variety-first tie-break.
///
/// See [module docs](self) for the algorithm.
#[derive(Debug, Clone, Default)]
pub struct GreedySolver {
    _private: (),
}

impl GreedySolver {
    /// Construct a fresh solver instance. No configuration today; reserved
    /// for future tuning knobs (weight of variety tie-break, etc.).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Choose the best spot for a single agent given the full context.
    ///
    /// Returns `None` if no spot is available (all stocks are zero) or the
    /// agent has no fuel.
    fn choose_spot<'a>(
        agent: &Agent,
        spots: &'a mut [Spot],
        available_series: &[u16],
        assigned_series: &[u16],
    ) -> Option<&'a mut Spot> {
        if !agent.can_move() {
            return None;
        }
        spots.iter_mut().filter(|s| s.is_available()).min_by_key(|s| {
            let dist = agent.coord.distance(s.coord);
            let series_count = count_occurrences(available_series, s.series_id);
            let assigned_count = count_occurrences(assigned_series, s.series_id);
            (dist, assigned_count, series_count, s.series_id, s.id.0)
        })
    }
}

impl Solver for GreedySolver {
    fn name(&self) -> &'static str {
        STRATEGY_NAME
    }

    fn decide(&self, ctx: &SolveContext) -> Vec<Action> {
        let mut available_series: Vec<u16> = Vec::with_capacity(ctx.spots.len());
        for s in &ctx.spots {
            if s.is_available() {
                available_series.push(s.series_id);
            }
        }

        let mut remaining_spots = ctx.spots.clone();
        let mut assigned_series = Vec::with_capacity(ctx.agents.len());
        ctx.agents
            .iter()
            .map(|agent| {
                let chosen = Self::choose_spot(
                    agent,
                    &mut remaining_spots,
                    &available_series,
                    &assigned_series,
                );
                if let Some(spot) = chosen {
                    spot.stock -= 1;
                    assigned_series.push(spot.series_id);
                    trace!(
                        agent = agent.id.0,
                        spot = spot.id.0,
                        series = spot.series_id,
                        "greedy: chose spot"
                    );
                    Action::MoveToward { agent: agent.id, spot: spot.id }
                } else {
                    trace!(agent = agent.id.0, "greedy: no available spot");
                    Action::Stay { agent: agent.id }
                }
            })
            .collect()
    }
}

fn count_occurrences(haystack: &[u16], needle: u16) -> u32 {
    haystack.iter().filter(|&&x| x == needle).count().try_into().unwrap_or(u32::MAX)
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
    fn empty_world_emits_stay_for_every_agent() {
        let ctx = SolveContext::for_test(0, vec![agent(0, 0, 0, 10), agent(1, 1, 0, 10)], vec![]);
        let actions = GreedySolver::new().decide(&ctx);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::Stay { .. }));
        assert!(matches!(actions[1], Action::Stay { .. }));
    }

    #[test]
    fn no_fuel_means_stay() {
        let ctx = SolveContext::for_test(0, vec![agent(0, 0, 0, 0)], vec![spot(0, 1, 0, 0, 1)]);
        let actions = GreedySolver::new().decide(&ctx);
        assert!(matches!(actions[0], Action::Stay { .. }));
    }

    #[test]
    fn closest_spot_wins_over_farther() {
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![
                spot(0, 5, 5, 0, 1), // far
                spot(1, 1, 0, 0, 1), // close
                spot(2, 2, 2, 1, 1), // medium
            ],
        );
        let actions = GreedySolver::new().decide(&ctx);
        assert_eq!(actions[0], Action::MoveToward { agent: AgentId(0), spot: SpotId(1) });
    }

    #[test]
    fn variety_first_breaks_distance_ties() {
        // agent at (0, 0). Two equally-close spots — series 0 and series 1.
        // The candidate pool already over-represents series 0 (three spots
        // vs one of series 1). The variety-first tie-break says "pick the
        // *less* represented series", so the agent must pick series 1
        // even though both candidates are at distance 1.
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![
                spot(0, 1, 0, 0, 1), // distance 1, series 0 — over-represented
                spot(1, 2, 0, 0, 1), // distance 2, series 0 — adds to over-rep
                spot(2, 3, 0, 0, 1), // distance 3, series 0 — adds to over-rep
                spot(3, 0, 1, 1, 1), // distance 1, series 1 — variety pick
            ],
        );
        let actions = GreedySolver::new().decide(&ctx);
        assert_eq!(
            actions[0],
            Action::MoveToward { agent: AgentId(0), spot: SpotId(3) },
            "expected variety-first tie-break to pick series 1"
        );
    }

    #[test]
    fn deterministic_tie_break_by_series_then_id() {
        // Both spots distance 1, both new series. Tie-break is series_id
        // ascending, then spot_id ascending.
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![
                spot(0, 1, 0, 7, 1), // distance 1, series 7
                spot(1, 0, 1, 3, 1), // distance 1, series 3
            ],
        );
        let actions = GreedySolver::new().decide(&ctx);
        assert_eq!(actions[0], Action::MoveToward { agent: AgentId(0), spot: SpotId(1) });
    }

    #[test]
    fn taken_spots_are_skipped() {
        // stock 0 means "claimed today" — must be filtered out even if it's
        // the closest.
        let ctx = SolveContext::for_test(
            0,
            vec![agent(0, 0, 0, 10)],
            vec![
                spot(0, 1, 0, 0, 0), // closest but empty
                spot(1, 2, 0, 0, 1), // second-closest, available
            ],
        );
        let actions = GreedySolver::new().decide(&ctx);
        assert_eq!(actions[0], Action::MoveToward { agent: AgentId(0), spot: SpotId(1) });
    }
}
