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
use crate::types::{Agent, AgentId, HexCoord, SolveRequest};
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
