//! Anytime solver: use the full answer window to improve the plan (Issue #340).
//!
//! Starts from the current best candidate (park=far) and generates variants.
//! Scores by current-day axes plus next-day positioning (fuel + proximity).
//! Always returns the best valid plan found, even if interrupted.
//!
//! Unlike `search`, this is a true anytime algorithm:
//! - Initial solution is the strongest candidate (park=far)
//! - Variants explore assignment/ordering changes, not just cost jitter
//! - Scoring includes next-day positioning value
//! - Returns immediately when no improvement is found
use crate::adapters::coverage_solver::CoverageCandidate;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{AgentActions, AgentKind, HexCoord, Solver, SolverContext};
use engine::MatchSim;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
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
        "Anytime search from park=far with next-day positioning score"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("anytime requires budget=official".into()));
        }
        let budget_ms = spec
            .param("budget_ms")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let seed = spec.param("seed").and_then(|s| s.parse::<u64>().ok()).unwrap_or(42);
        Ok(Box::new(AnytimeCandidate { budget_ms, seed }))
    }
}

struct AnytimeCandidate {
    budget_ms: u32,
    seed: u64,
}

/// Score a plan: (series_today, balls_today, next_day_value).
/// next_day_value rewards fuel remaining and proximity to unvisited series.
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
    
    // Next-day positioning: fuel remaining + proximity to unvisited series.
    let fuel_value: u32 = state
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Patrol)
        .map(|a| a.fuel.min(100)) // cap to avoid dominating
        .sum();
    
    // Proximity: reward being close to spots of series not yet acquired today.
    let acquired: std::collections::BTreeSet<u32> = state
        .series_acquired_by_day
        .get(&ctx.day)
        .cloned()
        .unwrap_or_default();
    let mut proximity: u32 = 0;
    for c in state.setting.map.cells() {
        if let Some(spot) = &c.spot {
            if !acquired.contains(&spot.series) {
                // Reward agents being close to this spot.
                for agent in &state.agents {
                    if agent.kind == AgentKind::Patrol {
                        let dist = crate::adapters::walk::hex_distance(agent.pos, c.coord);
                        if dist < 10 {
                            proximity += 10 - dist;
                        }
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
        
        // Initial solution: park=far (current best).
        let mut best = CoverageCandidate { park_risk: true, variant: 0, fuel_aware: false }.decide(ctx);
        let mut best_score = score_plan(ctx, &best);
        
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
        
        // Anytime exploration: generate variants and keep the best.
        let threads = std::thread::available_parallelism().map_or(1, usize::from).clamp(1, 8);
        let trial_counter = AtomicU64::new(1);
        let stop = AtomicBool::new(false);
        let global_best: Mutex<Option<(u64, (u32, u32, u32), AgentActions)>> = Mutex::new(None);
        
        // Store initial solution as the baseline.
        *global_best.lock().unwrap() = Some((0, best_score, best.clone()));
        
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    let mut local_best: Option<(u64, (u32, u32, u32), AgentActions)> = None;
                    let mut iterations = 0u64;
                    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                        let trial = trial_counter.fetch_add(1, Ordering::Relaxed);
                        iterations += 1;
                        
                        // Generate variant with different seed.
                        let plan = generate_variant(ctx, self.seed.wrapping_add(trial));
                        let score = score_plan(ctx, &plan);
                        
                        // Keep if better than local best.
                        if local_best.as_ref().is_none_or(|(_, s, _)| score > *s) {
                            local_best = Some((trial, score, plan));
                            
                            // Update global best.
                            let mut global = global_best.lock().unwrap();
                            if global.as_ref().is_none_or(|(_, s, _)| score > *s) {
                                *global = Some((trial, score, local_best.as_ref().unwrap().2.clone()));
                            }
                        }
                        
                        // Stop if no improvement for many iterations (converged).
                        if iterations > 1000 && local_best.is_some() {
                            let global = global_best.lock().unwrap();
                            if let Some((_, global_score, _)) = global.as_ref() {
                                if local_best.as_ref().unwrap().1 == *global_score {
                                    // No improvement for 1000 iterations, stop.
                                    stop.store(true, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                });
            }
        });
        
        // Return the global best.
        let result = global_best.lock().unwrap();
        if let Some((_, _, plan)) = result.as_ref() {
            plan.clone()
        } else {
            best
        }
    }
}
