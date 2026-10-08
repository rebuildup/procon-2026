//! [`Solver`] trait stub and [`SeriesClusteredSolver`] implementation.
//!
//! The `Solver` trait is the seam the engine layer will call into once the
//! `domain` / `engine` integration tickets (#57, #18, #53) land. Until then it
//! is a deliberately small local stub so this crate can be exercised in
//! isolation; the trait shape is intentionally compatible with the
//! `Submitter`-style interface planned for #18.

use crate::assignment::{Assignment, AssignmentConfig, assign_clusters};
use crate::cluster::{ClusterConfig, Clustering};
use crate::types::{
    Action, AgentId, AgentPlan, HexCoord, SolveRequest, SolveResponse, Spot, SpotId,
};

/// Common seam every solver must implement.
///
/// The engine will dispatch on `&dyn Solver` and treat the returned
/// `SolveResponse` as the team's per-day submission. Inputs are owned (rather
/// than borrowed) because the engine may need to keep them alive across the
/// async dispatch boundary.
pub trait Solver {
    /// Produce a full submission for one day.
    fn solve(&self, request: SolveRequest) -> SolveResponse;
}

/// Configuration knobs for [`SeriesClusteredSolver`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SolverConfig {
    pub cluster: ClusterConfig,
    pub assignment: AssignmentConfig,
}

/// Greedy nearest-neighbour traversal.
///
/// Concrete solver for the Phase 2 strategy in `docs/contest/strategy.md`
/// §5.3:
/// 1. Cluster same-series, spatially-close spots via [`Clustering::build`].
/// 2. Assign clusters to agents via [`assign_clusters`].
/// 3. Within each cluster, walk spots in `nearest` order, emitting one
///    `Action::Move` followed by `Action::Claim` per spot.
///
/// `tie-break: variety-first` means: if two candidate next spots are at the
/// same distance, the one that gives the agent access to a series it has not
/// yet touched wins. If the series is also tied, `SpotId` ascending wins.
#[derive(Debug, Default, Clone)]
pub struct SeriesClusteredSolver {
    config: SolverConfig,
}

