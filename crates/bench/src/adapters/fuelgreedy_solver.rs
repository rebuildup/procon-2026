//! `fuelgreedy` candidate — `solver-greedy::FuelTrafficGreedy`, wired into the
//! canonical evaluator as the *fuel-economy* mechanism of the greedy family.
//!
//! # Day contract (identical to `greedy`)
//!
//! Same shape as [`super::greedy_solver::GreedyCandidate`]: ask the solver
//! once at day start for one intent per agent, map the intents to per-agent
//! target lists via [`super::context`], then walk with
//! [`super::walk::plan_walk`]. Both candidates therefore share the world view,
//! the spot-id packing, and the walk; the *only* difference under comparison
//! is the assignment policy inside `FuelTrafficGreedy`.
//!
//! # What is actually being tested (Issue #192 non-scope split)
//!
//! `FuelTrafficGreedy` scores a candidate spot as
//! `distance + traffic_weight × Σ traffic_penalty`, branching on the agent's
//! fuel: below [`DEFAULT_FUEL_LOW_THRESHOLD`] it drops the traffic term and
//! heads for the nearest spot (refuel first), otherwise it keeps the full
//! score.
//!
//! This adapter deliberately feeds **no traffic data**: the traffic map is
//! empty, so every `Σ traffic_penalty` evaluates to `0` and the score
//! collapses to pure distance on both branches. What this candidate isolates
//! is therefore the **fuel-threshold switching + distance term** — exactly
//! the Issue #192 In/Out split. Traffic-weight effects are out of scope here
//! (they need #187 fixtures / a multi-day corpus) and are left to a later
//! child candidate.
//!
//! `crates/solver-greedy` itself is untouched.

use std::collections::HashMap;

use engine::model::{AgentActions, HexCoord, Solver, SolverContext};
use solver_greedy::{
    DEFAULT_FUEL_LOW_THRESHOLD, FuelTrafficGreedy, FuelTrafficSolveContext, FuelTrafficSolver as _,
    HexCoord as GreedyCoord, TrafficState,
};

use super::context;
use super::walk::plan_walk;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};

/// Parameter schema. Empty today — this candidate pins
/// [`DEFAULT_FUEL_LOW_THRESHOLD`] and the default traffic weight, because
/// Issue #192 evaluates the fuel mechanism at the solver's defaults.
/// `fuelgreedy:traffic_weight=…` / `fuelgreedy:fuel_low=…` are the natural
/// follow-up axes for child candidates; they do not exist yet, so the id
/// grammar rejects them.
pub const PARAMS: &[ParamSpec] = &[];

/// `fuelgreedy`: fuel-aware greedy assignment, then the shared day walk.
pub struct FuelGreedyCandidate {
    /// Per-cell traffic handed to `FuelTrafficGreedy`.
    ///
    /// Always empty in production — see the module docs. Never synthesised:
    /// traffic penalties are explicitly out of this candidate's scope.
    traffic: HashMap<GreedyCoord, TrafficState>,
    inner: FuelTrafficGreedy,
}

impl FuelGreedyCandidate {
    fn new() -> Self {
        Self { traffic: HashMap::new(), inner: FuelTrafficGreedy::new() }
    }

    /// Build `FuelTrafficGreedy`'s world view from the live match state at
    /// day start: fuel per controlled agent, every spot with live stock, and
    /// this candidate's (empty) traffic map.
    ///
    /// The world view itself comes from the shared `context::solve_context`
    /// (same spots / same agents in the same order as `greedy` and
    /// `nearest`); only the context *type* and the fuel/traffic knobs are
    /// specific to this candidate.
    fn solve_context(&self, ctx: &SolverContext<'_>) -> FuelTrafficSolveContext {
        let base = context::solve_context(ctx);
        FuelTrafficSolveContext {
            step: 0,
            start_of_day: true,
            agents: base.agents,
            spots: base.spots,
            traffic: self.traffic.clone(),
            fuel_low_threshold: DEFAULT_FUEL_LOW_THRESHOLD,
        }
    }

    /// One target per controlled agent, indexed like `ctx.agent_ids`.
    fn day_targets(&self, ctx: &SolverContext<'_>) -> Vec<Vec<HexCoord>> {
        let intents = self.inner.decide(&self.solve_context(ctx));
        context::targets_from_intents(ctx, &intents)
    }
}

impl Solver for FuelGreedyCandidate {
    fn name(&self) -> &'static str {
        "fuelgreedy"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        plan_walk(ctx, &self.day_targets(ctx))
    }
}

/// Factory for [`FuelGreedyCandidate`].
pub struct FuelGreedyFactory;

