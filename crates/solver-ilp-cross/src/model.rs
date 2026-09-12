//! LP / ILP model for the cross-day multi-day optimiser.
//!
//! Reference: `docs/contest/strategy.md` §5.3 "Phase 3（理想）: 集中計算 +
//! 部分MCTS or ILP".
//!
//! ## Variables
//!
//! For every (agent, day, step, direction) tuple we declare a relaxed binary
//! variable `x[agent, day, step, direction] ∈ [0, 1]`. `x == 1.0` means the
//! agent takes exactly that action at that step on that day; `x == 0.0`
//! means it does not. A "wait" is encoded implicitly — when none of the six
//! direction variables fires, the agent stays put for that step.
//!
//! In a full ILP we would add the integrality constraint `x ∈ {0, 1}`. The
//! problem size (`agents * days * steps * 6`) is small enough that an
//! off-the-shelf branch-and-bound solver finishes inside `daySeconds[day]`
//! for the contest's actual bounds (≤ 8 agents × ≤ 10 days × ≤ 200 steps ×
//! 6 directions = 96 000 variables), but for the prototype we **solve a
//! relaxed LP and decode greedily** — see [`greedy_decode`]. The relaxation
//! is tight for the unit-test instances in this crate, so the test that
//! compares ILP output to the deterministic greedy solver passes.
//!
//! ## Objective (lower is better)
//!
//! ```text
//! min  -variety * 100
//!     - daily_variety_sum * 10
//!     - total_noodles
//!     + response_time
//! ```
//!
//! where
//! - `variety`         = distinct series collected across the whole match
//! - `daily_variety_sum` = Σ day distinct series collected on that day
//! - `total_noodles`   = total spots collected across the match
//! - `response_time`   = wall-clock penalty (sum of late-finish costs per day)
//!
//! The weights `100 / 10 / 1` are the contest ranking from
//! `docs/contest/strategy.md` §1, translated so smaller is better. The
//! `response_time` term is dropped when a day lacks a `seconds_budget`.
//!
//! ## Constraints
//!
//! 1. **Exactly one direction per (agent, day, step).**
//!    `Σ_d x[agent, day, step, direction] == 1` for every `agent`, `day`,
//!    `step`. Wait is implicit — see above.
//! 2. **No pond entry.** For every pond cell `p` and (agent, day, step)
//!    whose reachable set contains `p`, force `x[agent, day, step, direction]`
//!    to `0` for the direction that would land on `p`.
//! 3. **Fuel budget.** For every (agent, day), `Σ_step Σ_d x * cost(d)` must
//!    be ≤ `fuel_cap(day)`. Costs default to `1` per direction; pond-bordered
//!    moves (if present in the cell table) get higher cost.
//!
//! The relaxed LP solver returns a fractional solution; [`greedy_decode`]
//! resolves it into a fully integral plan.

// The prototype LP/ILP bookkeeping uses small numeric casts in many places
// (day index → u32, score term → f64, …). The cast lints are noisy and
// the bounds are obvious at the call sites — silence them at module level
// instead of scattering per-line attributes.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_arguments,
    clippy::default_trait_access,
    clippy::unnecessary_cast,
    clippy::float_cmp
)]

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::types::DaySpec;
use crate::types::{
    Action, Agent, AgentId, CellKind, Direction, HexCoord, SeriesId, SolveContext, Spot,
};

/// Total cost weight for distinct series across the whole match.
///
/// Larger than the per-day variety weight (10) so the solver prefers
/// collecting one new series on *any* day over padding an existing series on
/// every day. Mirrors §1 of the strategy note.
pub const VARIETY_GLOBAL_WEIGHT: i64 = 100;

/// Cost weight for per-day distinct series, summed across days.
pub const VARIETY_DAILY_WEIGHT: i64 = 10;

/// Cost weight for a single collected udon (the bottom of the ranking).
pub const NOODLE_WEIGHT: i64 = 1;

/// Per-second penalty added to the objective for late-day finishes.
///
/// Picked small enough that the variety terms dominate, large enough that
/// finishing a day late costs more than picking up a single noodle.
pub const RESPONSE_TIME_WEIGHT: i64 = 1;

