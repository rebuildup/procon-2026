//! Anytime solver: use the full answer window to improve the plan (Issue #340).
//!
//! Starts from the current best candidate (park=far) and generates variants.
//! Scores by the official lexicographic axes mapped onto one local day:
//! full-period V (axis 1), current-day series (axis 2, D), current-day
//! balls (axis 3, B), plus a diagnostic next-day positioning value.
//!
//! Unlike `search`, this is a true anytime algorithm:
//! - Initial solution is the strongest candidate (park=far)
//! - Variants explore assignment/ordering changes, not just cost jitter
//! - Acceptance is dictionary-order (V never drops; then D, then B) so a
//!   B gain can be kept only when D is maintained
//! - The final return re-checks `best` against the initial plan in the
//!   same order, so an accepted improvement always reaches the returned
//!   `AgentActions` (Issue #354 guard fix)
//!
//! # Offline vs live stopping rules
//!
//! - **Live** (`budget_ms`): wall-clock cap per day plan. The stop is
//!   time-based, so the *number* of explored variants (and therefore the
//!   returned plan) varies with CPU load. Use this in a real match, where
//!   the answer window is wall-clock. Prefer `budget_ms=50000` to leave
//!   margin inside the 60s day window.
//! - **Offline** (`trials`): exact trial-count budget. Same candidate id +
//!   seed always explores the same variant sequence and returns the same
//!   plan, independent of load / `--jobs`. Use this for every recorded
//!   measurement so evidence is reproducible.
//! - If both are set, `trials` wins and the wall clock is ignored.
use crate::adapters::coverage_solver::CoverageCandidate;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::MatchSim;
use engine::model::{AgentActions, AgentKind, Solver, SolverContext};
use std::time::{Duration, Instant};

pub struct AnytimeFactory;
const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "budget_ms",
        required: false,
        doc: "LIVE wall-clock cap per day plan (nondeterministic under CPU load): \
              0 = off | 1..59000 (live: 50000 leaves margin)",
    },
    ParamSpec {
        key: "trials",
        required: false,
        doc: "OFFLINE deterministic trial-count budget: exact N variants explored, \
              same id+seed always reproduces. Overrides budget_ms when set.",
    },
    ParamSpec { key: "seed", required: false, doc: "exploration seed (default 42)" },
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
];

impl CandidateFactory for AnytimeFactory {
    fn algorithm(&self) -> &'static str {
        "anytime"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "Anytime search from park=far; accept only strict series gains"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("anytime requires budget=official".into()));
        }
        // Strict parsing: reject invalid values explicitly (Issue #342).
        // Silent fallback to defaults is forbidden.
        let budget_ms = match spec.param("budget_ms") {
            None => 0u32,
            Some(s) => s
                .parse::<u32>()
                .map_err(|_| CandidateError::Build(format!("invalid budget_ms: {s:?}")))?,
        };
        if budget_ms > 59000 {
            return Err(CandidateError::Build(format!(
                "budget_ms must be 0..=59000, got {budget_ms}"
            )));
        }
        let trials = match spec.param("trials") {
            None => 0u32,
            Some(s) => s
                .parse::<u32>()
                .map_err(|_| CandidateError::Build(format!("invalid trials: {s:?}")))?,
        };
        let seed = match spec.param("seed") {
            None => 42u64,
            Some(s) => s
                .parse::<u64>()
                .map_err(|_| CandidateError::Build(format!("invalid seed: {s:?}")))?,
        };
        Ok(Box::new(AnytimeCandidate { budget_ms, trials, seed }))
    }
}

struct AnytimeCandidate {
    /// Live wall-clock budget in ms. Ignored when `trials > 0`.
    budget_ms: u32,
    /// Offline deterministic trial-count budget. `0` = use `budget_ms`.
    trials: u32,
    seed: u64,
}

/// Score a plan for anytime search (Issue #342, acceptance #354).
/// Returns (`full_variety`, `series_today`, `balls_today`, `next_day_value`).
///
/// The official ranking is lexicographic V → D → B → -response time. The
/// tuple maps onto it locally:
///
/// - `full_variety` (official axis 1, V): `series_taken` after simulating
///   this plan. Included so acceptance can never trade a previously
///   acquired series away for a today-only gain.
/// - `series_today` (official axis 2, D): today's contribution to
///   `daily_variety_sum`. This MUST NOT decrease.
/// - `balls_today` (official axis 3, B): total inventory after today.
/// - `next_day_value`: heuristic for future positioning, tie-breaker only.
///
/// Today's ending position decides tomorrow's reachable series, so a
/// positioning-driven acceptance is safe only as the last tie-break after
/// V, D and B are all preserved.
#[allow(
    clippy::cast_possible_truncation,
    reason = "acquired-series count fits in u32 on any supported board"
)]
fn score_plan(ctx: &SolverContext<'_>, plan: &AgentActions) -> (u32, u32, u32, u32) {
    let mut state = ctx.state.clone();
    let sim = MatchSim::default();
    for step in plan.actions.chunks(ctx.agent_ids.len()) {
        sim.step(&mut state, &AgentActions::new(step.to_vec()));
    }
    let series_today = state.series_acquired_by_day.get(&ctx.day).map_or(0, |s| s.len() as u32);
    let balls_today: u32 = state.agents.iter().map(engine::Agent::total_inventory).sum();

    // Next-day positioning: fuel remaining + proximity to ALL series (not just
    // unacquired, since tomorrow needs to re-acquire them).
    let fuel_value: u32 =
        state.agents.iter().filter(|a| a.kind == AgentKind::Patrol).map(|a| a.fuel.min(50)).sum();

    let mut proximity: u32 = 0;
    for c in state.setting.map.cells() {
        if c.spot.is_some() {
            for agent in &state.agents {
                if agent.kind == AgentKind::Patrol {
                    let dist = crate::adapters::walk::hex_distance(agent.pos, c.coord);
                    if dist < 15 {
                        proximity += 15 - dist;
                    }
                }
            }
        }
    }

    // Official axis 1 (V): full-period variety including this plan's effect.
    // Re-acquiring a series cannot remove it from `series_taken`, so this
    // only drops if a plan could somehow lose an earlier acquisition — the
    // engine cannot do that, but the guard keeps axis-1 safety local.
    let full_variety = state.series_taken.len() as u32;

    (full_variety, series_today, balls_today, fuel_value + proximity)
}

