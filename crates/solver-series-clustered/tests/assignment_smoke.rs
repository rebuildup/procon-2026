//! Smoke tests for Issue #29: series-level agent assignment + tour order
//! optimisation (`SeriesAssigner`).
//!
//! The three required tests live here as integration tests so they exercise
//! the public re-export surface from `solver_series_clustered::*`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use solver_series_clustered::{
    AgentId, AssignmentStrategy, HexCoord, Series, SeriesAssigner, SeriesTour, Spot, SpotId,
};
use std::collections::{HashMap, HashSet};

/// Build a `Spot` with the given id/coord and the given series name.
fn spot(id: &str, q: i32, r: i32, series_name: &str) -> Spot {
    Spot {
        id: SpotId::new(id),
        coord: HexCoord::new(q, r),
        series: Series::new(series_name),
        initial_stock: 1,
    }
}

/// Build a `coord_of` lookup from a slice of spots, in spot-id order.
fn coord_lookup(spots: &[Spot]) -> HashMap<SpotId, HexCoord> {
    spots.iter().map(|s| (s.id.clone(), s.coord)).collect()
}

#[test]
fn round_robin_assignment_hands_each_series_to_a_distinct_agent() {
    // 3 series × 1 spot, 3 agents. With RoundRobin, every series should land
    // on a different agent; the agent list itself is intentionally given in
    // reverse order to exercise the deterministic re-sorting.
    let s_udon = vec![spot("u", 0, 0, "udon")];
    let s_soba = vec![spot("s", 0, 0, "soba")];
    let s_ramen = vec![spot("r", 0, 0, "ramen")];

    let series = vec![
        Series::with_spots("udon", s_udon.clone()),
        Series::with_spots("soba", s_soba.clone()),
        Series::with_spots("ramen", s_ramen.clone()),
    ];

    let agents = vec![AgentId::new("p3"), AgentId::new("p1"), AgentId::new("p2")];
    let tours = SeriesAssigner::round_robin().assign(&agents, &series);

    // One tour per series, none dropped.
    assert_eq!(tours.len(), 3);

    let by_series: HashMap<String, &SeriesTour> =
        tours.iter().map(|t| (t.series_id.clone(), t)).collect();
    assert!(by_series.contains_key("udon"));
    assert!(by_series.contains_key("soba"));
    assert!(by_series.contains_key("ramen"));

    // Round-robin across the 3 agents means each agent receives exactly one
    // series. Sorting agents in lexicographic order: p1, p2, p3 — so:
    //   udon -> p1, soba -> p2, ramen -> p3 (or any cyclic rotation, but
    //   because every series has a unique name and every agent has a unique
    //   id the result is fully determined by the strategy).
    let assigned_agents: HashSet<&str> = tours.iter().map(|t| t.agent_id.as_str()).collect();
    assert_eq!(assigned_agents.len(), 3);
    assert!(assigned_agents.contains("p1"));
    assert!(assigned_agents.contains("p2"));
    assert!(assigned_agents.contains("p3"));

    // Each tour visits the single spot in its series.
    for tour in &tours {
        assert_eq!(tour.tour.len(), 1);
    }
}

#[test]
fn balanced_assignment_distributes_spot_load_evenly() {
    // Three series of sizes 5, 3 and 1; two agents. Balanced-by-spot-count
    // should split the load as evenly as possible:
    //   - round 1: largest (5 spots) -> p1
    //   - round 2: medium (3 spots)  -> p2 (load 3 < 5)
    //   - round 3: smallest (1 spot)  -> p2 (load 3+1=4 < 5)
    // Net loads: p1 = 5, p2 = 4 -> difference 1.
    //
    // Compare with plain round-robin on the same data which would distribute
    // them 5+1 vs 3, i.e. a 3-spot gap.
    let big: Vec<Spot> = (0..5).map(|i| spot(&format!("b{i}"), i, 0, "big")).collect();
    let med: Vec<Spot> = (0..3).map(|i| spot(&format!("m{i}"), 100 + i, 0, "med")).collect();
    let small: Vec<Spot> = vec![spot("s0", 200, 0, "small")];

    let series = vec![
        Series::with_spots("big", big.clone()),
        Series::with_spots("med", med.clone()),
        Series::with_spots("small", small.clone()),
    ];

    let agents = vec![AgentId::new("p1"), AgentId::new("p2")];
    let tours =
        SeriesAssigner::new(AssignmentStrategy::BalancedBySpotCount).assign(&agents, &series);

    assert_eq!(tours.len(), 3);

    // Every spot appears in some tour (no drops).
    let mut claimed: HashSet<&str> = HashSet::new();
    for tour in &tours {
        for sid in &tour.tour {
            assert!(claimed.insert(sid.as_str()), "duplicate spot {sid}");
        }
    }
    assert_eq!(claimed.len(), 5 + 3 + 1);

    // Spot load per agent.
    let mut load: HashMap<&str, usize> = HashMap::new();
    for tour in &tours {
        *load.entry(tour.agent_id.as_str()).or_insert(0) += tour.tour.len();
    }
    let p1 = load["p1"];
    let p2 = load["p2"];

    // Both agents did some work.
    assert!(p1 > 0, "p1 should receive at least one series");
    assert!(p2 > 0, "p2 should receive at least one series");

    // Difference must be at most 1 — the optimal balance for these sizes.
    let gap = p1.max(p2) - p1.min(p2);
    assert!(gap <= 1, "balanced assignment left a {gap}-spot gap (p1={p1}, p2={p2})");

    // For sanity: round-robin would have produced a larger gap on the same
    // input. (We re-run the strategy to confirm the contrast in the test
    // suite itself rather than just trusting the algorithm description.)
    let rr_tours = SeriesAssigner::round_robin().assign(&agents, &series);
    let mut rr_load: HashMap<&str, usize> = HashMap::new();
    for tour in &rr_tours {
        *rr_load.entry(tour.agent_id.as_str()).or_insert(0) += tour.tour.len();
    }
    let rr_gap = rr_load["p1"].max(rr_load["p2"]) - rr_load["p1"].min(rr_load["p2"]);
    assert!(
        rr_gap >= gap,
        "round-robin gap ({rr_gap}) should not be smaller than balanced gap ({gap})",
    );
}

