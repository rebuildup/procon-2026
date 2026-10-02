//! `multiloop` candidate — region-clustering tours from
//! `solver_loop::MultiLoopComposer`.
//!
//! `loop` gives every agent the same whole-map tour and lets the day walk
//! sort out who actually goes where. `multiloop` is a *different mechanism*
//! (Issue #190): partition the spots into regions first, build one independent
//! tour per region, then hand regions to agents round-robin — so agent
//! spatial division of labour is explicit rather than emergent.
//!
//! ```text
//! multiloop:grid=2x2                       rows × cols bounding-box grid
//! multiloop:grid=3x3
//! multiloop:kmeans=4:seed=42               k-means clustering (seeded)
//! multiloop:grid=2x2:refine=false          skip per-region 2-opt
//! ```
//!
//! # Parameter rules
//!
//! - **Exactly one of `grid` / `kmeans`** selects the partition strategy.
//! - `seed` is required **iff** `kmeans` is present — the same pattern as
//!   `loop:initial=random` requiring `seed`. With `grid` a `seed` is a
//!   contradiction (grid needs no randomness) and is rejected.
//! - `refine` toggles per-region 2-opt refinement
//!   (`MultiLoopComposer::new` vs `MultiLoopComposer::without_refinement`),
//!   defaulting to the composer's own default: **`true`**.
//!
//! # Day contract
//!
//! Identical to [`super::loop_solver`]: re-solve the touring problem once per
//! day from the live [`engine::model::MatchState`], map the resulting spot-id
//! plans to coordinates, and walk them with the selected `walk` mode —
//! `greedy` (the default) is [`super::walk::plan_walk`] byte for byte,
//! `astar` is [`super::astar_walk::plan_walk_astar`] (Issue #215). The
//! adapter reuses the loop adapter's `problem_from_state` / `agent_targets`
//! helpers so the two candidates differ only in tour construction (and, with
//! `walk=`, in how the resulting lists are walked, and with `assign=`, in who
//! receives a list at all — Issue #235, see [`super::walk::AssignMode`]).

use engine::model::{AgentActions, Solver, SolverContext};
use solver_loop::{ClusteringStrategy, MultiLoopComposer, Solver as _};

use super::loop_solver::{agent_targets, problem_from_state};
use super::walk::{
    AssignMode, BudgetMode, WalkMode, parse_assign, parse_budget, parse_walk, plan_walk_with,
};
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};

/// Parameter schema, in canonical id order.
pub const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "grid",
        required: false,
        doc: "bounding-box partition as <rows>x<cols> (each 1..=32, rows*cols <= 64), \
              e.g. 2x2; exactly one of grid / kmeans",
    },
    ParamSpec {
        key: "kmeans",
        required: false,
        doc: "k-means cluster count k as a plain integer 1..=64 (write kmeans=4, not \
              kmeans=k=4); exactly one of grid / kmeans; requires seed",
    },
    ParamSpec {
        key: "seed",
        required: false,
        doc: "deterministic seed for kmeans (u64); required when kmeans is present, \
              rejected when it is not",
    },
    ParamSpec {
        key: "refine",
        required: false,
        doc: "per-region 2-opt refinement: true | false (default true)",
    },
    ParamSpec {
        key: "walk",
        required: false,
        doc: "day walk over the solved target lists: greedy (default; today's plan_walk) | \
              astar (visit each list in order with A* legs, Issue #215)",
    },
    ParamSpec {
        key: "assign",
        required: false,
        doc: "who receives a tour target: agents (default; every agent, pre-#235) | \
              patrol (Supply agents get no target and wait, Issue #235) | \
              escort (Supply walks its paired Patrol's projected day path, Issue #248) | \
              all (every Patrol gets the union of every agent's share, Issue #253)",
    },
    ParamSpec {
        key: "budget",
        required: false,
        doc: "day budget clock: element (default; daySteps movement elements, byte-identical) | \
              official (truncate the finished plan to the official weighted budget before \
              simulating, Issue #255)",
    },
];