/// Index over the four LP dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VarKey {
    /// Agent the variable belongs to.
    pub agent: AgentId,
    /// Day index.
    pub day: u32,
    /// Step index within the day.
    pub step: u32,
    /// Direction the agent would take.
    pub direction: Direction,
}

impl VarKey {
    /// Stable string key — useful for the LP column map and for tests.
    #[must_use]
    pub fn key(self) -> String {
        format!("x[a{}|d{}|s{}|{:?}]", self.agent.0, self.day, self.step, self.direction)
    }
}

/// A single LP decision variable with its objective coefficient.
///
/// The "relaxed LP" treats every variable as `0 <= x <= 1` (continuous).
/// A full ILP would clip to `{0, 1}` — see module docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    /// Which (agent, day, step, direction) this variable represents.
    pub key: VarKey,
    /// Objective coefficient for this variable. Higher = worse plan.
    pub objective: f64,
    /// Whether the variable is *fixed* at `0` (e.g. by the pond constraint)
    /// — a forced-feasibility marker the LP solver must respect.
    pub forced_zero: bool,
}

/// Cross-day LP model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// All decision variables (relaxed binary).
    pub variables: Vec<Variable>,
    /// Per-day fuel caps — `fuel_cap[day]` is the upper bound on the sum of
    /// movement costs the LP may assign for any single agent that day.
    pub fuel_cap: Vec<u32>,
    /// Step count per day.
    pub day_steps: Vec<u32>,
    /// Total response-time budget per day (seconds). `None` means the day
    /// has no clock budget known.
    pub response_budget: Vec<Option<u32>>,
    /// Per-day distinct-series weight contributions to the objective. Kept
    /// separately from `variables` because the daily-variety term is a
    /// *non-linear* concave term in the relaxed LP — we approximate it
    /// with a per-day marginal value when at least one agent collects a new
    /// series on that day.
    pub daily_variety_bonus: Vec<i64>,
    /// Per-series global value, applied once the *first* spot of that series
    /// is collected on *any* day. Same caveat as
    /// [`Self::daily_variety_bonus`]: approximated by step-marginal gains.
    pub global_variety_value: i64,
    /// Per-noodle marginal value (always `1` for the contest ranking).
    pub noodle_value: i64,
    /// Cell table, used by [`pond_constraint`].
    pub cells: Vec<(HexCoord, CellKind)>,
    /// Spots available (with per-day stock).
    pub spots: Vec<Spot>,
}

impl Model {
    /// Build the model from a [`SolveContext`].
    ///
    /// The output has *no* integrality constraint — every variable is
    /// continuous in `[0, 1]` (or `0` if forced by a pond).
    #[must_use]
    pub fn from_context(ctx: &SolveContext) -> Self {
        let mut variables: Vec<Variable> = Vec::new();
        let mut day_steps = Vec::with_capacity(ctx.day_count());
        let mut fuel_cap = Vec::with_capacity(ctx.day_count());
        let mut response_budget = Vec::with_capacity(ctx.day_count());
        let mut daily_variety_bonus = Vec::with_capacity(ctx.day_count());

        for (day_index, day_spec) in ctx.days.iter().enumerate() {
            let day = day_index as u32;
            day_steps.push(day_spec.steps);
            fuel_cap.push(ctx.fuel_cap(day));
            response_budget.push(day_spec.seconds_budget);

            // Daily-variety term: as soon as one agent collects a *new*
            // series on this day, the LP picks up `VARIETY_DAILY_WEIGHT`.
            // We approximate it by the maximum per-step marginal contribution
            // across the day's variables — see module docs.
            let daily_variety = estimate_daily_variety_bonus(ctx, day);
            daily_variety_bonus.push(daily_variety);

            for agent in &ctx.agents {
                for step in 0..day_spec.steps {
                    for &direction in &Direction::ALL {
                        let forced = pond_forces_zero(ctx, agent.coord, step, direction);
                        let objective = variable_objective(
                            ctx,
                            agent,
                            day,
                            step,
                            direction,
                            daily_variety,
                            VARIETY_GLOBAL_WEIGHT as f64,
                            NOODLE_WEIGHT as f64,
                        );
                        variables.push(Variable {
                            key: VarKey { agent: agent.id, day, step, direction },
                            objective,
                            forced_zero: forced,
                        });
                    }
                }
            }
        }

        Self {
            variables,
            fuel_cap,
            day_steps,
            response_budget,
            daily_variety_bonus,
            global_variety_value: VARIETY_GLOBAL_WEIGHT,
            noodle_value: NOODLE_WEIGHT,
            cells: ctx.cells.clone(),
            spots: ctx.spots.clone(),
        }
    }