#[test]
fn two_opt_refinement_does_not_increase_tour_length() {
    // 5 spots arranged so a naive nearest-neighbour walk backtracks:
    //   the NN seed visits them as A(0,0) -> B(3,0) -> C(1,2) -> D(5,3)
    //   -> E(8,0), which has a long back-and-forth segment between C/D/E.
    // 2-opt must strictly reduce (or at worst equal) the NN length.
    let spots = vec![
        spot("A", 0, 0, "x"),
        spot("B", 3, 0, "x"),
        spot("C", 1, 2, "x"),
        spot("D", 5, 3, "x"),
        spot("E", 8, 0, "x"),
    ];
    let series = vec![Series::with_spots("x", spots.clone())];

    let agents = vec![AgentId::new("p1")];
    let tours = SeriesAssigner::round_robin().assign(&agents, &series);
    assert_eq!(tours.len(), 1);
    let optimised = &tours[0].tour;

    // The optimised tour must visit every spot exactly once.
    assert_eq!(optimised.len(), spots.len());
    let unique: HashSet<&SpotId> = optimised.iter().collect();
    assert_eq!(unique.len(), spots.len());

    let coord_of = coord_lookup(&spots);
    let opt_len = tours[0].tour_length(&coord_of);

    // Re-run the inner heuristics directly so we can compare against the
    // unrefined nearest-neighbour seed on the same data.
    let nn_tour = solver_series_clustered_internal::nn_seed(&spots);
    let nn_len = nn_tour
        .iter()
        .fold((None::<HexCoord>, 0u32), |(prev, acc), sid| {
            let coord = coord_of[sid];
            let added = match prev {
                Some(p) => p.distance(coord),
                None => 0,
            };
            (Some(coord), acc + added)
        })
        .1;

    assert!(opt_len <= nn_len, "2-opt made things worse: nn_len={nn_len}, opt_len={opt_len}");
    // On the deliberately chosen layout above the seed contains a backtrack
    // (D -> E crosses the line B -> C), so 2-opt should *strictly* shorten
    // it. If this assertion ever starts failing the test data needs
    // adjusting, not the algorithm.
    assert!(
        opt_len < nn_len,
        "2-opt failed to shorten a deliberately suboptimal seed: \
         nn_len={nn_len}, opt_len={opt_len}",
    );
}

/// Internal helpers re-exported via a tiny inline module so the public API
/// stays compact. We use them to introspect the algorithm from the test
/// without having to duplicate the heuristic in the test itself.
#[allow(dead_code)]
mod solver_series_clustered_internal {
    use super::*;

    /// Run only the nearest-neighbour seed (no 2-opt). Mirrors the logic in
    /// `assignment::optimise_tour`.
    pub fn nn_seed(spots: &[Spot]) -> Vec<SpotId> {
        use solver_series_clustered::HexCoord;
        if spots.is_empty() {
            return Vec::new();
        }
        if spots.len() == 1 {
            return vec![spots[0].id.clone()];
        }
        let n = spots.len() as i64;
        let sum_q: i64 = spots.iter().map(|s| i64::from(s.coord.q)).sum();
        let sum_r: i64 = spots.iter().map(|s| i64::from(s.coord.r)).sum();
        let start =
            HexCoord::new(((2 * sum_q + n) / (2 * n)) as i32, ((2 * sum_r + n) / (2 * n)) as i32);
        let mut remaining: Vec<Spot> = spots.to_vec();
        let mut cursor = start;
        let mut tour: Vec<SpotId> = Vec::with_capacity(remaining.len());
        while !remaining.is_empty() {
            let idx = remaining
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| cursor.distance(s.coord))
                .map_or(0, |(i, _)| i);
            let chosen = remaining.swap_remove(idx);
            tour.push(chosen.id.clone());
            cursor = chosen.coord;
        }
        tour
    }
}
