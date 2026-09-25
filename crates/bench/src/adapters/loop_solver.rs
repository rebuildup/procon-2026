//! `loop` candidate — the iterative-improvement loop solver from
//! `crates/solver-loop`.
//!
//! Parameterised so a whole axis of candidates is just different ids:
//!
//! ```text
//! loop                                  (defaults: iterations=50, initial=greedy,
//!                                        lookahead=none)
//! loop:iterations=32:initial=greedy
//! loop:iterations=128:initial=greedy
//! loop:iterations=64:initial=random:seed=42
//! loop:initial=sweep-row
//! loop:initial=sweep-col
//! loop:iterations=32:initial=greedy:lookahead=crossday
//! ```
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
//!
//! # `lookahead` — the multi-day variant (Issue #193)
//!
//! The `lookahead` param switches how many days a single solve plans for
//! (Issue #193), while keeping the "re-solve from live state every day"
//! contract above:
//!
//! - **`lookahead=none`** (default) — `SolveProblem::days = 1`: today only,
//!   exactly the behaviour above. Existing ids keep it implicitly, so the
//!   default path is byte-for-byte the pre-`lookahead` behaviour.
//! - **`lookahead=crossday`** — `SolveProblem.days = remaining days` of the
//!   match: the solve plans from *today* through the last day
//!   (`days = num_days − day`, see [`remaining_days`]), so the outcome
//!   carries one plan leg per remaining day, and the adapter walks **the
//!   current day's leg**. Positions and stock are still re-read every day,
//!   so tomorrow's solve starts from where the agents actually ended up.
//!
//! The multi-day leg selection inside `solver_loop` is the crate's dormant
//! round-robin path (`distribute_plan` splits the tour across
//! `(day, agent)`); the adapter drives it purely from the outside by
//! passing a larger `days` — `crates/solver-loop` itself is untouched.
//!
//! Note that the refinement (`initial_tour` + 2-opt / 3-opt) scores with an
//! internal one-day problem regardless of `SolveProblem.days`, so both
//! modes hill-climb the *same* tour; `lookahead` only changes how that tour
//! is distributed over days and agents.

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
        doc: "initial tour strategy: greedy | identity | sweep-row | sweep-col | random (default greedy)",
    },
    ParamSpec {
        key: "seed",
        required: false,
        doc: "RNG seed for initial=random (u64); required when initial=random",
    },
    ParamSpec {
        key: "lookahead",
        required: false,
        doc: "multi-day planning horizon: none | crossday (default none)",
    },
];

/// How many days one solve plans for (the `lookahead` param, Issue #193).
///
/// Both modes keep the adapter's day contract: re-solve from the live
/// [`SolverContext`] every day and walk the plan leg for *today*.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Lookahead {
    /// `lookahead=none` (default): solve for today only (`days = 1`) — the
    /// pre-#193 behaviour, byte for byte.
    #[default]
    None,
    /// `lookahead=crossday`: solve for the remaining days of the match
    /// (`days = num_days − day`) and walk today's leg of that plan.
    CrossDay,
}

impl Lookahead {
    /// The `SolveProblem.days` this mode passes to the solver, derived from
    /// the live context.
    fn plan_days(self, ctx: &SolverContext<'_>) -> u32 {
        match self {
            Self::None => 1,
            Self::CrossDay => remaining_days(ctx),
        }
    }
}

/// Days of the match left to play, derived from the live [`SolverContext`].
///
/// This is the derivation `lookahead=crossday` documents as "solve with the
/// remaining days of the match":
///
/// - **total days** — `ctx.state.num_days()` (`MatchState::num_days` →
///   `MatchSetting::num_days`, the length of the official `daySteps`
///   vector).
/// - **day index** — `ctx.day` (the context's 0-based current day; it is
///   mirrored from `MatchState::day` by [`SolverContext::from_state`]).
///
/// Clamped to at least `1` so a context whose day already reached
/// `num_days` still plans a single day instead of failing the solve with
/// [`solver_loop::SolverError::ZeroDays`].
fn remaining_days(ctx: &SolverContext<'_>) -> u32 {
    ctx.state.num_days().saturating_sub(ctx.day).max(1)
}

