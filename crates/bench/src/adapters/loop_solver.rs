//! `loop` candidate — the iterative-improvement loop solver from
//! `crates/solver-loop`.
//!
//! Parameterised so a whole axis of candidates is just different ids:
//!
//! ```text
//! loop                                  (defaults: iterations=50, initial=greedy,
//!                                        operators=classic, lookahead=none)
//! loop:iterations=32:initial=greedy
//! loop:iterations=128:initial=greedy
//! loop:iterations=64:initial=random:seed=42
//! loop:initial=sweep-row
//! loop:initial=sweep-col
//! loop:iterations=32:initial=greedy:operators=extended
//! loop:initial=reverse
//! loop:iterations=32:initial=greedy:lookahead=crossday
//! ```
//!
//! `operators=classic` (the default) is the pre-#191 move set, byte for
//! byte; `operators=extended` adds all seven classical 3-opt moves plus
//! or-opt (segment relocation ≤ 3) on top of it. The parameter is appended
//! last in [`PARAMS`] so existing canonical ids keep their shape.
//!
//! `walk=greedy` (the default) is today's [`super::walk::plan_walk`], byte
//! for byte; `walk=astar` walks the **same** target list in list order with
//! A*-planned legs (Issue #215) — see [`super::astar_walk`]. It is likewise
//! appended last in [`PARAMS`], after `operators`.
//!
//! `assign` (Issue #235) selects who the tour is *for*: `assign=agents`
//! (the default) is every agent, exactly as before; `assign=patrol` withholds
//! the target list from Supply agents so they wait instead of walking spots
//! they cannot collect from (Issue #234 made that explicit); `assign=escort`
//! (Issue #248) instead walks each Supply along its paired Patrol's projected
//! day path so the two meet and refuel mid-day. The parameter is appended
//! last in [`PARAMS`], so every pre-existing canonical id keeps its bytes and
//! its meaning.
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

use engine::model::{ActionKind, AgentActions, HexCoord, Solver, SolverContext};
use solver_loop::{
    Agent as LoopAgent, HexCoord as LoopCoord, InitialStrategy, LoopSolver, OperatorSet,
    Series as LoopSeries, SolveProblem, Solver as _, Spot as LoopSpot,
};

use super::walk::{AssignMode, WalkMode, parse_assign, parse_walk, plan_walk_with, step_towards};
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
    ParamSpec {
        key: "lookahead",
        required: false,
        doc: "multi-day planning horizon: none | crossday (default none)",
    },
    ParamSpec {
        key: "walk",
        required: false,
        doc: "day walk over the solved target list: greedy (default; today's plan_walk) | \
              astar (visit the list in order with A* legs, Issue #215)",
    },
    ParamSpec {
        key: "assign",
        required: false,
        doc: "who receives a tour target: agents (default; every agent, pre-#235) | \
              patrol (Supply agents get no target and wait, Issue #235) | \
              escort (Supply walks its paired Patrol's projected day path, Issue #248)",
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
    /// How the solved target lists are walked ([`super::walk::WalkMode`]).
    walk: WalkMode,
    /// Which agents receive a target list at all ([`super::walk::AssignMode`]).
    assign: AssignMode,
}

impl LoopCandidate {
    /// Build from an already-validated spec.
    fn from_spec(spec: &CandidateSpec) -> Result<Self, CandidateError> {
        let iterations = spec.param_u32("iterations")?.unwrap_or(50);
        let initial = parse_initial(spec)?;
        let operators = parse_operators(spec)?;
        let lookahead = parse_lookahead(spec)?;
        let walk = parse_walk(spec)?;
        let assign = parse_assign(spec)?;
        Ok(Self {
            inner: LoopSolver::with_operators(initial, iterations, operators),
            id: spec.id(),
            lookahead,
            walk,
            assign,
        })
    }

