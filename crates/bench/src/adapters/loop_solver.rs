//! `loop` candidate — the iterative-improvement loop solver from
//! `crates/solver-loop`.
//!
//! Parameterised so a whole axis of candidates is just different ids:
//!
//! ```text
//! loop                                  (defaults: iterations=50, initial=greedy, operators=classic)
//! loop:iterations=32:initial=greedy
//! loop:iterations=128:initial=greedy
//! loop:iterations=64:initial=random:seed=42
//! loop:initial=sweep-row
//! loop:initial=sweep-col
//! loop:iterations=32:initial=greedy:operators=extended
//! loop:initial=reverse
//! ```
//!
//! `operators=classic` (the default) is the pre-#191 move set, byte for
//! byte; `operators=extended` adds all seven classical 3-opt moves plus
//! or-opt (segment relocation ≤ 3) on top of it. The parameter is appended
//! last in [`PARAMS`] so existing canonical ids keep their shape.
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
    Agent as LoopAgent, HexCoord as LoopCoord, InitialStrategy, LoopSolver, OperatorSet,
    Series as LoopSeries, SolveProblem, Solver as _, Spot as LoopSpot,
};

use super::walk::{plan_walk, step_towards};
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};

/// Parameter schema, in canonical id order.
///
/// `operators` is appended after `seed` (Issue #191) so that every
/// pre-existing canonical id — `loop:iterations=32:initial=greedy`,
/// `loop:iterations=64:initial=random:seed=42`, … — keeps both its bytes
/// and its meaning (omitted `operators` selects `classic`).
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
    ParamSpec {
        key: "operators",
        required: false,
        doc: "refinement operator set: classic (default; pre-#191 move set) | extended (7-move 3-opt + or-opt)",
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
        let operators = parse_operators(spec)?;
        Ok(Self {
            inner: LoopSolver::with_operators(initial, iterations, operators),
            id: spec.id(),
        })
    }

    /// The per-agent target lists this candidate will walk.
    ///
    /// `operators=classic` (and therefore every default candidate id) takes
    /// the direct path and returns [`agent_targets`] unchanged — Issue #207
    /// must not perturb a single byte of the pre-existing behaviour. Only
    /// `operators=extended` may be re-anchored, and only when its default
    /// anchor would reach no spot at all; see [`rescue_anchor`].
    fn targets_for(
        &self,
        ctx: &SolverContext<'_>,
        outcome: &solver_loop::SolveOutcome,
    ) -> Vec<Vec<HexCoord>> {
        let per_agent = agent_targets(ctx, outcome);
        if self.inner.operators != OperatorSet::Extended
            || agents_reaching_a_spot(ctx, &per_agent) > 0
        {
            return per_agent;
        }
        rescue_anchor(ctx, outcome, &per_agent)
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
                let per_agent = self.targets_for(ctx, &outcome);
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

// ---------------------------------------------------------------------------
// Issue #207 — reachability probe + anchor rescue (`operators=extended` only)
// ---------------------------------------------------------------------------

/// How many controlled agents would stand on a spot cell at least once if
/// [`plan_walk`] walked `targets` for this day.
///
/// This replays `plan_walk`'s own position bookkeeping *exactly*: same live
/// start positions, same `step % targets.len()` target cycling, same
/// [`step_towards`]. Consequently the count is `0` **if and only if** no
/// agent can collect anything from this target assignment on this day —
/// `engine::MatchSim` only acquires when an agent ends a step on a spot.
///
/// It is a read-only probe: it never mutates the state and it is never used
/// to pick a *better* assignment when the default one already works (see
/// [`rescue_anchor`]).
fn agents_reaching_a_spot(ctx: &SolverContext<'_>, targets: &[Vec<HexCoord>]) -> usize {
    let map = &ctx.state.setting.map;
    let day_steps = ctx.day_steps as usize;
    ctx.agent_ids
        .iter()
        .enumerate()
        .filter(|(i, agent_id)| {
            let Some(list) = targets.get(*i).filter(|l| !l.is_empty()) else {
                return false;
            };
            let Some(start) = ctx.state.agents.iter().find(|a| a.id == **agent_id) else {
                return false;
            };
            let mut pos = start.pos;
            for step in 0..day_steps {
                if let Some(dir) = step_towards(pos, list[step % list.len()], map) {
                    pos = dir.step(pos);
                }
                if map.get(pos).is_some_and(|c| c.spot.is_some()) {
                    return true;
                }
            }
            false
        })
        .count()
}

/// Recover the refined closed tour from a `days = 1` solve outcome.
///
/// [`solver_loop`] hands back the tour already round-robined across agents
/// (`plans[0][a] = tour[a], tour[a + n], …`). Interleaving the per-agent
/// lists row-major undoes that exactly. Returns `None` when the shape is not
/// a clean round-robin, so a caller can never silently re-anchor a plan it
/// does not understand.
fn tour_from_outcome(outcome: &solver_loop::SolveOutcome, n_agents: usize) -> Option<Vec<u32>> {
    let day = outcome.plans.first()?;
    if n_agents == 0 || day.len() != n_agents {
        return None;
    }
    let max = day.iter().map(Vec::len).max()?;
    let min = day.iter().map(Vec::len).min()?;
    if max.saturating_sub(min) > 1 {
        return None;
    }
    let mut tour: Vec<u32> = Vec::with_capacity(day.iter().map(Vec::len).sum());
    for row in 0..max {
        for list in day {
            tour.extend(list.get(row));
        }
    }
    (!tour.is_empty()).then_some(tour)
}

/// Per-agent target lists for `tour` under `distribute_plan`'s index
/// round-robin (agent `a` gets `tour[a], tour[a + n], …`).
fn targets_for_tour(ctx: &SolverContext<'_>, tour: &[u32], n_agents: usize) -> Vec<Vec<HexCoord>> {
    let map = &ctx.state.setting.map;
    (0..n_agents)
        .map(|a| {
            let ids: Vec<u32> = tour.iter().skip(a).step_by(n_agents).copied().collect();
            coords_for(&ids, map)
        })
        .collect()
}

/// Issue #207 — re-anchor the closed tour when the default anchor would
/// collect **nothing**.
///
/// # Why this exists
///
/// `distribute_plan` hands out the tour by *array index* (`spot i` goes to
/// `agent i % n`), and `plan_walk` switches target on every *step*
/// (`step % len`). Both are index-driven, but collection is geometry-driven:
/// an agent that alternates between two targets only ever stands on a spot
/// when its `(start, target-pair)` happens to work out. Rotating or
/// reversing the same closed tour leaves its edge set — and therefore the
/// refinement objective — untouched, yet reassigns every agent's target
/// pair. `operators=extended`'s extra 3-opt moves rebuild the array in a
/// different orientation/anchor than `classic`'s, and on `road-heavy-v2`
/// that default anchor puts all five agents into a stall, so not one of them
/// ever reaches a spot (Issue #207).
///
/// # What this does
///
/// Only when the default anchor would reach *zero* spots, scan the `2n`
/// equivalent anchors (`n` rotations of the tour, plus `n` rotations of its
/// reversal), simulate each with [`agents_reaching_a_spot`] and keep the
/// first strictly better one (ties keep the incumbent, so the scan is a pure
/// rescue and never rewrites an assignment that already works).
///
/// # Why `operators=classic` is safe
///
/// [`LoopCandidate::targets_for`] never calls this for `classic` (or for any
/// default candidate), so their target lists — and therefore their actions,
/// byte for byte — are unchanged. For `extended` the probe fires only when
/// the default assignment stands on no spot at all that day, i.e. when that
/// day already scores `0`; the scan starts from the default anchor and only
/// ever replaces it with a strictly more reachable one, so it can never
/// rewrite a day that was already collecting.
fn rescue_anchor(
    ctx: &SolverContext<'_>,
    outcome: &solver_loop::SolveOutcome,
    default: &[Vec<HexCoord>],
) -> Vec<Vec<HexCoord>> {
    let n_agents = ctx.agent_ids.len();
    let Some(tour) = tour_from_outcome(outcome, n_agents) else {
        return default.to_vec();
    };

    let mut best = default.to_vec();
    let mut best_hits = agents_reaching_a_spot(ctx, &best);
    if best_hits == n_agents {
        return best;
    }

    let mut anchor = tour.clone();
    for reversed in [false, true] {
        if reversed {
            anchor = tour.iter().rev().copied().collect();
        }
        for r in 0..anchor.len() {
            if !reversed && r == 0 {
                continue; // == the default anchor already measured above
            }
            if r > 0 {
                anchor.rotate_left(1);
            }
            let candidate = targets_for_tour(ctx, &anchor, n_agents);
            let hits = agents_reaching_a_spot(ctx, &candidate);
            if hits > best_hits {
                best_hits = hits;
                best = candidate;
                if best_hits == n_agents {
                    return best;
                }
            }
        }
    }
    best
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

/// Parse the `operators` parameter into an [`OperatorSet`].
///
/// Omitted means `classic`: the pre-#191 move set, so existing ids keep
/// their exact behaviour (Issue #191).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any value other than
/// `classic` / `extended`.
fn parse_operators(spec: &CandidateSpec) -> Result<OperatorSet, CandidateError> {
    match spec.param("operators") {
        Some("extended") => Ok(OperatorSet::Extended),
        Some("classic") | None => Ok(OperatorSet::Classic),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "operators".to_owned(),
            value: other.to_owned(),
            reason: "expected classic | extended".to_owned(),
        }),
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
         then walk the visit order. Supports initial=, iterations= and operators= sweeps."
    }

    fn validate_spec(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        parse_initial(spec)?;
        parse_operators(spec)?;
        Ok(())
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

    /// Issue #191: `operators` is a schema-gated parameter. Both legal
    /// values build (including inside full canonical ids, with `operators`
    /// written last); anything else is rejected as
    /// `CandidateError::InvalidParameter` at registry-build time.
    #[test]
    fn operators_param_is_schema_validated() {
        let reg = CandidateRegistry::builtin();
        for id in [
            "loop:operators=classic",
            "loop:operators=extended",
            "loop:iterations=32:initial=greedy:operators=classic",
            "loop:iterations=32:initial=greedy:operators=extended",
            "loop:initial=sweep-row:operators=extended",
            "loop:iterations=64:initial=random:seed=42:operators=extended",
        ] {
            assert!(reg.build(id).is_ok(), "{id} must be a valid candidate");
        }
        for id in [
            "loop:operators=bogus",
            "loop:operators=Classic",
            "loop:operators=",
            "loop:iterations=32:initial=greedy:operators=none",
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
        // Unknown keys still lose to the schema before value validation.
        assert!(matches!(
            reg.build("loop:operator=extended"),
            Err(CandidateError::UnknownParameter { .. })
        ));
    }

    /// Issue #207 — minimal repro: same `SolveProblem`, two different tours,
    /// only one of which the walk can actually use.
    ///
    /// `problem_from_state` builds the exact problem the adapter solves for
    /// `fixture:road-heavy-v2` (10 spots, 5 agents, `days = 1`, 40 steps).
    /// The two operator sets refine it into two different *array* orders of
    /// a closed tour; `distribute_plan` hands those out by array index
    /// (`spot i` → `agent i % 5`), so each agent gets a different pair of
    /// targets. `plan_walk` then switches target on every step, so an agent
    /// only ever stands on a spot when its `(start, target pair)` works out
    /// geometrically:
    ///
    /// * `classic` — two agents reach a spot, so the day scores;
    /// * `extended` — **not one** agent reaches one, so the day would score
    ///   `(0, 0, 0)` if [`LoopCandidate::targets_for`] did not re-anchor it.
    #[test]
    fn issue207_minimal_repro_extended_default_anchor_reaches_no_spot() {
        let ctx = road_heavy_v2_context();
        let problem = problem_from_state(&ctx);
        assert_eq!(problem.days, 1, "the adapter re-solves one day at a time");

        let mut outcomes = Vec::new();
        for ops in [OperatorSet::Classic, OperatorSet::Extended] {
            let solver =
                LoopSolver::with_operators(InitialStrategy::GreedyNearestNeighbor, 32, ops);
            let outcome = solver.solve(&problem).expect("solve");
            let tour =
                tour_from_outcome(&outcome, problem.agents.len()).expect("clean round-robin");
            let reached = agents_reaching_a_spot(&ctx, &agent_targets(&ctx, &outcome));
            println!("{ops:?}: reachable_agents={reached}/{} tour={tour:?}", problem.agents.len());
            outcomes.push((tour, outcome));
        }

        let (classic_tour, classic_outcome) = &outcomes[0];
        let (extended_tour, extended_outcome) = &outcomes[1];
        let classic_hits = agents_reaching_a_spot(&ctx, &agent_targets(&ctx, classic_outcome));
        let extended_hits = agents_reaching_a_spot(&ctx, &agent_targets(&ctx, extended_outcome));

        // The two operator sets must actually disagree about the array —
        // otherwise there would be no defect to rescue.
        assert_ne!(
            classic_tour, extended_tour,
            "extended must refine to a different tour array than classic for \
             the Issue #207 repro to exist"
        );
        // Control: the probe and the walk model are sound — classic's default
        // anchor is walkable on the very same problem.
        assert!(
            classic_hits >= 1,
            "classic's default anchor must reach a spot on road-heavy-v2 \
             (probe control), got {classic_hits}"
        );
        // The defect: extended's default anchor reaches nothing.
        assert_eq!(
            extended_hits, 0,
            "Issue #207: extended's default anchor is the zero-scoring one; \
             got {extended_hits} reachable agents"
        );

        // …and the rescue must re-anchor it to something walkable.
        let rescued = extended_candidate().targets_for(&ctx, extended_outcome);
        assert!(
            agents_reaching_a_spot(&ctx, &rescued) >= 1,
            "the Issue #207 rescue must re-anchor extended to a walkable target set"
        );
    }

    /// The rescue is *scoped*: `operators=classic` (and therefore every
    /// default `loop:…` id) never has its target list touched, so its
    /// actions stay byte-for-byte what they were before Issue #207.
    #[test]
    fn issue207_rescue_is_scoped_to_extended() {
        let ctx = road_heavy_v2_context();
        let problem = problem_from_state(&ctx);

        for id in ["loop:iterations=32:initial=greedy", CLASSIC_ID] {
            let candidate = loop_candidate(id);
            let solver = LoopSolver::with_operators(
                InitialStrategy::GreedyNearestNeighbor,
                32,
                candidate.inner.operators,
            );
            let outcome = solver.solve(&problem).expect("solve");
            assert_eq!(
                candidate.targets_for(&ctx, &outcome),
                agent_targets(&ctx, &outcome),
                "candidate {id} must keep its plain target list byte for byte"
            );
        }

        // Extended keeps its default anchor whenever that anchor already
        // reaches a spot — the rescue is a rescue, not a re-optimisation.
        let candidate = loop_candidate(EXTENDED_ID);
        let solver = LoopSolver::with_operators(
            InitialStrategy::GreedyNearestNeighbor,
            32,
            candidate.inner.operators,
        );
        let mut outcome = solver.solve(&problem).expect("solve");
        let default = agent_targets(&ctx, &outcome);
        if agents_reaching_a_spot(&ctx, &default) > 0 {
            assert_eq!(
                candidate.targets_for(&ctx, &outcome),
                default,
                "extended must keep a working default anchor"
            );
        }
        // A plan shape the adapter cannot decode round-robins falls through
        // untouched instead of being re-anchored on a guess.
        outcome.plans = vec![vec![Vec::new(); problem.agents.len()]];
        assert_eq!(
            candidate.targets_for(&ctx, &outcome),
            agent_targets(&ctx, &outcome),
            "an unrecognisable plan shape must fall through untouched"
        );
    }

    const CLASSIC_ID: &str = "loop:iterations=32:initial=greedy:operators=classic";
    const EXTENDED_ID: &str = "loop:iterations=32:initial=greedy:operators=extended";

    /// Live `fixture:road-heavy-v2` context as the adapter sees it.
    ///
    /// The `MatchState` is leaked so the borrowed context can outlive this
    /// helper; a unit-test process exits right after, so the trade is safe
    /// and it keeps the signature simple.
    fn road_heavy_v2_context() -> SolverContext<'static> {
        use crate::case::CaseSpec;
        use engine::model::MatchState;

        let case = CaseSpec::from_bundled_fixture("road-heavy-v2").expect("bundled fixture loads");
        let team = domain::PlayerId::new("player-a").expect("valid team id");
        let setting = case.to_engine_setting(&team).expect("case converts");
        let state: &'static MatchState =
            Box::leak(Box::new(MatchState::new(setting).expect("validated setting")));
        SolverContext::from_state(state)
    }

    fn loop_candidate(id: &str) -> LoopCandidate {
        LoopCandidate::from_spec(&CandidateSpec::parse(id).expect("spec parses"))
            .expect("candidate builds")
    }

    fn extended_candidate() -> LoopCandidate {
        loop_candidate(EXTENDED_ID)
    }

    #[test]
    fn spot_ids_round_trip_through_coordinates() {
        for (col, row) in [(0, 0), (3, 7), (31, 31), (12, 4)] {
            let c = HexCoord::new(col, row);
            assert_eq!(coord_for_spot_id(spot_id_for_coord(c)), Some(c));
        }
    }
}