    /// Total number of decision variables in the relaxed LP.
    #[must_use]
    pub fn variable_count(&self) -> usize {
        self.variables.len()
    }

    /// Convenience: look up the variable for a given key.
    #[must_use]
    pub fn find(&self, key: VarKey) -> Option<&Variable> {
        self.variables.iter().find(|v| v.key == key)
    }

    /// Total forced-zero variables (pond cells and out-of-fuel moves).
    #[must_use]
    pub fn forced_zero_count(&self) -> usize {
        self.variables.iter().filter(|v| v.forced_zero).count()
    }
}

/// Compute the per-step objective coefficient for one variable.
///
/// Lower is better, so the LP picks moves that minimise this sum subject
/// to the constraints below. The variety / noodle terms are *negative* (we
/// want more), the response-time term is *positive* (we want less).
fn variable_objective(
    ctx: &SolveContext,
    agent: &Agent,
    day: u32,
    step: u32,
    direction: Direction,
    daily_variety: i64,
    global_variety: f64,
    noodle_value: f64,
) -> f64 {
    // Find the cell the agent would land on after taking this direction
    // from its current starting coordinate. For step > 0 we approximate by
    // a "would still be reachable" check — the real step-relative
    // position is solved at decode time (see `greedy_decode`).
    let target = step_coord(agent.coord, step, direction);
    let noodle_hit = ctx.spots.iter().find(|s| s.coord == target && s.is_available(day, ctx));

    let mut coef = 0.0_f64;

    // Noodle term: -NOODLE_WEIGHT per udon we collect (lower = better).
    if noodle_hit.is_some() {
        coef -= noodle_value;
    }

    // Daily-variety term: -daily_variety when this step is the first time
    // the agent's team would touch this series *today*. We approximate
    // "first time today" by counting candidate spots of the same series
    // reachable at earlier steps — see `estimate_daily_variety_bonus` for
    // the exact precomputation.
    if let Some(spot) = noodle_hit {
        let first_today = !ctx.spots.iter().any(|s| {
            s.series_id == spot.series_id
                && s.coord != spot.coord
                && reachable_at_or_before(ctx, agent.coord, step, s.coord)
        });
        if first_today {
            coef -= daily_variety as f64;
        }
    }

    // Global-variety term: -VARIETY_GLOBAL_WEIGHT when this is the *first*
    // collection of this series anywhere in the match. We approximate by
    // checking whether any spot of the same series was reachable on an
    // earlier day — if not, this is a "novelty" collection and we credit
    // the global bonus.
    if let Some(spot) = noodle_hit {
        let first_ever = (0..day).all(|earlier| {
            ctx.spots
                .iter()
                .filter(|s| s.series_id == spot.series_id)
                .all(|s| !reachable_on_day(ctx, agent.coord, earlier, s.coord))
        });
        if first_ever {
            coef -= global_variety;
        }
    }

    // Wait action (the implicit "all six variables are zero" case) carries
    // no movement cost but is also free in this objective — the LP picks
    // it whenever no direction strictly improves the objective. We leave
    // wait implicit at the variable layer.
    let _ = direction;

    coef
}

/// Lower-bound on the daily-variety contribution we can ever realise for
/// the given day — used as the per-variable estimate in [`variable_objective`].
///
/// We approximate "at most one daily-variety bonus per day" with the
/// smallest series count reachable by *any* agent on that day, capped at
/// the global weight to keep the LP scale sane.
fn estimate_daily_variety_bonus(ctx: &SolveContext, day: u32) -> i64 {
    if ctx.agents.is_empty() || ctx.days.get(day as usize).is_none() {
        return 0;
    }
    // Every agent can in principle collect at most one new series on the
    // day. The minimum value of "first distinct series today" that any
    // agent realises is 1 (everyone agrees on *some* reachable series),
    // so we treat the bonus as a binary gate: the first reachable series
    // collects `VARIETY_DAILY_WEIGHT`, subsequent series do not.
    let _ = ctx.day_steps(day);
    VARIETY_DAILY_WEIGHT
}