impl SeriesClusteredSolver {
    #[must_use]
    pub fn new(config: SolverConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn with_default_config() -> Self {
        Self::new(SolverConfig::default())
    }
}

impl Solver for SeriesClusteredSolver {
    fn solve(&self, request: SolveRequest) -> SolveResponse {
        let clustering = Clustering::build(&request.spots, self.config.cluster);
        let assignment = assign_clusters(&request, &clustering, self.config.assignment);

        let mut plans = Vec::with_capacity(assignment.plans.len());
        let mut touched_series: Vec<String> = Vec::new();

        for plan in &assignment.plans {
            let starting = request
                .agents
                .iter()
                .find(|a| a.id == plan.agent)
                .map_or(plan.starting_coord, |a| a.coord);

            let member_set: std::collections::HashSet<&SpotId> =
                plan.cluster.members.iter().collect();
            let spots: Vec<&Spot> =
                request.spots.iter().filter(|s| member_set.contains(&s.id)).collect();

            let actions = nearest_traverse(&spots, starting);
            // Dedup series into the running tracker.
            let series_label = plan.cluster.series.as_str().to_owned();
            if !touched_series.contains(&series_label) {
                touched_series.push(series_label);
            }
            plans.push(AgentPlan { agent: plan.agent.clone(), actions });
        }

        plans.sort_by(|a, b| a.agent.cmp(&b.agent));
        // The contest caps `len()` well below `u32::MAX`; the cast is safe.
        #[allow(clippy::cast_possible_truncation)]
        let variety = touched_series.len() as u32;

        SolveResponse { plans, variety }
    }
}

/// Walk `spots` in a nearest-first order starting from `start`, with
/// variety-first tie-break.
///
/// Returns one `Move` + one `Claim` per spot (in order) with no intermediate
/// `Stay`s. Pathfinding between spots is delegated to the engine layer.
fn nearest_traverse(spots: &[&Spot], start: HexCoord) -> Vec<Action> {
    let mut remaining: Vec<Spot> = spots.iter().map(|s| (*s).clone()).collect();
    let mut cursor = start;
    let mut touched_series: Vec<String> = Vec::new();
    let mut actions: Vec<Action> = Vec::with_capacity(remaining.len() * 2);

    while !remaining.is_empty() {
        // Pick the next spot: nearest, variety-first, lowest id on ties.
        // The sort key is `(distance, visited_flag, id)` so a lexicographic
        // min implements the policy directly: smaller distance wins; ties
        // go to the unvisited series (`visited_flag = 0`); final ties
        // resolve by `SpotId` ascending for determinism.
        let best_idx = remaining
            .iter()
            .enumerate()
            .min_by_key(|(_, spot)| {
                let dist = cursor.distance(spot.coord);
                let visited = touched_series.iter().any(|s| s == spot.series.as_str());
                (dist, u8::from(visited), spot.id.as_str().to_owned())
            })
            .map_or(0, |(idx, _)| idx);

        let chosen = remaining.swap_remove(best_idx);
        actions.push(Action::Move { to: chosen.coord });
        actions.push(Action::Claim { spot: chosen.id.clone(), coord: chosen.coord });
        let series_label = chosen.series.as_str().to_owned();
        if !touched_series.contains(&series_label) {
            touched_series.push(series_label);
        }
        cursor = chosen.coord;
    }

    actions
}

/// Reorder `plans` so that they read in deterministic agent-id order. Used by
/// tests and downstream consumers that want stable iterators.
#[must_use]
pub fn plans_by_agent(mut plans: Vec<AgentPlan>) -> Vec<AgentPlan> {
    plans.sort_by(|a, b| a.agent.cmp(&b.agent));
    plans
}

/// Return the count of agents that contributed a plan in `assignment`.
#[must_use]
pub fn unique_agent_count(assignment: &Assignment) -> usize {
    let mut set = std::collections::HashSet::new();
    for plan in &assignment.plans {
        set.insert(&plan.agent);
    }
    set.len()
}

/// Helper for tests: build a placeholder agent-id when only the cluster is
/// needed.
#[must_use]
pub fn synthetic_agent_id(name: &str) -> AgentId {
    AgentId::new(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Series, Spot};

    fn spot(id: &str, q: i32, r: i32, series: &str) -> Spot {
        Spot {
            id: SpotId::new(id),
            coord: HexCoord::new(q, r),
            series: Series::new(series),
            initial_stock: 1,
        }
    }

    fn agent(id: &str, q: i32, r: i32) -> crate::types::Agent {
        crate::types::Agent { id: AgentId::new(id), coord: HexCoord::new(q, r), patrol_radius: 8 }
    }

    #[test]
    fn solver_produces_one_plan_per_agent() {
        let req = SolveRequest {
            spots: vec![
                spot("a1", 0, 0, "udon"),
                spot("a2", 1, 0, "udon"),
                spot("b1", 10, 10, "soba"),
                spot("b2", 11, 10, "soba"),
            ],
            agents: vec![agent("p1", 0, 0), agent("p2", 10, 10)],
        };
        let resp = SeriesClusteredSolver::with_default_config().solve(req);
        assert_eq!(resp.plans.len(), 2);
        assert_ne!(resp.plans.len(), 0);
        // Variety = 2 distinct series touched.
        assert_eq!(resp.variety, 2);
    }

    #[test]
    fn nearest_traverse_visits_every_spot_exactly_once() {
        let s1 = spot("a", 0, 0, "u");
        let s2 = spot("b", 5, 0, "u");
        let s3 = spot("c", 10, 0, "u");
        let owned: [Spot; 3] = [s1, s2, s3];
        let refs: Vec<&Spot> = owned.iter().collect();
        let actions = nearest_traverse(&refs, HexCoord::new(1, 0));
        // Two actions per spot.
        assert_eq!(actions.len(), 6);
        // Each spot appears in exactly one `Claim`.
        let claims: Vec<&SpotId> = actions
            .iter()
            .filter_map(|a| match a {
                Action::Claim { spot, .. } => Some(spot),
                _ => None,
            })
            .collect();
        let unique: std::collections::HashSet<&SpotId> = claims.iter().copied().collect();
        assert_eq!(unique.len(), 3);
    }
}