/// Widest accepted grid dimension: the contest map is at most
/// `MapSize::MAX = 32` cells on a side, so a grid finer than that on either
/// axis can only carve empty cells out of an empty coordinate range.
const MAX_GRID_DIM: usize = 32;

/// Widest accepted region count, for both `grid` and `kmeans`.
///
/// The bundled corpus tops out at 16 spots and 8 agents
/// (`fixture:large-32x32`), so `64` is 4× the largest spot count and 8× the
/// largest agent count. Past the spot count every extra region is empty by
/// construction, and past the agent count every extra region merely appends
/// to an agent that already has a tour — so 64 is a generous ceiling well
/// beyond any partition that can still express a hypothesis.
const MAX_REGIONS: usize = 64;

/// Lloyd iteration cap passed to `kmeans`. K-means on at most
/// `MapSize::MAX² = 1024` points converges (assignment stops changing) far
/// below this; the cap only bounds the worst case. Fixed so a candidate id
/// fully determines the run.
const KMEANS_MAX_ITERS: usize = 50;

/// [`MultiLoopComposer`] configured from a candidate spec, driving one day at
/// a time against the live match state.
pub struct MultiLoopCandidate {
    inner: MultiLoopComposer,
    /// The candidate id, echoed into solver naming for logs.
    id: String,
    /// How the solved per-agent target lists are walked
    /// ([`super::walk::WalkMode`]).
    walk: WalkMode,
    /// Which agents receive a target list at all ([`super::walk::AssignMode`]).
    assign: AssignMode,
    /// Which clock the finished plan is checked against ([`super::walk::BudgetMode`]).
    budget: BudgetMode,
}

impl MultiLoopCandidate {
    /// Build from an already-validated spec.
    fn from_spec(spec: &CandidateSpec) -> Result<Self, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        let (num_regions, clustering) = clustering_from_spec(spec)?;
        let refine = parse_refine(spec)?;
        let walk = parse_walk(spec)?;
        let assign = parse_assign(spec)?;
        let budget = parse_budget(spec)?;
        let inner = if refine {
            MultiLoopComposer::new(num_regions, clustering)
        } else {
            MultiLoopComposer::without_refinement(num_regions, clustering)
        };
        Ok(Self { inner, id: spec.id(), walk, assign, budget })
    }
}

impl Solver for MultiLoopCandidate {
    fn name(&self) -> &'static str {
        // `Solver::name` is a compile-time constant; per-run parameters live
        // in the candidate id, which the evaluator records separately.
        "multiloop"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let problem = problem_from_state(ctx);
        match self.inner.solve(&problem) {
            Ok(outcome) => {
                let per_agent = agent_targets(ctx, &outcome);
                let plan =
                    plan_walk_with(ctx, &self.assign.apply(ctx, per_agent, self.walk), self.walk);
                self.budget.apply(ctx, plan)
            }
            Err(err) => {
                tracing::debug!(candidate = %self.id, error = %err, "multiloop solve failed; waiting");
                AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
            }
        }
    }
}