/// Index into `outcome.plans` of the current day's leg.
///
/// Each mode anchors its solve at the *live* day: `none` plans exactly
/// `ctx.day`, `crossday` plans `ctx.day .. num_days`. Leg `i` of the
/// outcome therefore covers match day `anchor_day + i`, so the current
/// day's leg — Issue #193's `outcome.plans[current_day]`, re-expressed
/// against a plan anchored at the live day — is
/// `i = current_day − anchor_day`, which is `0` for every mode and every
/// day. Tomorrow re-anchors at `current_day + 1` with a fresh solve from
/// the then-live state.
const CURRENT_DAY_LEG: usize = 0;

/// A [`LoopSolver`] configured from a candidate spec, driving one day at a
/// time against the live match state.
pub struct LoopCandidate {
    inner: LoopSolver,
    /// The candidate id, echoed into solver naming for logs.
    id: String,
    /// Multi-day planning horizon (`lookahead`).
    lookahead: Lookahead,
}

impl LoopCandidate {
    /// Build from an already-validated spec.
    fn from_spec(spec: &CandidateSpec) -> Result<Self, CandidateError> {
        let iterations = spec.param_u32("iterations")?.unwrap_or(50);
        let initial = parse_initial(spec)?;
        let lookahead = parse_lookahead(spec)?;
        Ok(Self { inner: LoopSolver::new(initial, iterations), id: spec.id(), lookahead })
    }

    /// Solve once from the live state and return **today's** per-agent
    /// target lists — the exact path [`Solver::decide`] walks.
    ///
    /// Exposed to the module's tests so they can assert which leg of a
    /// multi-day outcome the adapter follows (Issue #193's AC), without
    /// duplicating the `decide` plumbing.
    fn day_targets(
        &self,
        ctx: &SolverContext<'_>,
    ) -> Result<Vec<Vec<HexCoord>>, solver_loop::SolverError> {
        let problem = problem_from_days(ctx, self.lookahead.plan_days(ctx));
        let outcome = self.inner.solve(&problem)?;
        Ok(agent_targets_on_day(ctx, &outcome, CURRENT_DAY_LEG))
    }
}

