//! Spatial clustering — Union-Find over spatially-adjacent, same-series spots.
//!
//! See `docs/contest/strategy.md` §5.3. The high-level idea:
//!
//! 1. For every pair of spots whose series match and whose hex distance is
//!    within `merge_radius`, unify them in a disjoint-set.
//! 2. Union-Find uses path compression + union by rank so the amortised
//!    cost of any `find` / `union` operation is `O(α(n))` (inverse Ackermann).
//! 3. After all unions we collect the resulting connected components, one of
//!    which corresponds to one cluster.
//!
//! A cluster additionally stores its centroid (mean of `HexCoord` components
//! rounded to the nearest cell) and bounding-box extent (max hex distance from
//! the centroid), so downstream solvers can reason about feasible coverage.
//!
//! Casting truncations in the arithmetic below are guarded: cluster ids fit in
//! `u32` because the contest cap is 32 spots per cluster map, and centroid
//! coordinates fit in `i32` because the map itself fits in a 32×32 hex grid.

#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss, clippy::cast_sign_loss)]

use crate::types::{HexCoord, Series, Spot, SpotId};
use std::collections::HashMap;

/// Path-compressing, rank-balanced disjoint-set forest.
///
/// Public to allow callers (e.g. tests, the assignment layer) to introspect
/// representative ids; they should not mutate the internal `parent` / `rank`
/// vectors directly.
#[derive(Debug, Clone)]
pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    /// Construct a Union-Find of size `n`. Each element starts in its own set
    /// and is its own parent.
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self { parent: (0..n).collect(), rank: vec![0; n] }
    }

    /// Representative for element `x`. Amortised `O(α(n))`.
    #[must_use]
    pub fn find(&mut self, mut x: usize) -> usize {
        // Two-pass with path compression:
        // pass 1 walks parent pointers until it reaches the root;
        // pass 2 rewrites every visited edge to point directly at the root.
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        while self.parent[x] != root {
            let next = self.parent[x];
            self.parent[x] = root;
            x = next;
        }
        root
    }

    /// Union the sets containing `a` and `b`. Returns `true` if they were
    /// distinct before the call. Amortised `O(α(n))`.
    pub fn union(&mut self, a: usize, b: usize) -> bool {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return false;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => {
                self.parent[ra] = rb;
            }
            std::cmp::Ordering::Greater => {
                self.parent[rb] = ra;
            }
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] = self.rank[ra].saturating_add(1);
            }
        }
        true
    }

    /// Number of distinct sets in the forest over the live range `0..len()`.
    /// Each `find` call is amortised `O(α(n))`, so total complexity is `O(n)`.
    #[must_use]
    pub fn set_count(&mut self) -> usize {
        let mut reps = std::collections::HashSet::with_capacity(self.parent.len());
        for i in 0..self.parent.len() {
            reps.insert(self.find(i));
        }
        reps.len()
    }
}

/// One spatial cluster — a connected component of same-series spots.
///
/// Cluster ids are assigned by [`Clustering::build`] in deterministic order
/// (sorted by smallest member spot id, ascending).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cluster {
    pub id: ClusterId,
    pub series: Series,
    pub members: Vec<SpotId>,
    pub centroid: HexCoord,
    /// Max hex distance from `centroid` to any member; useful as a coverage
    /// radius hint.
    pub radius: u32,
}

/// Newtype so `Cluster` stays distinguishable from other ids in error messages.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClusterId(pub u32);

impl ClusterId {
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for ClusterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cluster-{}", self.0)
    }
}

/// Output of a clustering pass.
#[derive(Debug, Clone)]
pub struct Clustering {
    pub clusters: Vec<Cluster>,
    /// Original spot ids in their request order, paired with their cluster id.
    pub membership: Vec<(SpotId, ClusterId)>,
}

/// Configuration for [`Clustering::build`].
#[derive(Debug, Clone, Copy)]
pub struct ClusterConfig {
    /// Spots within `merge_radius` hex steps of each other (and sharing a
    /// series) are unified. Defaults to `1` (immediate hex neighbours only).
    pub merge_radius: u32,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self { merge_radius: 1 }
    }
}

impl Clustering {
    /// Cluster `spots` according to `config`.
    ///
    /// The merge criterion is:
    /// `same_series && hex_distance(other) <= merge_radius`.
    ///
    /// This deliberately ignores cross-series edges so a flavour family that
    /// happens to be collinear still becomes its own cluster. The reasoning is
    /// recorded in `strategy.md §5.3`: a single patrol agent picking from
    /// multiple series in the same trip costs variety.
    #[must_use]
    pub fn build(spots: &[Spot], config: ClusterConfig) -> Self {
        if spots.is_empty() {
            return Self { clusters: Vec::new(), membership: Vec::new() };
        }

        // Pre-build a per-series index so the O(n²) merge pass is bounded
        // by the largest series, not the whole map.
        let mut by_series: HashMap<&Series, Vec<usize>> = HashMap::new();
        for (idx, spot) in spots.iter().enumerate() {
            by_series.entry(&spot.series).or_default().push(idx);
        }

        let mut uf = UnionFind::new(spots.len());

        for indices in by_series.values() {
            // O(k²) merge pass within one series, where k is the series size.
            // Every pair has to be considered: the previous
            // `windows(2)` pass only compared *consecutive input entries*,
            // so two spots that were close in space merged only when they
            // happened to sit next to each other in the slice — the result
            // depended on input order instead of on `merge_radius`.
            for (pos, &i) in indices.iter().enumerate() {
                for &j in &indices[pos + 1..] {
                    let a = &spots[i];
                    let b = &spots[j];
                    if a.coord.distance(b.coord) <= config.merge_radius {
                        uf.union(i, j);
                    }
                }
            }
        }

        // Bucket spots by their representative.
        let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
        for idx in 0..spots.len() {
            let rep = uf.find(idx);
            components.entry(rep).or_default().push(idx);
        }

        // Sort components by the smallest member id so cluster ids are
        // stable regardless of HashMap iteration order.
        let mut component_lists: Vec<Vec<usize>> = components.into_values().collect();
        component_lists.sort_by_key(|members| {
            members
                .iter()
                .map(|i| spots[*i].id.as_str())
                .min()
                .map(str::to_owned)
                .unwrap_or_default()
        });

        let mut clusters = Vec::with_capacity(component_lists.len());
        let mut membership = Vec::with_capacity(spots.len());

        for (cid, indices) in component_lists.into_iter().enumerate() {
            let series = spots[indices[0]].series.clone();
            let members: Vec<SpotId> = indices.iter().map(|i| spots[*i].id.clone()).collect();
            let (centroid, radius) = centroid_and_radius(&indices, spots);

            let cluster_id = ClusterId::new(cid as u32);
            clusters.push(Cluster { id: cluster_id.clone(), series, members, centroid, radius });

            for i in indices {
                membership.push((spots[i].id.clone(), cluster_id.clone()));
            }
        }

        Self { clusters, membership }
    }
}