/// Generate a variant of park=far with a different exploration seed.
fn generate_variant(ctx: &SolverContext<'_>, seed: u64) -> AgentActions {
    // Use park=far with a seed-dependent variant.
    let candidate = CoverageCandidate { park_risk: true, variant: seed, fuel_aware: false };
    candidate.decide(ctx)
}

impl Solver for AnytimeCandidate {
    fn name(&self) -> &'static str {
        "anytime"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        // Initial solution: park=far (current best). Byte-identical to the
        // `coverage:budget=official:park=far` baseline, so returning it
        // unchanged cannot score below that candidate's full-match D.
        let initial =
            CoverageCandidate { park_risk: true, variant: 0, fuel_aware: false }.decide(ctx);
        let initial_score = score_plan(ctx, &initial);
        let mut best = initial.clone();
        let mut best_score = initial_score;

        // If no budget at all, return the initial solution.
        if self.budget_ms == 0 && self.trials == 0 {
            return best;
        }

        // Offline mode (`trials > 0`): exact trial count, ZERO wall-clock
        // reads (`Instant::now()` exists only in the live branch below), so
        // the same id+seed always returns the same plan (reproducible
        // measurements). Live mode (`budget_ms`): wall-clock stop — the
        // explored variant count varies with CPU load, which is fine in a
        // match and wrong for recorded evidence.
        let offline = self.trials > 0;
        // `None` in offline mode: no wall clock anywhere on the offline path.
        let deadline = if offline {
            None
        } else {
            let started = Instant::now();
            let live_window = (ctx.remaining_ms != u32::MAX)
                .then(|| Duration::from_millis(u64::from(ctx.remaining_ms.saturating_sub(5_000))));
            let search_budget = Duration::from_millis(u64::from(self.budget_ms));
            Some(match live_window {
                Some(live) => started + live.min(search_budget),
                None => started + search_budget,
            })
        };

        // Deterministic anytime exploration (Issue #342).
        // Single-threaded with fixed trial order ensures reproducibility
        // regardless of --jobs parallelism.
        let mut trial = 1u64;
        let mut no_improve_count = 0u64;

        loop {
            if offline {
                if trial > u64::from(self.trials) {
                    break;
                }
            } else if Instant::now() >= deadline.expect("live mode has a wall-clock deadline") {
                break;
            }
            // Generate variant with deterministic seed.
            let plan = generate_variant(ctx, self.seed.wrapping_add(trial));
            let score = score_plan(ctx, &plan);

            // Dictionary-order acceptance (Issue #342, guard fix #354).
            // Official ranking is lexicographic V → D → B. Locally:
            // full_variety guards axis 1, series_today is today's D
            // contribution, balls_today is today's B contribution.
            //
            // Accept only when V (axis 1) does not decrease AND the plan is
            // strictly better in the local dictionary order:
            //   1. series_today increases (today's D contribution grows), or
            //   2. series_today is equal and balls_today increases
            //      (today's B improves with D maintained), or
            //   3. V/D/B are all equal and next-day positioning improves.
            // Never accept when series_today decreases: today's ending
            // position decides tomorrow's reachable series, and a B gain
            // bought with a D drop lowers full-match D (Σ per-day varieties).
            let v_safe = score.0 >= best_score.0;
            let d_gain = score.1 > best_score.1;
            let d_same_b_gain = score.1 == best_score.1 && score.2 > best_score.2;
            let tie_better_pos =
                score.1 == best_score.1 && score.2 == best_score.2 && score.3 > best_score.3;
            if v_safe && (d_gain || d_same_b_gain || tie_better_pos) {
                best = plan;
                best_score = score;
                no_improve_count = 0;
            } else {
                no_improve_count += 1;
            }

            trial += 1;

            // Early stop: no improvement for many iterations. Count-based,
            // so it stays deterministic in offline mode too.
            if no_improve_count > 500 {
                break;
            }
        }

        // Final return guard (Issue #354). Acceptance above is *internal*:
        // without this check a B-only improvement (series same, balls
        // higher) sat in `best` while the returned value stayed `initial`,
        // so the improvement never reached the answer. Compare `best`
        // against the initial plan in the same local dictionary order and
        // return `best` only when it is strictly better; the acceptance
        // loop guarantees `best_score == initial_score` implies
        // `best == initial`, so no plan can regress here.
        if best_score > initial_score {
            best
        } else {
            // No improvement (or regression): return the untouched baseline.
            // Full-match D then matches the park=far baseline.
            initial
        }
    }
}