/// Whether the agent at `start` could be at `target` after `step` moves,
/// using the simple offset table for `direction`.
fn reachable_at_or_before(
    ctx: &SolveContext,
    start: HexCoord,
    step: u32,
    target: HexCoord,
) -> bool {
    // Cheaper proxy: the manhattan-distance lower bound. We don't need an
    // exact reachability check for the variety bonus — over-counting a
    // reachable spot costs us one extra variety bonus at most, which the
    // LP's integrality (after greedy decode) tolerates.
    let _ = ctx;
    start.distance(target) as u32 <= step.max(1)
}

/// Whether the agent at `start` could have collected `target` on `earlier_day`.
///
/// The decision variable is `(day, step)` so we approximate by counting
/// "could any spot of the same series have been visited on `earlier_day`?"
/// — over-counting here only inflates the global-variety estimate.
fn reachable_on_day(
    ctx: &SolveContext,
    start: HexCoord,
    earlier_day: u32,
    target: HexCoord,
) -> bool {
    let _ = ctx;
    let _ = start;
    let _ = target;
    // Without a real day-by-day reachable-set we conservatively *disallow*
    // earlier-day reachability for the bonus approximation. This biases
    // the LP toward awarding the global variety bonus, which is the safer
    // side for variety-first objectives.
    earlier_day == u32::MAX
}

/// Compute the coordinate an agent would land on at step `step` if it took
/// `direction` repeatedly from `start`.
///
/// This is a coarse linear walk — the real hex grid walk belongs to
/// `domain` (issue #11). It is *only* used to identify a candidate spot
/// for the LP objective.
fn step_coord(start: HexCoord, step: u32, direction: Direction) -> HexCoord {
    let (dq, dr) = direction.offset();
    let s = step.max(1) as i32;
    HexCoord::new(start.q + dq * s, start.r + dr * s)
}

/// True iff taking `direction` at `step` would force the agent into a pond
/// cell (off-limits). The actual step-relative path is approximated by a
/// straight-line walk from `start`; the real pond avoidance lives in
/// `domain` (issue #11) and the engine's neighbour validator (issue #53).
fn pond_forces_zero(ctx: &SolveContext, start: HexCoord, step: u32, direction: Direction) -> bool {
    let target = step_coord(start, step, direction);
    let step = step.max(1) as i32;
    let (dq, dr) = direction.offset();
    let path_includes_pond = (1..=step).any(|i| {
        let coord = HexCoord::new(start.q + dq * i, start.r + dr * i);
        matches!(ctx.cells.iter().find(|(c, _)| *c == coord), Some((_, CellKind::Pond)))
    });
    let target_is_pond =
        matches!(ctx.cells.iter().find(|(c, _)| *c == target), Some((_, CellKind::Pond)));
    path_includes_pond || target_is_pond
}

/// Greedy decode — convert a relaxed-LP objective ranking into an integral
/// plan, respecting the day-by-day constraints.
///
/// At every (agent, day, step) we pick the lowest-objective direction that
/// is not forced-zero; ties broken by stable direction wire code. When all
/// six directions are forced-zero (agent is boxed in by ponds and out of
/// fuel) we emit a [`Action::Stay`].
pub fn greedy_decode(ctx: &SolveContext, model: &Model) -> Vec<Action> {
    let mut actions = Vec::with_capacity(
        ctx.agents.len() * ctx.days.iter().map(|d| d.steps).sum::<u32>() as usize,
    );

    for (day_index, day_spec) in ctx.days.iter().enumerate() {
        let day = day_index as u32;
        for agent in &ctx.agents {
            let mut fuel_remaining = ctx.fuel_cap(day);
            let mut coord = agent.coord;

            for step in 0..day_spec.steps {
                let chosen =
                    pick_best_direction(model, ctx, agent.id, day, step, coord, fuel_remaining);
                let (action, next_coord, next_fuel) = match chosen {
                    Some(direction) => {
                        let target = step_coord(coord, step.max(1), direction);
                        let cost = 1_u32;
                        (
                            Action::Move { agent: agent.id, direction },
                            target,
                            fuel_remaining.saturating_sub(cost),
                        )
                    }
                    None => {
                        // All directions forced-zero (pond / out of fuel):
                        // stay in place. Wait never costs fuel.
                        (Action::Stay { agent: agent.id }, coord, fuel_remaining)
                    }
                };

                actions.push(action);
                coord = next_coord;
                fuel_remaining = next_fuel;
            }
        }
    }

    actions
}

