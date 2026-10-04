//! Greedy assignment of clusters to patrol agents.
//!
//! See `docs/contest/strategy.md` §5.3. The goal in Phase 2 is simple:
//! the largest cluster (most variety-rich, largest patrol budget requirement)
//! is assigned to the most-available agent first; we then iterate.
//!
//! Ties are broken by *variety-first*: when two clusters have the same number
//! of spots, the one whose series is rarer in the map wins (we want every
//! series represented before we double up on a single flavour).
//!
//! Surplus clusters (more clusters than agents) are split onto the same
//! agent in deterministic order so every spot is still claimed.

use crate::cluster::{Cluster, Clustering};
use crate::types::{Agent, AgentId, HexCoord, Series, SolveRequest, Spot, SpotId};
use std::collections::HashMap;

/// For every spot in the request, lists the cluster it belongs to and the
/// agent that is responsible for the patrol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// Sorted by `ClusterId` for deterministic output.
    pub plans: Vec<ClusterPlan>,
    /// Spots sorted by id for easy lookup.
    pub spots_to_agent: HashMap<crate::types::SpotId, AgentId>,
}

/// One cluster committed to one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterPlan {
    pub cluster: Cluster,
    pub agent: AgentId,
    pub starting_coord: HexCoord,
}

/// Strategy parameters for [`assign_clusters`].
#[derive(Debug, Clone, Copy)]
pub struct AssignmentConfig {
    /// Maximum number of clusters a single agent will be responsible for
    /// when `clusters.len() > agents.len()`. Defaults to `2` so we never
    /// overload a single agent with the whole map.
    pub max_clusters_per_agent: u32,
}

impl Default for AssignmentConfig {
    fn default() -> Self {
        Self { max_clusters_per_agent: 2 }
    }
}

/// Greedy assignment of `clustering.clusters` to the agents in `request.agents`.
///
/// The sort key is `(members.len().desc(), series_rarity.desc(), cluster_id)`
/// so a bigger cluster wins ties; rarer series win over more common ones; and
/// cluster ids keep the result stable.
#[must_use]
pub fn assign_clusters(
    request: &SolveRequest,
    clustering: &Clustering,
    config: AssignmentConfig,
) -> Assignment {
    if clustering.clusters.is_empty() || request.agents.is_empty() {
        return Assignment { plans: Vec::new(), spots_to_agent: HashMap::new() };
    }

    let mut series_popularity: HashMap<&str, usize> = HashMap::new();
    for cluster in &clustering.clusters {
        *series_popularity.entry(cluster.series.as_str()).or_insert(0) += 1;
    }

    let mut ordered_clusters: Vec<&Cluster> = clustering.clusters.iter().collect();
    ordered_clusters.sort_by(|a, b| {
        let size = b.members.len().cmp(&a.members.len());
        if size != std::cmp::Ordering::Equal {
            return size;
        }
        let pop_a = series_popularity.get(a.series.as_str()).copied().unwrap_or(0);
        let pop_b = series_popularity.get(b.series.as_str()).copied().unwrap_or(0);
        let rarity = pop_a.cmp(&pop_b);
        if rarity != std::cmp::Ordering::Equal {
            return rarity;
        }
        a.id.cmp(&b.id)
    });

    let mut agent_loads: HashMap<AgentId, u32> =
        request.agents.iter().map(|a| (a.id.clone(), 0)).collect();

    let plans = greedy_assign(&ordered_clusters, &request.agents, config, &mut agent_loads);

    let mut spots_to_agent: HashMap<crate::types::SpotId, AgentId> = HashMap::new();
    for plan in &plans {
        for spot_id in &plan.cluster.members {
            spots_to_agent.insert(spot_id.clone(), plan.agent.clone());
        }
    }

    Assignment { plans, spots_to_agent }
}