/// Build the partition strategy and the matching region count from the spec.
///
/// Enforces the two cross-field rules of the schema:
/// exactly one of `grid` / `kmeans`, and `seed` present iff `kmeans` is.
///
/// # Errors
/// [`CandidateError::MissingParameter`] when neither strategy is given (the
/// schema requires one) or when `kmeans` is given without `seed`;
/// [`CandidateError::InvalidParameter`] when both strategies are given, when
/// `seed` appears without `kmeans`, or when a value is malformed / out of
/// bounds.
fn clustering_from_spec(
    spec: &CandidateSpec,
) -> Result<(usize, ClusteringStrategy), CandidateError> {
    let grid = spec.param("grid");
    let kmeans = spec.param("kmeans");
    // Parse up front so a malformed seed is reported even when the strategy
    // rules below would also reject the id.
    let seed = spec.param_u64("seed")?;
    match (grid, kmeans) {
        (Some(_), Some(_)) => Err(invalid_param(
            "grid",
            grid.unwrap_or_default(),
            "exactly one of grid / kmeans is allowed, not both",
        )),
        (None, None) => Err(CandidateError::MissingParameter {
            algorithm: spec.algorithm.clone(),
            key: "grid".to_owned(),
        }),
        (Some(_), None) => {
            if seed.is_some() {
                return Err(invalid_param(
                    "seed",
                    spec.param("seed").unwrap_or_default(),
                    "seed requires kmeans; grid partitioning needs no seed",
                ));
            }
            let (rows, cols) = parse_grid(spec)?;
            Ok((rows.saturating_mul(cols), ClusteringStrategy::Grid { rows, cols }))
        }
        (None, Some(_)) => {
            let Some(seed) = seed else {
                return Err(CandidateError::MissingParameter {
                    algorithm: spec.algorithm.clone(),
                    key: "seed".to_owned(),
                });
            };
            let k = parse_kmeans(spec)?;
            Ok((k, ClusteringStrategy::KMeans { k, max_iters: KMEANS_MAX_ITERS, seed }))
        }
    }
}

/// Parse `grid=<rows>x<cols>` into `(rows, cols)`.
///
/// Grammar: two decimal integers separated by exactly one `x`
/// (`2x2`, `1x4`, `8x8`). Bounds: each dimension `1..=32`
/// ([`MAX_GRID_DIM`]) and `rows * cols <= 64` ([`MAX_REGIONS`]).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] on a value that does not match the
/// grammar or falls outside the bounds.
fn parse_grid(spec: &CandidateSpec) -> Result<(usize, usize), CandidateError> {
    let raw = spec.param("grid").unwrap_or_default();
    let mut parts = raw.split('x');
    let rows_raw = parts.next().unwrap_or_default();
    let cols_raw = parts.next().unwrap_or_default();
    let extra = parts.next();
    if rows_raw.is_empty() || cols_raw.is_empty() || extra.is_some() {
        return Err(invalid_param(
            "grid",
            raw,
            "expected <rows>x<cols> with a single 'x', e.g. 2x2",
        ));
    }
    let rows = parse_dim(rows_raw).ok_or_else(|| dim_error(raw))?;
    let cols = parse_dim(cols_raw).ok_or_else(|| dim_error(raw))?;
    if rows == 0 || cols == 0 {
        return Err(invalid_param("grid", raw, "rows and cols must each be >= 1"));
    }
    if rows > MAX_GRID_DIM || cols > MAX_GRID_DIM {
        return Err(invalid_param(
            "grid",
            raw,
            "rows and cols must each be <= 32 (contest MapSize::MAX)",
        ));
    }
    let regions = rows.saturating_mul(cols);
    if regions > MAX_REGIONS {
        return Err(invalid_param("grid", raw, "rows * cols must be <= 64 regions"));
    }
    Ok((rows, cols))
}

/// Parse `kmeans=<k>` into `k`.
///
/// The value is the bare number (`kmeans=4`), not a `k=4` binding — the
/// candidate-id grammar is already `key=value`, so nesting it would read as
/// `kmeans=k=4` and duplicate the key.
///
/// # Errors
/// [`CandidateError::InvalidParameter`] when the value is not a plain
/// integer or falls outside `1..=64` ([`MAX_REGIONS`]).
fn parse_kmeans(spec: &CandidateSpec) -> Result<usize, CandidateError> {
    let raw = spec.param("kmeans").unwrap_or_default();
    let k = raw
        .parse::<usize>()
        .map_err(|_| invalid_param("kmeans", raw, "expected a plain integer, e.g. kmeans=4"))?;
    if k == 0 {
        return Err(invalid_param("kmeans", raw, "k must be >= 1"));
    }
    if k > MAX_REGIONS {
        return Err(invalid_param("kmeans", raw, "k must be <= 64 regions"));
    }
    Ok(k)
}