    /// Solve once from the live state and return **today's** per-agent
    /// target lists — the exact path [`Solver::decide`] walks.
    ///
    /// Composes both gates: `lookahead` picks the planning horizon
    /// (`problem_from_days`) and `operators=extended` may re-anchor a
    /// zero-reach outcome via [`Self::targets_for`]. Exposed to the
    /// module's tests so they can assert which leg of a multi-day outcome
    /// the adapter follows (Issue #193's AC), without duplicating the
    /// `decide` plumbing.
    fn day_targets(
        &self,
        ctx: &SolverContext<'_>,
    ) -> Result<Vec<Vec<HexCoord>>, solver_loop::SolverError> {
        let problem = problem_from_days(ctx, self.lookahead.plan_days(ctx));
        let outcome = self.inner.solve(&problem)?;
        Ok(self.targets_for(ctx, &outcome))
    }

    /// The per-agent target lists this candidate will walk.
    ///
    /// `operators=classic` (and therefore every default candidate id) takes
    /// the direct path and returns [`agent_targets`] unchanged — Issue #207
    /// must not perturb a single byte of the pre-existing behaviour. Only
    /// `operators=extended` may be re-anchored, and only when its default
    /// anchor would reach no spot at all; see [`rescue_anchor`].
    ///
    /// The probe behind that decision is evaluated with **this candidate's
    /// walk mode** ([`WalkMode`]), so `walk=astar` is only re-anchored when
    /// the A* walk itself would collect nothing — the rescue keeps matching
    /// the contract "0 iff this walk reaches no spot" no matter which walk
    /// the candidate selected, and a working assignment is never rewritten.
    fn targets_for(
        &self,
        ctx: &SolverContext<'_>,
        outcome: &solver_loop::SolveOutcome,
    ) -> Vec<Vec<HexCoord>> {
        let per_agent = agent_targets(ctx, outcome);
        if self.inner.operators != OperatorSet::Extended
            || agents_reaching_a_spot(ctx, &per_agent, self.walk) > 0
        {
            return per_agent;
        }
        rescue_anchor(ctx, outcome, &per_agent, self.walk)
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
            // `assign` filters the finished target lists, so it composes with
            // every `lookahead` / `operators` / `walk` combination without any
            // of them having to know kinds exist (Issue #235). The walk mode
            // doubles as the escort projection's walker (Issue #248), so the
            // Supply chases exactly the path its paired Patrol will walk.
            Ok(targets) => {
                plan_walk_with(ctx, &self.assign.apply(ctx, targets, self.walk), self.walk)
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

// ---------------------------------------------------------------------------
// Issue #207 — reachability probe + anchor rescue (`operators=extended` only)
// ---------------------------------------------------------------------------

/// How many controlled agents would stand on a spot cell at least once if
/// `walk` walked `targets` for this day.
///
/// The probe replays **the walk the candidate selected**: for
/// [`WalkMode::Greedy`] that is [`super::walk::plan_walk`]'s own position
/// bookkeeping *exactly* — same live start positions, same
/// `step % targets.len()` target cycling, same [`step_towards`] — and for
/// [`WalkMode::Astar`] it is the actions
/// [`super::astar_walk::plan_walk_astar`] actually produces. Consequently
/// the count is `0` **if and only if** no agent can collect anything from
/// this target assignment on this day *with this walk* — `engine::MatchSim`
/// only acquires when an agent ends a step on a spot.
///
/// It is a read-only probe: it never mutates the state and it is never used
/// to pick a *better* assignment when the default one already works (see
/// [`rescue_anchor`]).
fn agents_reaching_a_spot(
    ctx: &SolverContext<'_>,
    targets: &[Vec<HexCoord>],
    walk: WalkMode,
) -> usize {
    let map = &ctx.state.setting.map;
    let day_steps = ctx.day_steps as usize;
    match walk {
        WalkMode::Greedy => ctx
            .agent_ids
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
            .count(),
        WalkMode::Astar => {
            // Trace the very plan the candidate would emit, step-major.
            let plan = super::astar_walk::plan_walk_astar(ctx, targets);
            let agent_count = ctx.agent_ids.len();
            ctx.agent_ids
                .iter()
                .enumerate()
                .filter(|(i, agent_id)| {
                    let Some(start) = ctx.state.agents.iter().find(|a| a.id == **agent_id) else {
                        return false;
                    };
                    let mut pos = start.pos;
                    for step in 0..day_steps {
                        let Some(action) = plan.actions.get(step * agent_count + *i) else {
                            break;
                        };
                        if let ActionKind::Move(dir) = action.kind {
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
    }
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
    walk: WalkMode,
) -> Vec<Vec<HexCoord>> {
    let n_agents = ctx.agent_ids.len();
    let Some(tour) = tour_from_outcome(outcome, n_agents) else {
        return default.to_vec();
    };

    let mut best = default.to_vec();
    let mut best_hits = agents_reaching_a_spot(ctx, &best, walk);
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
            let hits = agents_reaching_a_spot(ctx, &candidate, walk);
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
         then walk the visit order. Supports initial=, iterations=, operators=, \
         lookahead=, walk= and assign= sweeps (lookahead=crossday plans the \
         remaining days; assign=patrol withholds the tour from Supply agents; \
         assign=escort walks each Supply along its paired Patrol's path, Issue #248)."
    }

    fn validate_spec(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        parse_initial(spec)?;
        parse_operators(spec)?;
        parse_lookahead(spec)?;
        parse_walk(spec)?;
        parse_assign(spec)?;
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
    use crate::adapters::walk::plan_walk;
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
            let reached =
                agents_reaching_a_spot(&ctx, &agent_targets(&ctx, &outcome), WalkMode::Greedy);
            println!("{ops:?}: reachable_agents={reached}/{} tour={tour:?}", problem.agents.len());
            outcomes.push((tour, outcome));
        }

        let (classic_tour, classic_outcome) = &outcomes[0];
        let (extended_tour, extended_outcome) = &outcomes[1];
        let classic_hits =
            agents_reaching_a_spot(&ctx, &agent_targets(&ctx, classic_outcome), WalkMode::Greedy);
        let extended_hits =
            agents_reaching_a_spot(&ctx, &agent_targets(&ctx, extended_outcome), WalkMode::Greedy);

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
            agents_reaching_a_spot(&ctx, &rescued, WalkMode::Greedy) >= 1,
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
        if agents_reaching_a_spot(&ctx, &default, WalkMode::Greedy) > 0 {
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

    /// Issue #215 — `walk` is a schema-gated parameter appended **last**, so
    /// every pre-existing id keeps its bytes. Both legal values build
    /// (including inside full canonical ids), any other value is
    /// `InvalidParameter`, and an unknown key still loses to the schema
    /// before value validation.
    #[test]
    fn walk_param_is_schema_validated() {
        let reg = CandidateRegistry::builtin();
        for id in [
            "loop:walk=greedy",
            "loop:walk=astar",
            "loop:iterations=32:initial=greedy:walk=astar",
            "loop:iterations=32:initial=greedy:operators=extended:walk=astar",
            "loop:initial=sweep-row:walk=greedy",
        ] {
            let candidate = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(candidate.id(), id, "canonical order must round-trip: {id}");
            assert!(candidate.build_solver().is_ok(), "{id} must build a solver");
        }
        for id in [
            "loop:walk=nope",
            "loop:walk=Astar",
            "loop:walk=",
            "loop:iterations=32:initial=greedy:walk=step",
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
            reg.build("loop:walkmode=astar"),
            Err(CandidateError::UnknownParameter { .. })
        ));
    }

    /// Issue #215 AC — default equivalence: writing `walk=greedy` explicitly
    /// changes not one action of an existing candidate id on a fixed live
    /// input, for both the plain and the rescue-carrying `extended` id.
    #[test]
    fn walk_greedy_decide_is_byte_identical_to_the_omitted_default() {
        let ctx = road_heavy_v2_context();
        for (base, explicit) in [
            ("loop:iterations=32:initial=greedy", "loop:iterations=32:initial=greedy:walk=greedy"),
            (EXTENDED_ID, "loop:iterations=32:initial=greedy:operators=extended:walk=greedy"),
        ] {
            assert_eq!(
                loop_candidate(base).decide(&ctx),
                loop_candidate(explicit).decide(&ctx),
                "{explicit} must be byte-for-byte {base}"
            );
        }
    }

    /// Issue #215 — the Issue #207 anchor that `plan_walk` never reaches on
    /// `fixture:road-heavy-v2` **is** reached by `walk=astar` on the very
    /// same assignment, so `walk=astar` must not fire the rescue: the
    /// assignment stays identical and only the walk moves. `walk=greedy`
    /// keeps rescuing exactly as it did before #215.
    #[test]
    fn walk_astar_reaches_the_207_anchor_and_keeps_the_assignment() {
        let ctx = road_heavy_v2_context();
        let problem = problem_from_state(&ctx);
        let solver = LoopSolver::with_operators(
            InitialStrategy::GreedyNearestNeighbor,
            32,
            OperatorSet::Extended,
        );
        let outcome = solver.solve(&problem).expect("solve");
        let targets = agent_targets(&ctx, &outcome);

        // Control (Issue #207): the greedy walk model reports zero reachable
        // agents — the rescue's documented trigger.
        assert_eq!(
            agents_reaching_a_spot(&ctx, &targets, WalkMode::Greedy),
            0,
            "the Issue #207 repro must still be a zero under plan_walk"
        );
        // The fix: the ordered A* walk on the *same* assignment collects.
        assert!(
            agents_reaching_a_spot(&ctx, &targets, WalkMode::Astar) >= 1,
            "walk=astar must reach where plan_walk oscillates"
        );

        // Therefore the astar candidate keeps the default anchor untouched …
        let astar =
            loop_candidate("loop:iterations=32:initial=greedy:operators=extended:walk=astar");
        assert_eq!(
            astar.targets_for(&ctx, &outcome),
            targets,
            "walk=astar must not re-anchor an assignment its own walk can use"
        );
        // … while the greedy candidate still re-anchors, byte for byte as
        // before #215.
        let rescued = extended_candidate().targets_for(&ctx, &outcome);
        assert_ne!(rescued, targets, "the greedy rescue must still fire here");
        assert!(
            agents_reaching_a_spot(&ctx, &rescued, WalkMode::Greedy) >= 1,
            "and the rescued anchor must still be walkable"
        );
    }

    /// `walk=astar` rides the same `decide` path: a well-shaped day whose
    /// actions are genuinely the ordered A* walk, not the greedy one.
    #[test]
    fn walk_astar_decide_is_well_shaped_and_changes_the_walk() {
        let ctx = road_heavy_v2_context();
        let plan = loop_candidate("loop:iterations=32:initial=greedy:walk=astar").decide(&ctx);
        assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps), "step-major shape");
        assert_ne!(
            plan,
            loop_candidate("loop:iterations=32:initial=greedy").decide(&ctx),
            "walk=astar must actually change the emitted walk"
        );
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

        // Canonical order is the factory schema: landed `operators` (#191)
        // then `lookahead` (#193), with `walk` (#215) appended last, so every
        // pre-existing id stays byte-stable.
        let cat = reg.catalog();
        let entry = cat.algorithms.iter().find(|a| a.algorithm == "loop").expect("loop listed");
        let keys: Vec<&str> = entry.params.iter().map(|p| p.key.as_str()).collect();
        // `assign` (#235) is appended after `walk` (#215): every older
        // canonical id keeps its exact shape.
        assert_eq!(
            keys,
            ["iterations", "initial", "seed", "operators", "lookahead", "walk", "assign"]
        );
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