fn greedy_assign(
    ordered_clusters: &[&Cluster],
    agents: &[Agent],
    config: AssignmentConfig,
    agent_loads: &mut HashMap<AgentId, u32>,
) -> Vec<ClusterPlan> {
    let mut plans: Vec<ClusterPlan> = Vec::with_capacity(ordered_clusters.len());
    let mut round_robin = 0usize;

    for cluster in ordered_clusters {
        let starting_coord = cluster.centroid;

        let agent = pick_agent(cluster, agents, agent_loads, config, round_robin);
        round_robin = round_robin.wrapping_add(1);
        let _ = round_robin; // suppress unused if next branch bypasses update

        agent_loads.entry(agent.clone()).and_modify(|n| *n += 1).or_insert(1);

        plans.push(ClusterPlan {
            cluster: (*cluster).clone(),
            agent: agent.clone(),
            starting_coord,
        });
    }

    plans.sort_by_key(|p| p.cluster.id.clone());
    plans
}

fn pick_agent(
    cluster: &Cluster,
    agents: &[Agent],
    agent_loads: &mut HashMap<AgentId, u32>,
    config: AssignmentConfig,
    round_robin_seed: usize,
) -> AgentId {
    // Eligible = the agent has not exceeded `max_clusters_per_agent`.
    // Among them pick the one closest to the cluster centroid.
    // Tie-break: smallest current load; final tie-break: lexicographically
    // smallest AgentId (deterministic).
    let mut best: Option<(AgentId, u32, u32)> = None;
    for agent in agents {
        let load = agent_loads.get(&agent.id).copied().unwrap_or(0);
        if load >= config.max_clusters_per_agent {
            continue;
        }
        let dist = agent.coord.distance(cluster.centroid);
        let key = (agent.id.clone(), dist, load);
        best = match best {
            None => Some(key),
            Some(cur) => {
                let cmp = dist.cmp(&cur.1).then(load.cmp(&cur.2)).then(std::cmp::Ordering::Equal); // won't matter on distance & load ties
                let prefer_key = if cmp == std::cmp::Ordering::Less {
                    key
                } else if cmp == std::cmp::Ordering::Greater {
                    cur
                } else if key.0.0 < cur.0.0 {
                    key
                } else {
                    cur
                };
                Some(prefer_key)
            }
        };
    }

    if let Some((id, _, _)) = best {
        return id;
    }

    // Fall back: every agent is loaded up. Spread the surplus by round-robin
    // (deterministic seed comes from the cluster's order in the input).
    let idx = round_robin_seed % agents.len();
    agents[idx].id.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::ClusterConfig;
    use crate::types::{Series, Spot, SpotId};

    fn spot(id: &str, q: i32, r: i32, s: &str) -> Spot {
        Spot {
            id: SpotId::new(id),
            coord: HexCoord::new(q, r),
            series: Series::new(s),
            initial_stock: 1,
        }
    }

    fn agent(id: &str, q: i32, r: i32) -> Agent {
        Agent { id: AgentId::new(id), coord: HexCoord::new(q, r), patrol_radius: 8 }
    }

    #[test]
    fn assigns_each_cluster_to_some_agent() {
        let req = SolveRequest {
            spots: vec![spot("a", 0, 0, "udon"), spot("b", 0, 0, "soba"), spot("c", 0, 0, "ramen")],
            agents: vec![agent("p1", 0, 0), agent("p2", 1, 0)],
        };
        let cluster = Clustering::build(&req.spots, ClusterConfig::default());
        let a = assign_clusters(&req, &cluster, AssignmentConfig::default());
        assert_eq!(a.plans.len(), 3);
        // Every spot is assigned to exactly one of the known agents.
        for who in a.spots_to_agent.values() {
            assert!(who.as_str() == "p1" || who.as_str() == "p2");
        }
    }

    #[test]
    fn largest_cluster_wins_the_first_agent() {
        let req = SolveRequest {
            spots: vec![
                spot("a", 0, 0, "udon"),
                spot("b", 1, 0, "udon"),
                spot("c", 10, 10, "udon"), // singleton, far away
            ],
            agents: vec![agent("p1", 0, 0), agent("p2", 10, 10)],
        };
        let cluster = Clustering::build(&req.spots, ClusterConfig { merge_radius: 1 });
        let a = assign_clusters(&req, &cluster, AssignmentConfig::default());
        // The (a,b) cluster of 2 members is closest to p1, the singleton is
        // closest to p2.
        let for_p1: usize = a
            .plans
            .iter()
            .filter(|p| p.agent.as_str() == "p1")
            .map(|p| p.cluster.members.len())
            .sum();
        let for_p2: usize = a
            .plans
            .iter()
            .filter(|p| p.agent.as_str() == "p2")
            .map(|p| p.cluster.members.len())
            .sum();
        assert!(for_p1 >= 2);
        assert!(for_p2 >= 1);
    }

    #[test]
    fn honours_max_clusters_per_agent_with_surplus() {
        let req = SolveRequest {
            spots: vec![
                spot("a", 0, 0, "u"),
                spot("b", 5, 5, "u"),
                spot("c", 10, 10, "u"),
                spot("d", 15, 15, "u"),
            ],
            agents: vec![agent("p1", 0, 0)],
        };
        let cluster = Clustering::build(&req.spots, ClusterConfig { merge_radius: 1 });
        // 4 clusters, 1 agent, default cap = 2. Should reach the round-robin
        // fallback for the remaining 2 clusters, and the agent keeps all of
        // them (the map must remain fully covered).
        let a = assign_clusters(&req, &cluster, AssignmentConfig::default());
        assert_eq!(a.plans.len(), 4);
        for plan in &a.plans {
            assert_eq!(plan.agent.as_str(), "p1");
        }
    }
}

