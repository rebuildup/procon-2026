//! Anytime solver: use the full answer window to improve the plan (Issue #340).
//!
//! Starts from the current best candidate (park=far) and generates variants.
//! Scores by current-day series (the only axis that can replace the initial
//! plan) plus diagnostic ball / next-day positioning values.
//!
//! Unlike `search`, this is a true anytime algorithm:
//! - Initial solution is the strongest candidate (park=far)
//! - Variants explore assignment/ordering changes, not just cost jitter
//! - Acceptance is series-only so full-match D cannot be traded for B
//! - Returns immediately when no series improvement is found
use crate::adapters::coverage_solver::CoverageCandidate;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{AgentActions, AgentKind, Solver, SolverContext};
use engine::MatchSim;
use std::time::{Duration, Instant};

pub struct AnytimeFactory;
const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        key: "budget_ms",
        required: false,
        doc: "wall-clock cap per day plan: 0 = off (deterministic park=far) | \
              1..59000 explores with best-improvement (live: 50000 leaves margin)",
    },
    ParamSpec {
        key: "seed",
        required: false,
        doc: "exploration seed (default 42)",
    },
    ParamSpec {
        key: "budget",
        required: true,
        doc: "official (required)",
    },
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
            Some(s) => s.parse::<u32>().map_err(|_| {
                CandidateError::Build(format!("invalid budget_ms: {s:?}"))
            })?,
        };
        if budget_ms > 59000 {
            return Err(CandidateError::Build(format!(
                "budget_ms must be 0..=59000, got {budget_ms}"
            )));
        }
        let seed = match spec.param("seed") {
            None => 42u64,
            Some(s) => s.parse::<u64>().map_err(|_| {
                CandidateError::Build(format!("invalid seed: {s:?}"))
            })?,
        };
        Ok(Box::new(AnytimeCandidate { budget_ms, seed }))
    }
}

struct AnytimeCandidate {
    budget_ms: u32,
    seed: u64,
}

/// Score a plan for anytime search (Issue #342).
/// Returns (`series_today`, `balls_today`, `next_day_value`).
///
/// CRITICAL: `series_today` maps to official D (`daily_variety_sum`).
/// This MUST NOT decrease. `balls_today` maps to official B.
/// `next_day_value` is a heuristic for future positioning.
///
/// Only `series_today` is allowed to replace the initial park=far plan.
/// Today's ending position decides tomorrow's reachable series, so a
/// ball-only gain (or a series gain bought with a collapsed tomorrow)
/// can lower full-match D — the sum of per-day varieties.
fn score_plan(ctx: &SolverContext<'_>, plan: &AgentActions) -> (u32, u32, u32) {
    let mut state = ctx.state.clone();
    let sim = MatchSim::default();
    for step in plan.actions.chunks(ctx.agent_ids.len()) {
        sim.step(&mut state, &AgentActions::new(step.to_vec()));
    }
    let series_today = state
        .series_acquired_by_day
        .get(&ctx.day)
        .map_or(0, |s| s.len() as u32);
    let balls_today: u32 = state.agents.iter().map(|a| a.total_inventory()).sum();

    // Next-day positioning: fuel remaining + proximity to ALL series (not just
    // unacquired, since tomorrow needs to re-acquire them).
    let fuel_value: u32 = state
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Patrol)
        .map(|a| a.fuel.min(50))
        .sum();

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

    (series_today, balls_today, fuel_value + proximity)
}

/// Generate a variant of park=far with a different exploration seed.
fn generate_variant(ctx: &SolverContext<'_>, seed: u64) -> AgentActions {
    // Use park=far with a seed-dependent variant.
    let candidate = CoverageCandidate {
        park_risk: true,
        variant: seed,
        fuel_aware: false,
    };
    candidate.decide(ctx)
}

impl Solver for AnytimeCandidate {
    fn name(&self) -> &'static str {
        "anytime"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let started = Instant::now();

        // Initial solution: park=far (current best). Byte-identical to the
        // `coverage:budget=official:park=far` baseline, so returning it
        // unchanged cannot score below that candidate's full-match D.
        let initial = CoverageCandidate { park_risk: true, variant: 0, fuel_aware: false }.decide(ctx);
        let initial_score = score_plan(ctx, &initial);
        let mut best = initial.clone();
        let mut best_score = initial_score;

        // If no budget, return the initial solution.
        if self.budget_ms == 0 {
            return best;
        }

        // Calculate deadline: respect both explicit budget and live window.
        let live_window = (ctx.remaining_ms != u32::MAX)
            .then(|| Duration::from_millis(u64::from(ctx.remaining_ms.saturating_sub(5_000))));
        let search_budget = Duration::from_millis(u64::from(self.budget_ms));
        let deadline = match live_window {
            Some(live) => started + live.min(search_budget),
            None => started + search_budget,
        };

        // Deterministic anytime exploration (Issue #342).
        // Single-threaded with fixed trial order ensures reproducibility
        // regardless of --jobs parallelism.
        let mut trial = 1u64;
        let mut no_improve_count = 0u64;

        while Instant::now() < deadline {
            // Generate variant with deterministic seed.
            let plan = generate_variant(ctx, self.seed.wrapping_add(trial));
            let score = score_plan(ctx, &plan);

            // Series-only acceptance (Issue #342 Phase 3):
            // replace the plan only when series_today strictly increases.
            // Ball-only gains are rejected: today's ending position decides
            // tomorrow's reachable series, so trading positioning for B can
            // lower full-match D (Σ per-day varieties). A series gain is the
            // only local signal that maps 1:1 onto a D increase today.
            if score.0 > best_score.0 {
                best = plan;
                best_score = score;
                no_improve_count = 0;
            } else {
                no_improve_count += 1;
            }

            trial += 1;

            // Early stop: no improvement for many iterations.
            if no_improve_count > 500 {
                break;
            }
        }

        // If no variant beat the initial series count, return the initial
        // park=far plan untouched. Full-match D then matches the baseline.
        if best_score.0 <= initial_score.0 {
            return initial;
        }
        best
    }
}