/// Mean of integer hex coordinates, rounded to the nearest axial cell with
/// the half-up rule (a fractional part of exactly 0.5 adds one toward
/// positive infinity).
fn centroid_and_radius(indices: &[usize], spots: &[Spot]) -> (HexCoord, u32) {
    let n = indices.len() as f64;
    let mut sum_q: f64 = 0.0;
    let mut sum_r: f64 = 0.0;
    for &i in indices {
        sum_q += f64::from(spots[i].coord.q);
        sum_r += f64::from(spots[i].coord.r);
    }
    let mean_q = sum_q / n;
    let mean_r = sum_r / n;
    // Round half-up: `floor(x + 0.5)` gives the desired behaviour for any
    // signed value because `floor` always rounds toward negative infinity.
    let round_half_up = |x: f64| (x + 0.5).floor();
    let q = round_half_up(mean_q) as i32;
    let r = round_half_up(mean_r) as i32;
    let centroid = HexCoord::new(q, r);

    let radius = indices.iter().map(|i| centroid.distance(spots[*i].coord)).max().unwrap_or(0);

    (centroid, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(id: &str, q: i32, r: i32, series: &str) -> Spot {
        Spot {
            id: SpotId::new(id),
            coord: HexCoord::new(q, r),
            series: Series::new(series),
            initial_stock: 1,
        }
    }

    #[test]
    fn union_find_unites_and_reflects_distinctness() {
        let mut uf = UnionFind::new(4);
        assert!(uf.union(0, 1));
        assert!(uf.union(1, 2));
        assert!(!uf.union(0, 2));
        assert_eq!(uf.find(0), uf.find(2));
        assert_ne!(uf.find(0), uf.find(3));
        assert_eq!(uf.set_count(), 2);
    }

    #[test]
    fn clustering_keeps_each_series_in_its_own_cluster() {
        // Two series on the same tile: even though they overlap spatially
        // they must remain distinct clusters to preserve variety.
        let spots = vec![spot("a", 0, 0, "udon"), spot("b", 0, 0, "soba")];
        let c = Clustering::build(&spots, ClusterConfig::default());
        assert_eq!(c.clusters.len(), 2);
        assert_ne!(c.clusters[0].series, c.clusters[1].series);
    }

    #[test]
    fn clustering_unites_close_same_series_spots() {
        let spots = vec![
            spot("a", 0, 0, "udon"),
            spot("b", 1, 0, "udon"),
            spot("c", 2, 0, "udon"),
            spot("d", 5, 0, "udon"),
        ];
        let c = Clustering::build(&spots, ClusterConfig { merge_radius: 1 });
        // a,b,c form one chain; d is isolated.
        assert_eq!(c.clusters.len(), 2);
        let sizes: Vec<usize> = c.clusters.iter().map(|cl| cl.members.len()).collect();
        assert!(sizes.contains(&3));
        assert!(sizes.contains(&1));
    }

    /// Input order must not decide the outcome: `b` sits far from the other
    /// two, so the close pair (a, c) is *not* adjacent in the input and the
    /// old consecutive-only scan never compared them.
    #[test]
    fn clustering_merges_close_pairs_that_are_not_adjacent_in_input() {
        let spots = vec![
            spot("a", 0, 0, "udon"),
            spot("b", 10, 10, "udon"),
            spot("c", 1, 0, "udon"),
        ];
        let c = Clustering::build(&spots, ClusterConfig { merge_radius: 1 });
        assert_eq!(c.clusters.len(), 2, "a and c merge across the gap b leaves");
        assert_eq!(c.clusters[0].members.len() + c.clusters[1].members.len(), 3);
    }

    #[test]
    fn centroid_rounds_with_member_count() {
        // Two spots at hex-distance 2 -> still merged when merge_radius=2.
        // Their centroid sits at (0.5, -0.5): round-half-up to (1, 0).
        let spots = vec![spot("a", 0, -1, "x"), spot("b", 1, 0, "x")];
        let c = Clustering::build(&spots, ClusterConfig { merge_radius: 2 });
        assert_eq!(c.clusters[0].members.len(), 2);
        // Both fractional parts are .5 -> centroid should round toward +q,+r.
        let cen = c.clusters[0].centroid;
        assert_eq!(cen, HexCoord::new(1, 0));
    }
}