// =====================================================================
// Issue #29: series-level assignment + tour order optimisation.
//
// The casts inside `series_assignment` are intentional: contest maps are
// bounded so any `as u32` / `as i64` truncation on the spot-count or
// coordinate arithmetic is safe by construction. The coordinate mean also
// fits in `i32` because the 32×32 hex grid stays well inside `i32` range.
// =====================================================================
mod series_assignment {
    #![allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]

    use super::{AgentId, HashMap, HexCoord, Series, Spot, SpotId};

    /// Strategy for distributing [`Series`] groups across [`AgentId`]s.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AssignmentStrategy {
        /// Sort series by name (deterministic) and hand them out to agents in
        /// round-robin order.
        RoundRobin,
        /// Sort series by spot count (largest first), always give the next
        /// series to the agent carrying the fewest spots so far. Tie-break by
        /// lexicographic `Series` name for determinism.
        BalancedBySpotCount,
        /// For each agent (in `AgentId` order) pick the series whose centroid
        /// sits closest to the agent's starting coord. Series the agents do not
        /// pick up fall back to round-robin on the leftover agents.
        NearestFirst,
    }

    /// One series committed to one agent, with an optimised visiting order.
    ///
    /// The order is the result of a nearest-neighbour seed followed by 2-opt
    /// refinement, both measured in hex steps and seeded from the series'
    /// centroid so the result is independent of the (unknown) agent position.
    /// Callers may re-route from the agent's actual position; the relative
    /// order is the optimisation target.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SeriesTour {
        pub series_id: String,
        pub agent_id: AgentId,
        pub tour: Vec<SpotId>,
    }

    impl SeriesTour {
        /// Sum of hex distances between consecutive spots in `tour`.
        ///
        /// `coord_of` must map every spot id in `tour` to its coordinate; the
        /// first segment (from the implicit start to the first spot) is not
        /// counted because the agent's starting coordinate is not part of the
        /// stored tour.
        #[must_use]
        pub fn tour_length(&self, coord_of: &HashMap<SpotId, HexCoord>) -> u32 {
            let mut total = 0u32;
            let mut prev: Option<HexCoord> = None;
            for sid in &self.tour {
                if let Some(&coord) = coord_of.get(sid) {
                    if let Some(p) = prev {
                        total = total.saturating_add(p.distance(coord));
                    }
                    prev = Some(coord);
                }
            }
            total
        }
    }

    /// Builds a [`SeriesTour`] per (series, agent) pair.
    #[derive(Debug, Clone, Copy)]
    pub struct SeriesAssigner {
        strategy: AssignmentStrategy,
    }

    impl SeriesAssigner {
        /// Build an assigner with an explicit strategy.
        #[must_use]
        pub const fn new(strategy: AssignmentStrategy) -> Self {
            Self { strategy }
        }

        /// Convenience: round-robin assigner.
        #[must_use]
        pub fn round_robin() -> Self {
            Self::new(AssignmentStrategy::RoundRobin)
        }

        /// Convenience: balanced-by-spot-count assigner.
        #[must_use]
        pub fn balanced() -> Self {
            Self::new(AssignmentStrategy::BalancedBySpotCount)
        }

        /// Convenience: nearest-first assigner.
        #[must_use]
        pub fn nearest_first() -> Self {
            Self::new(AssignmentStrategy::NearestFirst)
        }

        /// Distribute `series` across `agents` and produce an optimised tour
        /// per assignment. The returned vector is sorted by
        /// `(agent_id, series_id)` so the output is deterministic for any
        /// given `(agents, series)` input.
        ///
        /// Series whose `spots` vector is empty are skipped — there is nothing
        /// to visit. Agents that receive no series do not appear in the output.
        #[must_use]
        pub fn assign(self, agents: &[AgentId], series: &[Series]) -> Vec<SeriesTour> {
            if agents.is_empty() {
                return Vec::new();
            }

            // Deterministic agent order for tie-breaks and the round-robin index.
            let mut sorted_agents: Vec<&AgentId> = agents.iter().collect();
            sorted_agents.sort();

            // Filter empty series + sort according to the strategy.
            let non_empty: Vec<&Series> = series.iter().filter(|s| !s.is_empty()).collect();
            if non_empty.is_empty() {
                return Vec::new();
            }

            let ordered = order_series(&non_empty, self.strategy);

            let mut agent_loads: HashMap<AgentId, u32> = HashMap::new();
            let mut plans: Vec<SeriesTour> = Vec::with_capacity(ordered.len());

            match self.strategy {
                AssignmentStrategy::RoundRobin | AssignmentStrategy::BalancedBySpotCount => {
                    for s in &ordered {
                        let agent =
                            pick_min_load(&sorted_agents, &mut agent_loads, s.spots.len() as u32);
                        let tour = optimise_tour(&s.spots);
                        *agent_loads.entry(agent.clone()).or_insert(0) += s.spots.len() as u32;
                        plans.push(SeriesTour { series_id: s.name.clone(), agent_id: agent, tour });
                    }
                }
                AssignmentStrategy::NearestFirst => {
                    // For each (sorted) agent, claim the unclaimed series whose
                    // centroid is closest to the agent's home coord. Agents
                    // without a coordinate are placed at (0, 0) for ranking
                    // purposes (we have no Agent here — just the id). When
                    // multiple agents would otherwise claim the same series
                    // we fall back to lexicographic agent order so the result
                    // stays deterministic.
                    //
                    // The `home` coord is the centroid of the first spot in
                    // series `s` for ranking when an agent has no spatial
                    // signal; the algorithm still walks the agents in sorted
                    // id order and assigns them the nearest unclaimed centroid.
                    let mut claimed: Vec<bool> = vec![false; ordered.len()];
                    let centroids: Vec<HexCoord> =
                        ordered.iter().map(|s| centroid_of(&s.spots)).collect();

                    for agent in &sorted_agents {
                        // No agent coords are available; use the centroid of the
                        // first remaining series as a stable proxy. This keeps
                        // the algorithm deterministic across runs.
                        let home = next_unclaimed_centroid(&ordered, &claimed);
                        let Some(home) = home else { continue };
                        let best = best_unclaimed(&centroids, &claimed, home, agent.as_str());
                        let Some(idx) = best else { continue };
                        claimed[idx] = true;
                        let s = ordered[idx];
                        let tour = optimise_tour(&s.spots);
                        *agent_loads.entry((*agent).clone()).or_insert(0) += s.spots.len() as u32;
                        plans.push(SeriesTour {
                            series_id: s.name.clone(),
                            agent_id: (*agent).clone(),
                            tour,
                        });
                    }

                    // Anything still unclaimed goes round-robin on the sorted
                    // agents so the map is fully covered.
                    let mut rr = 0usize;
                    for (idx, s) in ordered.iter().enumerate() {
                        if claimed[idx] {
                            continue;
                        }
                        let agent = sorted_agents[rr % sorted_agents.len()].clone();
                        rr = rr.wrapping_add(1);
                        let tour = optimise_tour(&s.spots);
                        *agent_loads.entry(agent.clone()).or_insert(0) += s.spots.len() as u32;
                        plans.push(SeriesTour { series_id: s.name.clone(), agent_id: agent, tour });
                    }
                }
            }

            plans.sort_by(|a, b| {
                a.agent_id.cmp(&b.agent_id).then_with(|| a.series_id.cmp(&b.series_id))
            });
            plans
        }
    }

    /// Apply the strategy-specific ordering to a slice of series references.
    fn order_series<'a>(series: &[&'a Series], strategy: AssignmentStrategy) -> Vec<&'a Series> {
        let mut ordered: Vec<&Series> = series.to_vec();
        match strategy {
            AssignmentStrategy::RoundRobin | AssignmentStrategy::NearestFirst => {
                ordered.sort_by(|a, b| a.name.cmp(&b.name));
            }
            AssignmentStrategy::BalancedBySpotCount => {
                // Largest first; tie-break by name for determinism.
                ordered.sort_by(|a, b| {
                    b.spots.len().cmp(&a.spots.len()).then_with(|| a.name.cmp(&b.name))
                });
            }
        }
        ordered
    }

    /// Pick the agent with the smallest current spot load; tie-break by
    /// lexicographic `AgentId` for determinism.
    fn pick_min_load(
        sorted_agents: &[&AgentId],
        loads: &mut HashMap<AgentId, u32>,
        weight: u32,
    ) -> AgentId {
        let mut best: Option<(AgentId, u32)> = None;
        for agent in sorted_agents {
            let load = loads.get(*agent).copied().unwrap_or(0);
            let candidate_load = load.saturating_add(weight);
            best = match best {
                None => Some(((*agent).clone(), candidate_load)),
                Some((cur_id, cur_load)) => {
                    if candidate_load < cur_load
                        || (candidate_load == cur_load && (*agent).as_str() < cur_id.as_str())
                    {
                        Some(((*agent).clone(), candidate_load))
                    } else {
                        Some((cur_id, cur_load))
                    }
                }
            };
        }
        best.map_or_else(|| sorted_agents[0].clone(), |(id, _)| id)
    }

    /// Compute the centroid of a set of spots as a hex coordinate, using the
    /// mean of q/rounded half-up.
    fn centroid_of(spots: &[Spot]) -> HexCoord {
        if spots.is_empty() {
            return HexCoord::new(0, 0);
        }
        let n = spots.len() as i64;
        let sum_q: i64 = spots.iter().map(|s| i64::from(s.coord.q)).sum();
        let sum_r: i64 = spots.iter().map(|s| i64::from(s.coord.r)).sum();
        // Round half-up: floor((x / n) + 0.5) == floor((2x + n) / (2n)).
        // `div_euclid` floors; plain `/` truncates toward zero and so lands
        // one off for a negative numerator (`-6/8` -> 0, not -1), which
        // contradicted the comment above and `cluster::centroid_and_radius`.
        let q = (2 * sum_q + n).div_euclid(2 * n) as i32;
        let r = (2 * sum_r + n).div_euclid(2 * n) as i32;
        HexCoord::new(q, r)
    }

    /// Centroid of the first still-unclaimed series, used as a deterministic
    /// proxy for "the agent's home coord" in the `NearestFirst` strategy when
    /// we do not have access to actual agent positions.
    fn next_unclaimed_centroid(ordered: &[&Series], claimed: &[bool]) -> Option<HexCoord> {
        ordered
            .iter()
            .zip(claimed.iter())
            .find_map(|(s, taken)| if *taken { None } else { Some(centroid_of(&s.spots)) })
    }

    /// Find the index of the unclaimed series whose centroid is closest to
    /// `home`. Tie-break by lexicographic series name. The `agent_label` is
    /// unused at the moment — kept for symmetry with future iterations that
    /// could rank by agent too.
    fn best_unclaimed(
        centroids: &[HexCoord],
        claimed: &[bool],
        home: HexCoord,
        _agent_label: &str,
    ) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (idx, cent) in centroids.iter().enumerate() {
            if claimed[idx] {
                continue;
            }
            let dist = home.distance(*cent);
            best = match best {
                None => Some((idx, dist)),
                Some((cur_idx, cur_dist)) => {
                    if dist < cur_dist {
                        Some((idx, dist))
                    } else {
                        Some((cur_idx, cur_dist))
                    }
                }
            };
        }
        best.map(|(idx, _)| idx)
    }

    /// Build an optimised tour: nearest-neighbour seed + 2-opt refinement.
    /// Returns spot ids in visiting order.
    fn optimise_tour(spots: &[Spot]) -> Vec<SpotId> {
        if spots.is_empty() {
            return Vec::new();
        }
        if spots.len() == 1 {
            return vec![spots[0].id.clone()];
        }

        let start = centroid_of(spots);
        let nn = nearest_neighbour_tour(spots, start);
        let coord_of: HashMap<SpotId, HexCoord> =
            spots.iter().map(|s| (s.id.clone(), s.coord)).collect();
        two_opt(nn, &coord_of)
    }

    /// Greedy nearest-neighbour tour from `start`.
    fn nearest_neighbour_tour(spots: &[Spot], start: HexCoord) -> Vec<SpotId> {
        let mut remaining: Vec<Spot> = spots.to_vec();
        let mut cursor = start;
        let mut tour: Vec<SpotId> = Vec::with_capacity(remaining.len());
        while !remaining.is_empty() {
            let nearest_idx = remaining
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| cursor.distance(s.coord))
                .map_or(0, |(i, _)| i);
            let chosen = remaining.swap_remove(nearest_idx);
            tour.push(chosen.id.clone());
            cursor = chosen.coord;
        }
        tour
    }

    /// 2-opt refinement: repeatedly reverse the segment between two non-adjacent
    /// edges when the swap strictly reduces the total tour length. The loop
    /// terminates when a full pass yields no improvement; a hard iteration cap
    /// guards against pathological inputs.
    fn two_opt(mut tour: Vec<SpotId>, coord_of: &HashMap<SpotId, HexCoord>) -> Vec<SpotId> {
        const MAX_ITERATIONS: usize = 32;
        let n = tour.len();
        if n < 4 {
            return tour;
        }

        let mut iterations = 0usize;

        loop {
            if iterations >= MAX_ITERATIONS {
                break;
            }
            iterations += 1;

            let mut improved = false;
            for i in 0..n.saturating_sub(2) {
                for j in (i + 2)..n.saturating_sub(1) {
                    let ai = coord_of[&tour[i]];
                    let bi = coord_of[&tour[i + 1]];
                    let cj = coord_of[&tour[j]];
                    let dj = coord_of[&tour[j + 1]];

                    let old = ai.distance(bi).saturating_add(cj.distance(dj));
                    let new = ai.distance(cj).saturating_add(bi.distance(dj));
                    if new < old {
                        tour[i + 1..=j].reverse();
                        improved = true;
                    }
                }
            }
            if !improved {
                break;
            }
        }

        tour
    }
} // mod series_assignment

pub use series_assignment::{AssignmentStrategy, SeriesAssigner, SeriesTour};
