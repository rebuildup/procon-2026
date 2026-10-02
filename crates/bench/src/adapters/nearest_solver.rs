//! `nearest` candidate — series round-robin assignment from
//! `crates/solver-greedy`'s [`NearestSpotGreedy`].
//!
//! # Hypothesis (Issue #189)
//!
//! `greedy` ranks candidate spots primarily by distance, so on a multi-series
//! map the fleet collapses onto whichever series happens to be closest and
//! `distinct_series_today` (tier 1) stalls. [`NearestSpotGreedy`] instead
//! commits each agent to one series up front — `agent[i] → series[i % n]`,
//! round-robin over the sorted distinct series — so a small fleet covers many
//! series in parallel. The adapter wires that assignment policy into the
//! canonical evaluator; scoring still comes from [`engine::MatchSim`] alone.
//!
//! # How the adapter works
//!
//! Identical day contract to [`super::greedy_solver`] — that is deliberate:
//! ask the solver **once** at day start for a target spot per agent, then
//! walk each agent there with [`super::walk::plan_walk`] for the rest of the
//! day. Combined with the shared [`super::context`] conversion, `nearest` and
//! `greedy` differ *only* in how a target spot is chosen, so any evaluation
//! gap isolates the assignment mechanism.
//!
//! # Parameters
//!
//! Bare id only today (`nearest`), wired to the basic
//! [`NearestSpotGreedy::new`] variant. The solver's other construction knob,
//! [`NearestSpotGreedy::fuel_aware`] (skip spots beyond the agent's remaining
//! fuel), is deliberately *not* exposed yet: `greedy` also picks its target
//! without a fuel-budget filter, so keeping `nearest` fuel-agnostic makes the
//! comparison isolate the assignment mechanism. Per Issue #189 a follow-up
//! child may add `nearest:fuel_aware=…` without changing the id grammar.

use engine::model::{AgentActions, Solver, SolverContext};
use solver_greedy::{NearestSpotGreedy, Solver as _};

use super::context;
use super::walk::plan_walk;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};

/// Parameter schema. Empty today — see the module docs for why `fuel_aware`
/// is held back for a child candidate.
pub const PARAMS: &[ParamSpec] = &[];

/// `nearest` candidate: round-robin series assignment, then walk.
pub struct NearestCandidate {
    inner: NearestSpotGreedy,
}

impl NearestCandidate {
    fn new() -> Self {
        Self { inner: NearestSpotGreedy::new() }
    }
}

impl Solver for NearestCandidate {
    fn name(&self) -> &'static str {
        "nearest"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let gctx = context::solve_context(ctx);
        let intents = self.inner.decide(&gctx);
        let targets = context::targets_from_intents(ctx, &intents);
        plan_walk(ctx, &targets)
    }
}

/// Factory for [`NearestCandidate`].
pub struct NearestFactory;

impl CandidateFactory for NearestFactory {
    fn algorithm(&self) -> &'static str {
        "nearest"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn describe(&self) -> &'static str {
        "Series round-robin greedy: agent[i] is assigned series[i % n] and \
         heads for the nearest spot in that series. Same day contract as \
         `greedy`; only the assignment differs."
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        Ok(Box::new(NearestCandidate::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{CandidateRegistry, CandidateSpec};
    use domain::{MatchId, PlayerId};
    use engine::model::{
        ActionKind, Agent, HexCoord, MapGrid, MapSize, MatchSetting, MatchState, SolverContext,
        Spot, Terrain,
    };

    #[test]
    fn bare_nearest_is_registered_and_round_trips() {
        let reg = CandidateRegistry::builtin();
        let candidate = reg.build("nearest").expect("nearest is builtin");
        // `parse → id` is exact, so a durable record matches its command.
        assert_eq!(CandidateSpec::parse("nearest").unwrap().id(), "nearest");
        assert_eq!(candidate.id(), "nearest");
        assert_eq!(candidate.algorithm(), "nearest");
        assert!(candidate.build_solver().is_ok());
    }

    #[test]
    fn factory_rejects_unknown_params() {
        let reg = CandidateRegistry::builtin();
        assert!(matches!(
            reg.build("nearest:fuel_aware=true"),
            Err(CandidateError::UnknownParameter { .. })
        ));
    }

    /// The whole point of the candidate: on a two-series map, `nearest`
    /// commits agent 0 to the *round-robin* series rather than the closest
    /// one, and therefore walks a different target than `greedy` would.
    ///
    /// Agent 0 starts at (0,0). Series 1's spot is far away at (6,0) and
    /// series 2's spot is adjacent at (1,0). `nearest` assigns agent 0 to
    /// series 1 (the lowest present series id), so it walks all four steps;
    /// `greedy` takes the adjacent spot and then waits.
    #[test]
    fn nearest_walks_the_round_robin_series_not_the_closest_spot() {
        let st = two_series_state();
        let ctx = SolverContext::from_state(&st);

        let nearest = NearestCandidate::new().decide(&ctx);
        assert!(
            nearest.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
            "plan must be acceptable to MatchSim"
        );
        let nearest_agent0: Vec<_> = nearest.actions.iter().step_by(3).map(|a| a.kind).collect();
        assert_eq!(nearest_agent0.len(), 4);
        assert!(
            nearest_agent0.iter().all(|k| matches!(k, ActionKind::Move(_))),
            "expected a full walk toward the assigned series, got {nearest_agent0:?}"
        );

        // Contrast with `greedy` — the two candidates must not be the same
        // wiring, otherwise this adapter proves nothing about assignment.
        let greedy_solver = CandidateRegistry::builtin()
            .build("greedy")
            .expect("greedy is builtin")
            .build_solver()
            .expect("greedy builds");
        let greedy = greedy_solver.decide(&ctx);
        let greedy_agent0: Vec<_> = greedy.actions.iter().step_by(3).map(|a| a.kind).collect();
        assert!(
            greedy_agent0.iter().any(|k| matches!(k, ActionKind::Wait)),
            "greedy is expected to stop on the adjacent spot, got {greedy_agent0:?}"
        );
        assert_ne!(
            format!("{nearest_agent0:?}"),
            format!("{greedy_agent0:?}"),
            "nearest must not be a duplicate of greedy"
        );
    }

    /// Two series on an 8x8 map: series 1 far at (6,0), series 2 adjacent
    /// to the origin at (1,0). Three agents (the team minimum) on plain
    /// cells, each with `day_steps * 3 = 12` fuel — enough for either target.
    fn two_series_state() -> MatchState {
        let size = MapSize::new(8, 8).unwrap();
        let cells = (0..8)
            .flat_map(|row| {
                (0..8).map(move |col| {
                    let coord = HexCoord::new(col, row);
                    let spot = match (col, row) {
                        (6, 0) => {
                            Some(Spot { series: 1, pos: coord, max_stock: 3, current_stock: 3 })
                        }
                        (1, 0) => {
                            Some(Spot { series: 2, pos: coord, max_stock: 3, current_stock: 3 })
                        }
                        _ => None,
                    };
                    engine::model::Cell { coord, terrain: Terrain::Plain, spot }
                })
            })
            .collect();
        MatchState::new(MatchSetting {
            match_id: MatchId::new("nearest-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map: MapGrid::new(size, cells).unwrap(),
            agents: vec![
                Agent::patrol(0, HexCoord::new(0, 0), 4, 3),
                Agent::patrol(1, HexCoord::new(4, 4), 4, 3),
                Agent::patrol(2, HexCoord::new(7, 7), 4, 3),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap()
    }
}
