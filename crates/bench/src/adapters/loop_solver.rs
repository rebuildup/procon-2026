//! `loop` candidate — the iterative-improvement loop solver from
//! `crates/solver-loop`.
//!
//! Parameterised so a whole axis of candidates is just different ids:
//!
//! ```text
//! loop                                  (defaults: iterations=50, initial=greedy)
//! loop:iterations=32:initial=greedy
//! loop:iterations=128:initial=greedy
//! loop:iterations=64:initial=random:seed=42
//! loop:initial=sweep-row
//! loop:initial=sweep-col
//! loop:initial=reverse
//! ```
//!
//! Every `initial=` value here has exactly one live-CLI counterpart
//! (`procon tournament|coordinator --loop-initial-strategy <label>`); the
//! correspondence table lives in `crates/cli/src/initial_strategy.rs`.
//!
//! # How the adapter works
//!
//! `solver_loop::LoopSolver` solves a *touring* problem: given spots and
//! agents it returns `plans[day][agent] = spot visit order`. The contest,
//! though, asks for one day's worth of concrete agent actions.
//!
//! This adapter re-solves the touring problem **once per day** from the live
//! [`engine::model::MatchState`] (so it sees where its agents actually ended
//! up and which spots are still stocked today) and then walks each agent
//! along its visit order with [`super::walk::plan_walk`].
//!
//! One day per solve is deliberate and matches the contest: spot stock
//! refills at every day boundary (募集要項), so each day wants a fresh tour of
//! the whole map rather than a slice of a multi-day plan computed from stale
//! positions. It is also what keeps `initial=` / `iterations=` meaningful —
//! the refinement runs on the day's actual problem.

use engine::model::{AgentActions, HexCoord, Solver, SolverContext};
use solver_loop::{
    Agent as LoopAgent, HexCoord as LoopCoord, InitialStrategy, LoopSolver, Series as LoopSeries,
    SolveProblem, Solver as _, Spot as LoopSpot,
};

use super::walk::plan_walk;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};

/// Parameter schema, in canonical id order.
pub const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "iterations",
        required: false,
        doc: "2-opt / 3-opt improvement iteration cap (u32, default 50)",
    },
    ParamSpec {
        key: "initial",
        required: false,
        doc: "initial tour strategy: greedy | identity | reverse | sweep-row | sweep-col | random (default greedy)",
    },
    ParamSpec {
        key: "seed",
        required: false,
        doc: "RNG seed for initial=random (u64); required when initial=random",
    },
];

/// A [`LoopSolver`] configured from a candidate spec, driving one day at a
/// time against the live match state.
pub struct LoopCandidate {
    inner: LoopSolver,
    /// The candidate id, echoed into solver naming for logs.
    id: String,
}

impl LoopCandidate {
    /// Build from an already-validated spec.
    fn from_spec(spec: &CandidateSpec) -> Result<Self, CandidateError> {
        let iterations = spec.param_u32("iterations")?.unwrap_or(50);
        let initial = parse_initial(spec)?;
        Ok(Self { inner: LoopSolver::new(initial, iterations), id: spec.id() })
    }
}

impl Solver for LoopCandidate {
    fn name(&self) -> &'static str {
        // `Solver::name` is a compile-time constant; per-run parameters live
        // in the candidate id, which the evaluator records separately.
        "loop"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let problem = problem_from_state(ctx);
        match self.inner.solve(&problem) {
            Ok(outcome) => {
                let per_agent = agent_targets(ctx, &outcome);
                plan_walk(ctx, &per_agent)
            }
            Err(err) => {
                tracing::debug!(candidate = %self.id, error = %err, "loop solve failed; waiting");
                AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
            }
        }
    }
}

/// Translate the live match state into the loop solver's problem.
///
/// Spot ids are synthesised from coordinates (`row * 1_000_000 + col`) so
/// they round-trip back to a coordinate exactly. That scheme is unique for
/// every contest map (`MapSize::MAX = 32`) and matches the convention used by
/// the CLI's wire-path adapter.
///
/// `pub(crate)` because every tour-shaped adapter (`loop`, `multiloop`) needs
/// the *same* spot-id ↔ coordinate round-trip: two schemes would silently
/// misroute visits. See `super::multiloop_solver`.
pub(crate) fn problem_from_state(ctx: &SolverContext<'_>) -> SolveProblem {
    let map = &ctx.state.setting.map;
    let spots: Vec<LoopSpot> = map
        .cells()
        .iter()
        .filter_map(|cell| {
            let spot = cell.spot.as_ref()?;
            Some(LoopSpot {
                id: spot_id_for_coord(cell.coord),
                coord: LoopCoord::new(cell.coord.col, cell.coord.row),
                series: LoopSeries::new(format!("series-{}", spot.series)),
                stock: spot.current_stock,
            })
        })
        .collect();
    let agents: Vec<LoopAgent> = ctx
        .agent_ids
        .iter()
        .filter_map(|id| {
            let a = ctx.state.agents.iter().find(|a| a.id == *id)?;
            Some(LoopAgent { id: u32::from(*id), start: LoopCoord::new(a.pos.col, a.pos.row) })
        })
        .collect();
    // One day at a time — see the module docs.
    SolveProblem::new(spots, agents, 1)
}