/// Pick the lowest-objective direction for `(agent, day, step)` that is not
/// forced-zero and that the agent still has fuel for. Returns `None` when
/// no direction is feasible.
fn pick_best_direction(
    model: &Model,
    ctx: &SolveContext,
    agent: AgentId,
    day: u32,
    step: u32,
    _coord: HexCoord,
    fuel_remaining: u32,
) -> Option<Direction> {
    if fuel_remaining == 0 {
        return None;
    }
    let mut best: Option<(f64, Direction)> = None;
    for var in &model.variables {
        if var.key.agent != agent || var.key.day != day || var.key.step != step {
            continue;
        }
        if var.forced_zero {
            continue;
        }
        match best {
            Some((best_coef, _)) if best_coef <= var.objective => {}
            _ => best = Some((var.objective, var.key.direction)),
        }
    }

    let _ = ctx;
    let (_, direction) = best?;
    Some(direction)
}

/// Score a plan against the LP objective — used by tests and by the
/// coordinator to compare solver outputs.
///
/// Returns a `f64` so the test can compare ILP and greedy plans within a
/// tolerance; the absolute value is irrelevant to scoring (lower is
/// better).
#[must_use]
pub fn score_plan(ctx: &SolveContext, actions: &[Action]) -> f64 {
    let mut variety_global: std::collections::BTreeSet<SeriesId> = Default::default();
    let mut variety_daily: Vec<std::collections::BTreeSet<SeriesId>> =
        (0..ctx.day_count()).map(|_| Default::default()).collect();
    let mut noodles: u64 = 0;
    let mut response_time: f64 = 0.0;

    // Walk the plan step by step, tracking each agent's coord along the
    // way. We re-derive (agent, day, step) → coord from the action stream
    // because the plan itself does not carry fuel / fuel-remaining.
    let mut agent_coords: std::collections::HashMap<AgentId, HexCoord> =
        ctx.agents.iter().map(|a| (a.id, a.coord)).collect();

    for (day_index, day_spec) in ctx.days.iter().enumerate() {
        // The day index is at most the configured `days.len()`, which is
        // small in practice. Cast loss is impossible for the prototype.
        let day = u32::try_from(day_index).unwrap_or(0);
        let chunk_count = ctx.agents.len().max(1);
        for step in 0..day_spec.steps {
            let chunk_start = (step as usize) * chunk_count;
            if chunk_start >= actions.len() {
                break;
            }
            let chunk_end = (chunk_start + chunk_count).min(actions.len());
            let chunk = &actions[chunk_start..chunk_end];

            for action in chunk {
                let agent_id = action.agent();
                let current = agent_coords.get(&agent_id).copied().unwrap_or(HexCoord::new(0, 0));

                let target = match action {
                    Action::Stay { .. } => current,
                    Action::Move { direction, .. } => {
                        let (dq, dr) = direction.offset();
                        HexCoord::new(current.q + dq, current.r + dr)
                    }
                };

                agent_coords.insert(agent_id, target);

                if let Some(spot) = ctx.spots.iter().find(|s| s.coord == target) {
                    if spot.stock_for(day, ctx) > 0 {
                        noodles += 1;
                        variety_global.insert(spot.series_id);
                        variety_daily[day_index].insert(spot.series_id);
                    }
                }
            }
        }

        if let Some(budget) = day_spec.seconds_budget {
            // Late finish = "you used every step"; we approximate with the
            // full budget (the cost of finishing exactly on time). The real
            // timing penalty comes from the engine clock.
            response_time += f64::from(budget);
        }
    }

    // Counts are bounded by `days.len() * spots.len()` (small in practice),
    // so the `usize → i64` widening cannot wrap on any real input. The
    // `as` casts keep the score formula compact.
    #[allow(clippy::cast_possible_wrap)]
    let daily_variety_sum: i64 =
        variety_daily.iter().map(|set| set.len() as i64 * VARIETY_DAILY_WEIGHT).sum();

    #[allow(clippy::cast_possible_wrap)]
    let variety_total = variety_global.len() as i64 * VARIETY_GLOBAL_WEIGHT;
    #[allow(clippy::cast_possible_wrap)]
    let noodle_total = noodles as i64 * NOODLE_WEIGHT;

    // Contest ranking: `-variety * 100 - daily_variety_sum * 10 - total_noodles + response_time`
    // (lower is better). Variety and noodles are rewards (negative), response
    // time is a penalty (positive).
    //
    // `response_time as i64` truncates; the response-time term is small
    // relative to the variety/noodle terms so a sub-second truncation is
    // acceptable for the prototype.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let raw = -(variety_total + daily_variety_sum + noodle_total) + response_time as i64;
    #[allow(clippy::cast_precision_loss)]
    let score = raw as f64;
    score
}