/// Parse one grid dimension. `None` when `raw` is not a plain decimal
/// integer.
fn parse_dim(raw: &str) -> Option<usize> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse::<usize>().ok()
}

/// The error reported for a grid dimension that is not a decimal integer.
fn dim_error(value: &str) -> CandidateError {
    invalid_param("grid", value, "expected decimal integers, e.g. 2x2")
}

/// Parse `refine=<bool>`; absent means the composer's own default (`true`).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for anything other than `true` /
/// `false`.
fn parse_refine(spec: &CandidateSpec) -> Result<bool, CandidateError> {
    match spec.param("refine") {
        // Absent: keep the composer's own default (`MultiLoopComposer::new`
        // refines; `Default` also refines).
        None | Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(invalid_param("refine", other, "expected true | false")),
    }
}

/// Convenience constructor for [`CandidateError::InvalidParameter`].
fn invalid_param(key: &str, value: &str, reason: impl Into<String>) -> CandidateError {
    CandidateError::InvalidParameter {
        key: key.to_owned(),
        value: value.to_owned(),
        reason: reason.into(),
    }
}

/// Factory for [`MultiLoopCandidate`].
pub struct MultiLoopFactory;

impl CandidateFactory for MultiLoopFactory {
    fn algorithm(&self) -> &'static str {
        "multiloop"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn describe(&self) -> &'static str {
        "Region-clustering tour: partition spots (grid or k-means), build one tour per \
         region, hand regions to agents round-robin. A different mechanism from `loop`. \
         Supports refine=, walk= and assign= sweeps (assign=patrol withholds the tour \
         from Supply agents, Issue #235; assign=escort walks each Supply along its paired \
         Patrol's path, Issue #248; assign=all unions every agent's share into every \
         Patrol, Issue #253)."
    }

    fn validate_spec(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        clustering_from_spec(spec)?;
        parse_refine(spec)?;
        parse_walk(spec)?;
        parse_assign(spec)?;
        parse_budget(spec)?;
        Ok(())
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        MultiLoopCandidate::from_spec(spec).map(|c| Box::new(c) as Box<dyn Solver>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateRegistry;

    const VARIANTS: &[&str] = &[
        "multiloop:grid=2x2",
        "multiloop:grid=3x3",
        "multiloop:kmeans=4:seed=42",
        "multiloop:grid=2x2:refine=false",
    ];

    #[test]
    fn registry_round_trips_every_named_multiloop_variant() {
        let reg = CandidateRegistry::builtin();
        for id in VARIANTS {
            let candidate = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(candidate.id(), *id, "id must round-trip unchanged");
            // parse -> id must be stable too: the durable record stores ids.
            assert_eq!(CandidateSpec::parse(&candidate.id()).unwrap().id(), *id);
            let solver = candidate.build_solver().unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(solver.name(), "multiloop");
        }
    }

    #[test]
    fn catalog_lists_multiloop_params_in_canonical_order() {
        let cat = CandidateRegistry::builtin().catalog();
        let entry =
            cat.algorithms.iter().find(|a| a.algorithm == "multiloop").expect("multiloop listed");
        let keys: Vec<&str> = entry.params.iter().map(|p| p.key.as_str()).collect();
        // `walk` (#215), then `assign` (#235), then `budget` (#255):
        // each is appended, so older canonical ids keep their exact shape.
        assert_eq!(keys, ["grid", "kmeans", "seed", "refine", "walk", "assign", "budget"]);
    }

    #[test]
    fn grid_and_kmeans_variants_both_build_solvers() {
        let reg = CandidateRegistry::builtin();
        for id in [
            "multiloop:grid=2x2",
            "multiloop:grid=3x3",
            "multiloop:grid=1x1",
            "multiloop:grid=32x2",
            "multiloop:kmeans=4:seed=42",
            "multiloop:kmeans=1:seed=0",
            "multiloop:kmeans=64:seed=7:refine=false",
        ] {
            let candidate = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(candidate.build_solver().is_ok(), "{id} must build a solver");
        }
    }

    #[test]
    fn validate_spec_matrix_rejects_incoherent_combinations() {
        let reg = CandidateRegistry::builtin();
        // Neither strategy: the schema requires exactly one.
        assert!(matches!(reg.build("multiloop"), Err(CandidateError::MissingParameter { .. })));
        // Both strategies.
        assert!(matches!(
            reg.build("multiloop:grid=2x2:kmeans=4:seed=42"),
            Err(CandidateError::InvalidParameter { key, .. }) if key == "grid"
        ));
        // kmeans without its seed.
        assert!(matches!(
            reg.build("multiloop:kmeans=4"),
            Err(CandidateError::MissingParameter { key, .. }) if key == "seed"
        ));
        // seed without kmeans.
        assert!(matches!(
            reg.build("multiloop:grid=2x2:seed=42"),
            Err(CandidateError::InvalidParameter { key, .. }) if key == "seed"
        ));
        // Unknown parameter key.
        assert!(matches!(
            reg.build("multiloop:grid=2x2:beam=4"),
            Err(CandidateError::UnknownParameter { .. })
        ));
    }

    #[test]
    fn validate_spec_matrix_rejects_bad_values() {
        let reg = CandidateRegistry::builtin();
        for id in [
            // grid grammar
            "multiloop:grid=2",
            "multiloop:grid=2x",
            "multiloop:grid=x2",
            "multiloop:grid=2x2x2",
            "multiloop:grid=2y2",
            "multiloop:grid=axb",
            // grid bounds
            "multiloop:grid=0x2",
            "multiloop:grid=2x0",
            "multiloop:grid=33x1",
            "multiloop:grid=1x33",
            "multiloop:grid=9x9",
            // kmeans values (plain integer only — no k= binding)
            "multiloop:kmeans=0:seed=42",
            "multiloop:kmeans=65:seed=42",
            "multiloop:kmeans=k=4:seed=42",
            "multiloop:kmeans=many:seed=42",
            // refine values
            "multiloop:grid=2x2:refine=yes",
            "multiloop:grid=2x2:refine=1",
        ] {
            assert!(
                matches!(reg.build(id), Err(CandidateError::InvalidParameter { .. })),
                "{id} must be rejected as an invalid parameter"
            );
        }
    }

    #[test]
    fn validate_spec_matrix_rejects_non_numeric_seed() {
        let reg = CandidateRegistry::builtin();
        assert!(matches!(
            reg.build("multiloop:kmeans=4:seed=abc"),
            Err(CandidateError::InvalidParameter { key, .. }) if key == "seed"
        ));
    }

    // ------------------------------------------------------------------
    // behavioural: does `multiloop` actually partition the map per agent?
    // ------------------------------------------------------------------

    /// An 8×8 map with two spot clusters, one on each edge, so a `1x2` grid
    /// split has an unambiguous answer: left spots to agent 0, right spots to
    /// agent 1.
    fn two_cluster_state() -> engine::model::MatchState {
        use domain::{MatchId, PlayerId};
        use engine::model::{Agent, MapGrid, MapSize, MatchSetting, MatchState, Spot, Terrain};

        let size = MapSize::new(8, 8).unwrap();
        let clusters = [(0, 0), (0, 4), (7, 0), (7, 4)];
        let cells = (0..8)
            .flat_map(|row| {
                (0..8).map(move |col| {
                    let coord = engine::model::HexCoord::new(col, row);
                    let spot = clusters.contains(&(col, row)).then_some(Spot {
                        series: 1,
                        pos: coord,
                        max_stock: 1,
                        current_stock: 1,
                    });
                    engine::model::Cell { coord, terrain: Terrain::Plain, spot }
                })
            })
            .collect();
        let map = MapGrid::new(size, cells).unwrap();
        // Three agents because a match team must have 3..=8; only two
        // regions exist, so agent 2 is the documented surplus agent that
        // stays idle for the day.
        let agents = vec![
            Agent::patrol(0, engine::model::HexCoord::new(3, 3), 4, 3),
            Agent::supply(1, engine::model::HexCoord::new(4, 3)),
            Agent::supply(2, engine::model::HexCoord::new(4, 4)),
        ];
        MatchState::new(MatchSetting {
            match_id: MatchId::new("multiloop-partition").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents,
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn grid_partition_gives_each_agent_its_own_region() {
        let reg = CandidateRegistry::builtin();
        let candidate = reg.build("multiloop:grid=1x2").expect("valid candidate");
        let solver = candidate.build_solver().expect("builds");

        let state = two_cluster_state();
        let ctx = SolverContext::from_state(&state);

        // Exactly the path `decide` walks: live state -> problem ->
        // composer -> per-agent targets. Asserting on the targets (rather
        // than on step directions) tests the partition itself instead of the
        // walk's tie-breaking.
        let (num_regions, clustering) =
            clustering_from_spec(candidate.spec()).expect("spec clusters");
        let composer = MultiLoopComposer::new(num_regions, clustering);
        let outcome = composer.solve(&problem_from_state(&ctx)).expect("solve");
        let targets = agent_targets(&ctx, &outcome);

        assert!(!targets[0].is_empty(), "agent 0 must get a non-empty region");
        assert!(!targets[1].is_empty(), "agent 1 must get a non-empty region");
        assert!(
            targets[0].iter().all(|c| c.col <= 3),
            "agent 0 tours only the left region, got {:?}",
            targets[0]
        );
        assert!(
            targets[1].iter().all(|c| c.col >= 4),
            "agent 1 tours only the right region, got {:?}",
            targets[1]
        );
        assert!(targets[2].is_empty(), "surplus agent has no region, got {:?}", targets[2]);

        // And the adapter's own entry point produces a plan the engine will
        // accept.
        let plan = solver.decide(&ctx);
        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
    }

    #[test]
    fn kmeans_variant_partitions_into_disjoint_regions_deterministically() {
        let reg = CandidateRegistry::builtin();
        let candidate = reg.build("multiloop:kmeans=2:seed=42").expect("valid candidate");
        let state = two_cluster_state();
        let ctx = SolverContext::from_state(&state);
        let (num_regions, clustering) =
            clustering_from_spec(candidate.spec()).expect("spec clusters");
        let composer = MultiLoopComposer::new(num_regions, clustering);
        let first = composer.solve(&problem_from_state(&ctx)).expect("solve");
        let targets = agent_targets(&ctx, &first);

        // Where k-means lands depends on its seeded centroid stride, so this
        // asserts the partition *property* rather than a specific split:
        // every region is used, regions are disjoint, together they cover
        // every spot exactly once.
        assert!(!targets[0].is_empty() && !targets[1].is_empty(), "both regions must be used");
        assert!(targets[2].is_empty(), "surplus agent has no region");
        let mut seen: Vec<engine::model::HexCoord> = targets[0].clone();
        seen.extend(&targets[1]);
        seen.sort_by_key(|c| (c.row, c.col));
        let mut unique = seen.clone();
        unique.dedup();
        assert_eq!(unique.len(), 4, "all four spots visited exactly once, got {seen:?}");

        // Same id, same state, same answer — k-means is seeded and Lloyd's
        // iterations are capped, so the run is reproducible.
        let again = composer.solve(&problem_from_state(&ctx)).expect("solve");
        assert_eq!(first.plans, again.plans, "kmeans partition must be deterministic");
    }

    /// Issue #215 — `walk` is schema-gated and appended last on `multiloop`
    /// too: both legal values build inside canonical ids, any other value is
    /// `InvalidParameter`, and an unknown key still loses to the schema.
    #[test]
    fn walk_param_is_schema_validated() {
        let reg = CandidateRegistry::builtin();
        for id in [
            "multiloop:grid=2x2:walk=greedy",
            "multiloop:grid=2x2:walk=astar",
            "multiloop:kmeans=4:seed=42:walk=astar",
            "multiloop:grid=2x2:refine=false:walk=astar",
        ] {
            let candidate = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(candidate.id(), id, "canonical order must round-trip: {id}");
            assert!(candidate.build_solver().is_ok(), "{id} must build a solver");
        }
        for id in [
            "multiloop:grid=2x2:walk=nope",
            "multiloop:grid=2x2:walk=Astar",
            "multiloop:grid=2x2:walk=",
            "multiloop:kmeans=4:seed=42:walk=step",
        ] {
            assert!(
                matches!(
                    reg.build(id),
                    Err(CandidateError::InvalidParameter { .. }
                        | CandidateError::MissingParamValue(_))
                ),
                "{id} must be rejected"
            );
        }
        assert!(matches!(
            reg.build("multiloop:grid=2x2:walkmode=astar"),
            Err(CandidateError::UnknownParameter { .. })
        ));
    }

    /// Issue #215 AC — default equivalence: `walk=greedy` spelled out is
    /// byte-identical to the omitted parameter on a fixed live input, for
    /// both clustering strategies.
    #[test]
    fn walk_greedy_decide_is_byte_identical_to_the_omitted_default() {
        let reg = CandidateRegistry::builtin();
        let state = two_cluster_state();
        let ctx = SolverContext::from_state(&state);
        for (base, explicit) in [
            ("multiloop:grid=1x2", "multiloop:grid=1x2:walk=greedy"),
            ("multiloop:kmeans=4:seed=42", "multiloop:kmeans=4:seed=42:walk=greedy"),
        ] {
            let build = |id: &str| {
                reg.build(id)
                    .unwrap_or_else(|e| panic!("{id}: {e}"))
                    .build_solver()
                    .unwrap_or_else(|e| panic!("{id}: {e}"))
            };
            assert_eq!(
                build(base).decide(&ctx),
                build(explicit).decide(&ctx),
                "{explicit} must be byte-for-byte {base}"
            );
        }
    }

    /// `walk=astar` rides `multiloop`'s own `decide` path: a well-shaped day
    /// whose agents actually act (it is not a silent all-wait), so the A*
    /// walk is wired into the candidate rather than bypassing it.
    #[test]
    fn walk_astar_decide_is_well_shaped_and_actually_walks() {
        let reg = CandidateRegistry::builtin();
        let state = two_cluster_state();
        let ctx = SolverContext::from_state(&state);
        let solver = reg
            .build("multiloop:grid=1x2:walk=astar")
            .expect("candidate builds")
            .build_solver()
            .expect("solver builds");
        let plan = solver.decide(&ctx);
        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");
        assert_eq!(
            plan.actions.len(),
            ctx.agent_ids.len() * ctx.day_steps as usize,
            "one action per agent per step"
        );
        assert_ne!(
            plan,
            engine::model::AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps),
            "walk=astar must emit real movement, not waits"
        );
    }

    #[test]
    fn refine_false_builds_a_composer_without_refinement() {
        let spec = CandidateSpec::parse("multiloop:grid=2x2:refine=false").unwrap();
        let (num_regions, clustering) = clustering_from_spec(&spec).expect("spec clusters");
        assert!(!parse_refine(&spec).unwrap(), "refine=false must parse to false");
        let composer = MultiLoopComposer::without_refinement(num_regions, clustering);
        assert!(!composer.refine);

        let omitted = CandidateSpec::parse("multiloop:grid=2x2").unwrap();
        assert!(parse_refine(&omitted).unwrap(), "refine defaults to the composer default");
    }
}