/// Convert the loop solver's per-agent spot-id visit order into map
/// coordinates, one target list per controlled agent (in `ctx.agent_ids`
/// order).
///
/// `pub(crate)` for the same reason as [`problem_from_state`]: `multiloop`
/// reuses this exact target/`plan_walk` path so the only difference between
/// the two candidates is the tour construction, not the day contract.
pub(crate) fn agent_targets(
    ctx: &SolverContext<'_>,
    outcome: &solver_loop::SolveOutcome,
) -> Vec<Vec<HexCoord>> {
    let map = &ctx.state.setting.map;
    ctx.agent_ids
        .iter()
        .enumerate()
        .map(|(agent_idx, _)| {
            outcome
                .plans
                .first()
                .and_then(|day| day.get(agent_idx))
                .map_or_else(Vec::new, |ids| coords_for(ids, map))
        })
        .collect()
}

/// Map loop-solver spot ids back to live coordinates, dropping unknown ids.
fn coords_for(ids: &[u32], map: &engine::model::MapGrid) -> Vec<HexCoord> {
    ids.iter()
        .filter_map(|id| {
            let coord = coord_for_spot_id(*id)?;
            map.get(coord).filter(|c| c.spot.is_some()).map(|_| coord)
        })
        .collect()
}

/// Stable per-cell id for the loop solver's visit order.
fn spot_id_for_coord(coord: HexCoord) -> u32 {
    let id = i64::from(coord.row) * 1_000_000 + i64::from(coord.col);
    u32::try_from(id).unwrap_or(0)
}

/// Inverse of [`spot_id_for_coord`].
fn coord_for_spot_id(spot_id: u32) -> Option<HexCoord> {
    let id = i64::from(spot_id);
    let row = i32::try_from(id / 1_000_000).ok()?;
    let col = i32::try_from(id.rem_euclid(1_000_000)).ok()?;
    Some(HexCoord::new(col, row))
}

/// Parse the `initial` parameter into an [`InitialStrategy`].
fn parse_initial(spec: &CandidateSpec) -> Result<InitialStrategy, CandidateError> {
    let bad = |reason: &str| CandidateError::InvalidParameter {
        key: "initial".to_owned(),
        value: spec.param("initial").unwrap_or_default().to_owned(),
        reason: reason.to_owned(),
    };
    let seed_param = spec.param_u64("seed")?;
    match spec.param("initial") {
        None => {
            if seed_param.is_some() {
                return Err(bad("seed requires initial=random"));
            }
            Ok(InitialStrategy::GreedyNearestNeighbor)
        }
        Some("greedy") => Ok(InitialStrategy::GreedyNearestNeighbor),
        Some("identity") => Ok(InitialStrategy::Identity),
        Some("reverse") => Ok(InitialStrategy::Reverse),
        Some("sweep-row") => Ok(InitialStrategy::SweepRow),
        Some("sweep-col") => Ok(InitialStrategy::SweepCol),
        Some("random") => match seed_param {
            Some(seed) => Ok(InitialStrategy::Random { seed }),
            None => Err(CandidateError::MissingParameter {
                algorithm: spec.algorithm.clone(),
                key: "seed".to_owned(),
            }),
        },
        Some(_) => {
            Err(bad("expected greedy | identity | reverse | sweep-row | sweep-col | random"))
        }
    }
}

/// Factory for [`LoopCandidate`].
pub struct LoopFactory;

impl CandidateFactory for LoopFactory {
    fn algorithm(&self) -> &'static str {
        "loop"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn describe(&self) -> &'static str {
        "Iterative-improvement loop tour (2-opt / 3-opt) over the day's spots, \
         then walk the visit order. Supports initial= and iterations= sweeps."
    }

    fn validate_spec(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        parse_initial(spec).map(|_| ())
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        LoopCandidate::from_spec(spec).map(|c| Box::new(c) as Box<dyn Solver>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateRegistry;

    #[test]
    fn factory_accepts_every_named_loop_variant() {
        let reg = CandidateRegistry::builtin();
        for id in [
            "loop",
            "loop:iterations=32:initial=greedy",
            "loop:iterations=128:initial=greedy",
            "loop:iterations=64:initial=random:seed=42",
            "loop:initial=sweep-row",
            "loop:initial=sweep-col",
            "loop:initial=reverse",
        ] {
            assert!(reg.build(id).is_ok(), "{id} must be a valid candidate");
        }
    }

    /// `loop:initial=reverse` is a stable candidate id that selects
    /// `InitialStrategy::Reverse` — the offline half of the live parity
    /// table in `crates/cli/src/initial_strategy.rs`.
    #[test]
    fn initial_reverse_maps_to_reverse_strategy() {
        let spec = CandidateSpec::parse("loop:initial=reverse").expect("parse");
        assert_eq!(parse_initial(&spec), Ok(InitialStrategy::Reverse));
        // With iterations it stays the same strategy (a separate axis).
        let spec = CandidateSpec::parse("loop:initial=reverse:iterations=7").expect("parse");
        assert_eq!(parse_initial(&spec), Ok(InitialStrategy::Reverse));
        // The id round-trips unchanged, as every candidate id must.
        assert_eq!(spec.id(), "loop:initial=reverse:iterations=7");
    }

    #[test]
    fn initial_parsing_rejects_incoherent_combinations() {
        let reg = CandidateRegistry::builtin();
        assert!(matches!(
            reg.build("loop:initial=random"),
            Err(CandidateError::MissingParameter { .. })
        ));
        assert!(matches!(reg.build("loop:seed=1"), Err(CandidateError::InvalidParameter { .. })));
        assert!(matches!(
            reg.build("loop:initial=nope"),
            Err(CandidateError::InvalidParameter { .. })
        ));
    }

    #[test]
    fn spot_ids_round_trip_through_coordinates() {
        for (col, row) in [(0, 0), (3, 7), (31, 31), (12, 4)] {
            let c = HexCoord::new(col, row);
            assert_eq!(coord_for_spot_id(spot_id_for_coord(c)), Some(c));
        }
    }
}