impl CandidateFactory for FuelGreedyFactory {
    fn algorithm(&self) -> &'static str {
        "fuelgreedy"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn describe(&self) -> &'static str {
        "Fuel-aware greedy: an agent below the fuel-low threshold heads for the \
         nearest spot, otherwise the closest spot wins. Traffic is not fed, so \
         only the fuel threshold and distance shape the assignment."
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        Ok(Box::new(FuelGreedyCandidate::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{CandidateError, CandidateRegistry};

    use domain::{MatchId, PlayerId};
    use engine::model::{Agent, Cell, MapGrid, MapSize, MatchSetting, MatchState, Spot, Terrain};

    /// 8x8 plain map with two spots: `(1,0)` (near, series 1) and `(0,3)`
    /// (far, series 2). Agent 0 starts at `(0,0)`.
    fn state(fuel: u32) -> MatchState {
        let size = MapSize::new(8, 8).unwrap();
        let cells = (0..8)
            .flat_map(|row| {
                (0..8).map(move |col| {
                    let coord = HexCoord::new(col, row);
                    let spot = match (col, row) {
                        (1, 0) => {
                            Some(Spot { series: 1, pos: coord, max_stock: 3, current_stock: 3 })
                        }
                        (0, 3) => {
                            Some(Spot { series: 2, pos: coord, max_stock: 3, current_stock: 3 })
                        }
                        _ => None,
                    };
                    Cell { coord, terrain: Terrain::Plain, spot }
                })
            })
            .collect();
        let map = MapGrid::new(size, cells).unwrap();
        let agents = vec![
            Agent::patrol_with_fuel(0, HexCoord::new(0, 0), fuel),
            Agent::supply(1, HexCoord::new(7, 7)),
            Agent::supply(2, HexCoord::new(7, 6)),
        ];
        MatchState::new(MatchSetting {
            match_id: MatchId::new("fuelgreedy-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents,
            day_steps: 8,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap()
    }

    /// Cells the agent crosses on the way to the near spot `(1,0)`.
    fn jammed_near_spot_traffic() -> HashMap<GreedyCoord, TrafficState> {
        let mut t = HashMap::new();
        t.insert(GreedyCoord::new(1, 0), TrafficState::Jammed);
        t
    }

    fn first_target(candidate: &FuelGreedyCandidate, st: &MatchState) -> HexCoord {
        let ctx = SolverContext::from_state(st);
        let targets = candidate.day_targets(&ctx);
        assert!(!targets[0].is_empty(), "agent 0 must receive a target");
        targets[0][0]
    }

    /// Registration / id round-trip: the bare id builds, is rejected with a
    /// parameter, and round-trips through the registry unchanged.
    #[test]
    fn registry_builds_bare_fuelgreedy_and_rejects_unknown_params() {
        let reg = CandidateRegistry::builtin();
        let c = reg.build("fuelgreedy").expect("fuelgreedy must be registered");
        assert_eq!(c.id(), "fuelgreedy");
        assert_eq!(c.algorithm(), "fuelgreedy");
        assert!(c.build_solver().is_ok());
        assert!(matches!(
            reg.build("fuelgreedy:traffic_weight=4"),
            Err(CandidateError::UnknownParameter { .. })
        ));
        assert!(reg.algorithms().contains(&"fuelgreedy"));
    }

    /// The production candidate carries no traffic data at all, so traffic
    /// penalties are structurally zero and only the fuel threshold + distance
    /// can differ from `greedy`.
    #[test]
    fn production_candidate_feeds_no_traffic_and_pins_the_default_threshold() {
        let c = FuelGreedyCandidate::new();
        assert!(c.traffic.is_empty(), "traffic must never be synthesised");
        assert_eq!(c.inner.fuel_low_threshold(), DEFAULT_FUEL_LOW_THRESHOLD);
        assert_eq!(DEFAULT_FUEL_LOW_THRESHOLD, 3);
    }

    /// Behavioural: with a jammed near spot, the **low-fuel** agent
    /// (fuel = 2 < threshold 3) drops the traffic term and takes the nearest
    /// spot, while the same world with a **high-fuel** agent keeps the traffic
    /// term and takes the farther smooth spot. This is the Issue #192
    /// mechanism, observed through the adapter's own context construction.
    #[test]
    fn fuel_low_branch_fires_and_switches_the_chosen_spot() {
        let traffic = jammed_near_spot_traffic();

        // Low fuel: refuel first — nearest spot at distance 1, jammed or not.
        let low = FuelGreedyCandidate { traffic: traffic.clone(), inner: FuelTrafficGreedy::new() };
        assert_eq!(
            first_target(&low, &state(2)),
            HexCoord::new(1, 0),
            "fuel below the threshold must pick the nearest spot, ignoring traffic"
        );

        // Sufficient fuel: distance 1 + jammed penalty 3 = 4 loses to the
        // smooth spot at distance 3.
        let high = FuelGreedyCandidate { traffic, inner: FuelTrafficGreedy::new() };
        assert_eq!(
            first_target(&high, &state(100)),
            HexCoord::new(0, 3),
            "high fuel must keep the traffic term and prefer the smooth spot"
        );
    }

    /// Same world, production (empty) traffic: distance alone decides for a
    /// high-fuel agent — the near spot wins because nothing penalises it.
    #[test]
    fn empty_traffic_collapses_to_nearest_spot_for_high_fuel() {
        let c = FuelGreedyCandidate::new();
        assert_eq!(
            first_target(&c, &state(100)),
            HexCoord::new(1, 0),
            "with no traffic fed, the high-fuel branch is pure distance"
        );
        assert_eq!(
            first_target(&c, &state(2)),
            HexCoord::new(1, 0),
            "and the low-fuel branch picks the same nearest spot"
        );
    }

    /// The full day contract stays well-shaped for every controlled agent.
    #[test]
    fn decide_produces_a_well_shaped_day_plan() {
        let st = state(24);
        let ctx = SolverContext::from_state(&st);
        let actions = FuelGreedyCandidate::new().decide(&ctx);
        assert!(
            actions.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
            "fuelgreedy must produce a well-shaped day plan"
        );
    }
}