impl Solver for LoopCandidate {
    fn name(&self) -> &'static str {
        // `Solver::name` is a compile-time constant; per-run parameters live
        // in the candidate id, which the evaluator records separately.
        "loop"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        match self.day_targets(ctx) {
            Ok(targets) => plan_walk(ctx, &targets),
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
///
/// One day at a time — see the module docs. `lookahead=crossday` keeps the
/// same translation and only widens the horizon via [`problem_from_days`].
pub(crate) fn problem_from_state(ctx: &SolverContext<'_>) -> SolveProblem {
    problem_from_days(ctx, 1)
}

/// [`problem_from_state`] with an explicit planning horizon.
///
/// `days` is [`Lookahead::plan_days`]'s answer: `1` for `lookahead=none`,
/// the match's remaining days for `lookahead=crossday`. Everything else —
/// spots, stocks, live agent positions — is translated exactly as
/// [`problem_from_state`] does, so the two modes differ only in how many
/// days the solver plans for.
pub(crate) fn problem_from_days(ctx: &SolverContext<'_>, days: u32) -> SolveProblem {
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
    SolveProblem::new(spots, agents, days)
}

/// Convert the loop solver's per-agent spot-id visit order into map
/// coordinates, one target list per controlled agent (in `ctx.agent_ids`
/// order) — for the plan's **first** leg, i.e. [`CURRENT_DAY_LEG`].
///
/// `pub(crate)` for the same reason as [`problem_from_state`]: `multiloop`
/// reuses this exact target/`plan_walk` path so the only difference between
/// the two candidates is the tour construction, not the day contract.
pub(crate) fn agent_targets(
    ctx: &SolverContext<'_>,
    outcome: &solver_loop::SolveOutcome,
) -> Vec<Vec<HexCoord>> {
    agent_targets_on_day(ctx, outcome, CURRENT_DAY_LEG)
}

/// [`agent_targets`] for one specific leg of the outcome (`plans[plan_day]`).
///
/// `plan_day` selects a day of the solve, not of the match: leg `i` covers
/// match day `anchor_day + i` (see [`CURRENT_DAY_LEG`]). A leg index past
/// the outcome yields empty target lists — the agent waits rather than
/// walking a plan that does not exist.
pub(crate) fn agent_targets_on_day(
    ctx: &SolverContext<'_>,
    outcome: &solver_loop::SolveOutcome,
    plan_day: usize,
) -> Vec<Vec<HexCoord>> {
    let map = &ctx.state.setting.map;
    let day_plan = outcome.plans.get(plan_day);
    ctx.agent_ids
        .iter()
        .enumerate()
        .map(|(agent_idx, _)| {
            day_plan
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
        Some("sweep-row") => Ok(InitialStrategy::SweepRow),
        Some("sweep-col") => Ok(InitialStrategy::SweepCol),
        Some("random") => match seed_param {
            Some(seed) => Ok(InitialStrategy::Random { seed }),
            None => Err(CandidateError::MissingParameter {
                algorithm: spec.algorithm.clone(),
                key: "seed".to_owned(),
            }),
        },
        Some(other) => {
            let _ = other;
            Err(bad("expected greedy | identity | sweep-row | sweep-col | random"))
        }
    }
}

/// Parse the `lookahead` parameter into a [`Lookahead`].
///
/// Absent means [`Lookahead::None`], which is byte-for-byte the behaviour
/// `loop` had before the param existed — every existing id therefore keeps
/// its semantics without a migration.
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any value other than
/// `none` / `crossday` (the value is taken verbatim: `CrossDay` and
/// `cross-day` are typos, not aliases).
fn parse_lookahead(spec: &CandidateSpec) -> Result<Lookahead, CandidateError> {
    match spec.param("lookahead") {
        None | Some("none") => Ok(Lookahead::None),
        Some("crossday") => Ok(Lookahead::CrossDay),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "lookahead".to_owned(),
            value: other.to_owned(),
            reason: "expected none | crossday".to_owned(),
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
         then walk the visit order. Supports initial=, iterations= and \
         lookahead= sweeps (lookahead=crossday plans the match's remaining days)."
    }

    fn validate_spec(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        parse_initial(spec)?;
        parse_lookahead(spec).map(|_| ())
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
        ] {
            assert!(reg.build(id).is_ok(), "{id} must be a valid candidate");
        }
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

    // ------------------------------------------------------------------
    // lookahead param gate (Issue #193)
    // ------------------------------------------------------------------

    #[test]
    fn lookahead_param_validation_matrix() {
        let reg = CandidateRegistry::builtin();
        // Accepted: absent (default), explicit none, explicit crossday.
        for id in [
            "loop",
            "loop:lookahead=none",
            "loop:lookahead=crossday",
            "loop:iterations=32:initial=greedy:lookahead=crossday",
            "loop:lookahead=crossday:initial=random:seed=7",
        ] {
            reg.build(id).unwrap_or_else(|e| panic!("{id} must build: {e}"));
        }
        // Rejected: any other value — through `validate_spec` (the AC path)
        // and through the registry's `build`, which calls it.
        for id in [
            "loop:lookahead=nope",
            "loop:lookahead=CrossDay",
            "loop:lookahead=cross-day",
            "loop:lookahead=true",
            "loop:lookahead=1",
        ] {
            let spec = CandidateSpec::parse(id).expect("syntax is fine");
            assert!(
                matches!(
                    LoopFactory.validate_spec(&spec),
                    Err(CandidateError::InvalidParameter { key, .. }) if key == "lookahead"
                ),
                "validate_spec must reject {id}"
            );
            assert!(
                matches!(
                    reg.build(id),
                    Err(CandidateError::InvalidParameter { key, .. }) if key == "lookahead"
                ),
                "build must reject {id}"
            );
        }
        // Empty value dies at the parse layer, unknown key at the schema layer.
        assert!(matches!(reg.build("loop:lookahead="), Err(CandidateError::MissingParamValue(_))));
        assert!(matches!(
            reg.build("loop:lookahead2=none"),
            Err(CandidateError::UnknownParameter { .. })
        ));
        // `Lookahead` itself defaults to the pre-#193 mode.
        assert_eq!(Lookahead::default(), Lookahead::None);
    }

    #[test]
    fn lookahead_canonicalises_into_schema_order_and_round_trips() {
        let reg = CandidateRegistry::builtin();
        let candidate =
            reg.build("loop:lookahead=crossday:iterations=32:initial=greedy").expect("builds");
        assert_eq!(candidate.id(), "loop:iterations=32:initial=greedy:lookahead=crossday");
        // The durable record stores ids, so parse -> id must reproduce it.
        let round_tripped =
            CandidateSpec::parse(&candidate.id()).expect("canonical id parses").id();
        assert_eq!(round_tripped, candidate.id());
        assert_eq!(candidate.build_solver().expect("builds").name(), "loop");

        // Canonical order is the factory schema, `lookahead` appended last so
        // every pre-existing id stays byte-stable.
        let cat = reg.catalog();
        let entry = cat.algorithms.iter().find(|a| a.algorithm == "loop").expect("loop listed");
        let keys: Vec<&str> = entry.params.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["iterations", "initial", "seed", "lookahead"]);
    }

    // ------------------------------------------------------------------
    // day contract: none ≡ current behaviour, crossday = remaining days
    // ------------------------------------------------------------------

    /// Deterministic fixture for the day-contract tests: 8×8 plain map,
    /// six spots (three series), three agents, `num_days` days of 8 steps.
    /// `day_steps_by_day` stays empty, which [`engine::model::MatchSetting`]
    /// defines as "every day uses `day_steps`".
    fn match_state(num_days: u32) -> engine::model::MatchState {
        use domain::{MatchId, PlayerId};
        use engine::model::{Agent, Cell, MapGrid, MapSize, MatchSetting, MatchState, Spot};

        let spots = [(1, 1), (3, 1), (5, 1), (1, 5), (3, 5), (5, 5)];
        let series = [1, 2, 3, 1, 2, 3];
        let cells = (0..8)
            .flat_map(|row| {
                (0..8).map(move |col| {
                    let coord = HexCoord::new(col, row);
                    let spot =
                        spots.iter().zip(series.iter()).find(|(c, _)| **c == (col, row)).map(
                            |(_, s)| Spot {
                                series: *s,
                                pos: coord,
                                max_stock: 3,
                                current_stock: 3,
                            },
                        );
                    Cell { coord, terrain: engine::model::Terrain::Plain, spot }
                })
            })
            .collect();
        let map = MapGrid::new(MapSize::new(8, 8).expect("8x8"), cells).expect("grid");
        let agents = vec![
            Agent::patrol(0, HexCoord::new(7, 3), 8, 4),
            Agent::supply(1, HexCoord::new(0, 2)),
            Agent::supply(2, HexCoord::new(6, 7)),
        ];
        MatchState::new(MatchSetting {
            match_id: MatchId::new("loop-lookahead").expect("id"),
            team: PlayerId::new("player-a").expect("id"),
            map,
            agents,
            day_steps: 8,
            num_days,
            day_steps_by_day: Vec::new(),
        })
        .expect("valid setting")
    }

    fn candidate_for(id: &str) -> LoopCandidate {
        LoopCandidate::from_spec(&CandidateSpec::parse(id).expect("syntax")).expect("valid spec")
    }

    /// `lookahead=none` — explicit and default — must be the exact behaviour
    /// `loop` shipped with before the param existed: a single-day solve of
    /// [`problem_from_state`] walking its first leg.
    #[test]
    fn lookahead_none_matches_the_default_and_the_single_day_reference_path() {
        let reg = CandidateRegistry::builtin();
        let default_solver = reg
            .build("loop:iterations=32:initial=greedy")
            .expect("builds")
            .build_solver()
            .expect("solver");
        let none_solver = reg
            .build("loop:iterations=32:initial=greedy:lookahead=none")
            .expect("builds")
            .build_solver()
            .expect("solver");

        for num_days in [1, 3] {
            for day in 0..num_days {
                let mut state = match_state(num_days);
                state.day = day;
                let ctx = SolverContext::from_state(&state);

                // The reference is the pre-#193 call chain, verbatim:
                // one solve with `days = 1`, then `plans.first()`.
                assert_eq!(problem_from_state(&ctx).days, 1, "the day contract is days=1");
                let reference_outcome = LoopSolver::new(InitialStrategy::GreedyNearestNeighbor, 32)
                    .solve(&problem_from_state(&ctx))
                    .expect("solve");
                let reference = plan_walk(&ctx, &agent_targets(&ctx, &reference_outcome));

                let from_default = default_solver.decide(&ctx);
                let from_none = none_solver.decide(&ctx);
                assert_eq!(
                    from_default, from_none,
                    "default ≡ lookahead=none (num_days={num_days} day={day})"
                );
                assert_eq!(
                    from_none, reference,
                    "none ≡ single-day reference path (num_days={num_days} day={day})"
                );
                assert!(from_none.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
            }
        }
    }

    /// The whole point of `lookahead=crossday`: plan the match's remaining
    /// days in one solve, then walk the current day's leg of that plan,
    /// re-anchoring from live state every day.
    #[test]
    fn crossday_solves_the_remaining_days_and_walks_the_current_leg() {
        let crossday = candidate_for("loop:iterations=32:initial=greedy:lookahead=crossday");
        let none = candidate_for("loop:iterations=32:initial=greedy");
        assert_eq!(crossday.lookahead, Lookahead::CrossDay);

        // Day 0 of a 3-day match: days = num_days - day = 3.
        let state = match_state(3);
        let ctx = SolverContext::from_state(&state);
        assert_eq!(ctx.day, 0);
        assert_eq!(remaining_days(&ctx), 3);
        assert_eq!(crossday.lookahead.plan_days(&ctx), 3, "plans the whole match from day 0");

        let outcome = crossday.inner.solve(&problem_from_days(&ctx, 3)).expect("crossday solves");
        assert_eq!(outcome.plans.len(), 3, "crossday produces multi-day plans");
        assert!(
            outcome.plans.iter().all(|leg| leg.iter().any(|a| !a.is_empty())),
            "each day's leg carries a share of the tour: {:?}",
            outcome.plans
        );

        // Walks the current day's leg — at day 0 of an anchored plan that is
        // literally `outcome.plans[current_day]`.
        let targets = crossday.day_targets(&ctx).expect("solves");
        assert_eq!(targets, agent_targets_on_day(&ctx, &outcome, ctx.day as usize));
        assert_eq!(targets, agent_targets_on_day(&ctx, &outcome, CURRENT_DAY_LEG));
        assert!(
            targets.iter().any(|t| !t.is_empty()),
            "crossday must hand out targets, got {targets:?}"
        );
        // ...and it is a real distribution difference, not `none` in disguise:
        // only the current leg's share of the tour, not the whole tour.
        assert_ne!(targets, none.day_targets(&ctx).expect("none solves"));
        assert!(crossday.decide(&ctx).is_well_shaped(&ctx.agent_ids, ctx.day_steps));

        // Day 1: re-solved from the same live state, now 2 days remain and
        // the plan has 2 legs; leg `i` covers match day `anchor + i`, so the
        // current day's leg is still index `current_day - anchor_day`.
        let mut state = match_state(3);
        state.day = 1;
        let ctx = SolverContext::from_state(&state);
        assert_eq!(remaining_days(&ctx), 2);
        assert_eq!(crossday.lookahead.plan_days(&ctx), 2, "days = num_days - day");
        let outcome = crossday.inner.solve(&problem_from_days(&ctx, 2)).expect("crossday solves");
        assert_eq!(outcome.plans.len(), 2, "one leg per remaining day");
        let targets = crossday.day_targets(&ctx).expect("solves");
        assert_eq!(targets, agent_targets_on_day(&ctx, &outcome, CURRENT_DAY_LEG));
        assert!(targets.iter().any(|t| !t.is_empty()), "{targets:?}");
        assert_ne!(targets, none.day_targets(&ctx).expect("none solves"));
        assert!(crossday.decide(&ctx).is_well_shaped(&ctx.agent_ids, ctx.day_steps));

        // Last day: one day remains, so the horizon collapses to `days = 1`
        // and crossday coincides with none exactly.
        let mut state = match_state(3);
        state.day = 2;
        let ctx = SolverContext::from_state(&state);
        assert_eq!(crossday.lookahead.plan_days(&ctx), 1);
        assert_eq!(
            crossday.day_targets(&ctx).expect("solves"),
            none.day_targets(&ctx).expect("none solves"),
            "last day of a multi-day match: crossday ≡ none"
        );

        // Defensive clamp: a context whose day reached num_days still plans
        // one day instead of failing with ZeroDays.
        let mut state = match_state(3);
        state.day = 3;
        let ctx = SolverContext::from_state(&state);
        assert_eq!(remaining_days(&ctx), 1);
        assert!(crossday.day_targets(&ctx).is_ok());
    }

    /// `crossday` on a 1-day match has `remaining = 1`, so it must reduce to
    /// `none` exactly — single-day cases are regression-free by construction.
    #[test]
    fn crossday_reduces_to_none_on_a_single_day_match() {
        let crossday = candidate_for("loop:iterations=32:initial=greedy:lookahead=crossday");
        let none = candidate_for("loop:iterations=32:initial=greedy");

        let state = match_state(1);
        let ctx = SolverContext::from_state(&state);
        assert_eq!(crossday.lookahead.plan_days(&ctx), 1);
        assert_eq!(
            crossday.day_targets(&ctx).expect("solves"),
            none.day_targets(&ctx).expect("none solves"),
            "1-day match: crossday ≡ none target lists"
        );
        assert_eq!(
            crossday.decide(&ctx),
            none.decide(&ctx),
            "1-day match: crossday ≡ none actions"
        );
    }
}