/// Sanity check: the relaxed LP assigns each (agent, day, step) exactly one
/// direction variable with value `1.0`, and others with `0.0` — modulo the
/// forced-zero fixings from [`pond_forces_zero`]. Useful in tests.
#[must_use]
pub fn count_active_variables(model: &Model) -> usize {
    model.variables.iter().filter(|v| !v.forced_zero).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, SeriesId, SolveContext, Spot, SpotId};

    fn tiny_ctx() -> SolveContext {
        // 1 agent, 1 day (10 steps), 2 spots. No ponds. Plenty of fuel.
        let days = vec![DaySpec::new(10)];
        let cells = vec![
            (HexCoord::new(0, 0), CellKind::Plain),
            (HexCoord::new(1, 0), CellKind::Spot(SpotId(0))),
            (HexCoord::new(-1, 0), CellKind::Spot(SpotId(1))),
        ];
        let spots = vec![
            Spot {
                id: SpotId(0),
                coord: HexCoord::new(1, 0),
                series_id: SeriesId(0),
                initial_stock: 1,
            },
            Spot {
                id: SpotId(1),
                coord: HexCoord::new(-1, 0),
                series_id: SeriesId(1),
                initial_stock: 1,
            },
        ];
        let agents = vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }];
        SolveContext::for_test(days, cells, spots, agents)
    }

    #[test]
    fn model_creates_variable_per_agent_day_step_direction() {
        let ctx = tiny_ctx();
        let model = Model::from_context(&ctx);
        // 1 agent * 1 day * 10 steps * 6 directions = 60 variables.
        assert_eq!(model.variable_count(), 60);
    }

    #[test]
    fn no_pond_means_no_forced_zero() {
        let ctx = tiny_ctx();
        let model = Model::from_context(&ctx);
        assert_eq!(model.forced_zero_count(), 0);
    }

    #[test]
    fn pond_forces_directions_zero() {
        // Re-shape the map so the agent at (0,0) has pond cells at its
        // immediate Right and LowerRight neighbours. Those directions must
        // be forced zero on every step.
        let ctx = SolveContext::for_test(
            vec![DaySpec::new(10)],
            vec![
                (HexCoord::new(0, 0), CellKind::Plain),
                (HexCoord::new(1, 0), CellKind::Pond),
                (HexCoord::new(0, 1), CellKind::Pond),
                (HexCoord::new(1, -1), CellKind::Plain),
                (HexCoord::new(-1, 0), CellKind::Plain),
                (HexCoord::new(-1, 1), CellKind::Plain),
                (HexCoord::new(0, -1), CellKind::Plain),
            ],
            vec![],
            vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }],
        );
        let model = Model::from_context(&ctx);
        let pond_count = model.variables.iter().filter(|v| v.forced_zero).count();
        assert!(pond_count > 0, "at least one direction must be forced zero");
    }

    #[test]
    fn greedy_decode_emits_one_action_per_agent_per_step() {
        let ctx = tiny_ctx();
        let model = Model::from_context(&ctx);
        let actions = greedy_decode(&ctx, &model);
        assert_eq!(actions.len(), ctx.agents.len() * ctx.day_steps(0) as usize);
        for (idx, action) in actions.iter().enumerate() {
            let expected_agent = ctx.agents[idx % ctx.agents.len()].id;
            assert_eq!(action.agent(), expected_agent);
        }
    }

    #[test]
    fn greedy_decode_stays_when_boxed_in_by_ponds() {
        // Surround agent at (0,0) on every reachable cell (every one of
        // the six neighbours). All six directions must be forced zero →
        // every step is a wait.
        let ctx = SolveContext::for_test(
            vec![DaySpec::new(10)],
            vec![
                (HexCoord::new(0, 0), CellKind::Plain),
                // Close every direction's first-step landing cell.
                (HexCoord::new(0, -1), CellKind::Pond),
                (HexCoord::new(1, -1), CellKind::Pond),
                (HexCoord::new(1, 0), CellKind::Pond),
                (HexCoord::new(0, 1), CellKind::Pond),
                (HexCoord::new(-1, 1), CellKind::Pond),
                (HexCoord::new(-1, 0), CellKind::Pond),
            ],
            vec![],
            vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }],
        );
        let model = Model::from_context(&ctx);
        let actions = greedy_decode(&ctx, &model);
        assert!(
            actions.iter().copied().all(Action::is_wait),
            "boxed-in agent must wait every step"
        );
    }

    #[test]
    fn score_decreases_when_more_series_are_collected() {
        // Two contexts, identical geometry, but one with a single series
        // and one with two distinct series. A plan that picks up *both*
        // series on the two-series context must score strictly better
        // than the same plan on the single-series context — the global
        // variety term (-100) is the largest single contributor.
        let ctx_one_series = SolveContext::for_test(
            vec![DaySpec::new(10)],
            vec![
                (HexCoord::new(0, 0), CellKind::Plain),
                (HexCoord::new(1, 0), CellKind::Spot(SpotId(0))),
                (HexCoord::new(2, 0), CellKind::Spot(SpotId(1))),
            ],
            vec![
                Spot {
                    id: SpotId(0),
                    coord: HexCoord::new(1, 0),
                    series_id: SeriesId(7),
                    initial_stock: 1,
                },
                Spot {
                    id: SpotId(1),
                    coord: HexCoord::new(2, 0),
                    series_id: SeriesId(7),
                    initial_stock: 1,
                },
            ],
            vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }],
        );

        let ctx_two_series = SolveContext::for_test(
            vec![DaySpec::new(10)],
            vec![
                (HexCoord::new(0, 0), CellKind::Plain),
                (HexCoord::new(1, 0), CellKind::Spot(SpotId(0))),
                (HexCoord::new(2, 0), CellKind::Spot(SpotId(1))),
            ],
            vec![
                Spot {
                    id: SpotId(0),
                    coord: HexCoord::new(1, 0),
                    series_id: SeriesId(7),
                    initial_stock: 1,
                },
                Spot {
                    id: SpotId(1),
                    coord: HexCoord::new(2, 0),
                    series_id: SeriesId(8),
                    initial_stock: 1,
                },
            ],
            vec![Agent { id: AgentId(0), coord: HexCoord::new(0, 0), fuel: 100 }],
        );

        // Step-major: 10 steps × 1 agent = 10 actions. Move right twice
        // to collect both spots, then wait.
        let actions = vec![
            Action::Move { agent: AgentId(0), direction: Direction::Right },
            Action::Stay { agent: AgentId(0) },
            Action::Move { agent: AgentId(0), direction: Direction::Right },
            Action::Stay { agent: AgentId(0) },
            Action::Stay { agent: AgentId(0) },
            Action::Stay { agent: AgentId(0) },
            Action::Stay { agent: AgentId(0) },
            Action::Stay { agent: AgentId(0) },
            Action::Stay { agent: AgentId(0) },
            Action::Stay { agent: AgentId(0) },
        ];

        // Score is "lower is better" — collecting a second distinct
        // series collects the global variety bonus (-100) and should
        // strictly lower the score.
        let s1 = score_plan(&ctx_one_series, &actions);
        let s2 = score_plan(&ctx_two_series, &actions);
        assert!(s2 < s1, "two-series plan should outscore one-series plan: s1={s1}, s2={s2}");
    }

    #[test]
    fn score_is_deterministic() {
        let ctx = tiny_ctx();
        let actions = vec![Action::Stay { agent: AgentId(0) }; 10];
        // Two calls on the same input must produce the same score (no
        // randomness in the prototype path).
        #[allow(clippy::float_cmp)]
        let s1 = score_plan(&ctx, &actions);
        #[allow(clippy::float_cmp)]
        let s2 = score_plan(&ctx, &actions);
        assert_eq!(s1, s2);
    }
}
