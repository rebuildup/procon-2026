//! Bridge from the Issue #178 candidate registry to the live coordinator.
//!
//! The offline canonical evaluator builds solvers as
//! `Box<dyn engine::model::Solver>` — one day's worth of
//! [`engine::model::AgentActions`] decided from an
//! [`engine::model::MatchState`]. The coordinator loop drives
//! `Box<dyn engine::solver::Solver>` — a [`engine::solver::SolverDecision`]
//! decided from a wire-facing [`engine::solver::SolverContext`].
//!
//! [`CandidateBridge`] implements the wire-facing trait **on top of** an
//! offline candidate, so a selected candidate id such as
//! `multiloop:kmeans=4:seed=42` can be played through the *unchanged*
//! canonical coordinator / `protocol::MatchClient` path (Issue #179 AC —
//! no arena-specific protocol client, no duplicate coordinator).
//!
//! # State model
//!
//! The bridge rebuilds the offline [`engine::model::MatchState`] for every
//! day from two slots the coordinator fills:
//!
//! 1. **engine setting** (Phase 1, shared with `EngineLoopAdapter`) — map,
//!    spots, agent composition, per-day budgets;
//! 2. **live wire state** (Phase 5, `GET /`) — that day's agent positions
//!    and fuel plus current road traffic.
//!
//! Spot stock and inventories follow the simulator's own day-boundary
//! rules: `MatchState::new` refills every spot to its maximum, which is
//! exactly what `phase_spawn` does at a day boundary (Q&A), so a plan for
//! day *N* sees the same stock as an offline `MatchSim` would.
//!
//! Anything that cannot be trusted (setting missing, malformed candidate
//! plan, agent-count mismatch) degrades to `wait_all` with a metadata
//! marker instead of submitting a half-shaped answer — the official rules
//! reject an answer as a whole (Q&A Q6).
//!
//! # Correspondence with offline evidence
//!
//! The candidate id handed in is the **verbatim** registry id (canonical
//! form after [`bench::CandidateRegistry::build`]) and is echoed into the
//! decision metadata and the arena evidence record, which is what lets an
//! arena result be matched against the offline `RunRecord` that selected
//! it.

use std::sync::{Arc, Mutex};

use engine::model::{
    Action as ModelAction, ActionKind as ModelActionKind, Direction as ModelDirection,
    MatchState as ModelMatchState, SolverContext as ModelSolverContext, TrafficState,
};
use engine::solver::{
    Action as WireAction, ActionKind as WireActionKind, Direction as WireDirection, Solver,
    SolverContext, SolverDecision,
};
use protocol::match_config::{MatchState as WireMatchState, TrafficStatus};
use serde_json::json;

/// The two live-state slots the coordinator fills for a candidate run.
///
/// Created together by [`CandidateBridge::new`] and cloned into
/// [`crate::coordinator_loop::CoordinatorLoopConfig`], mirroring how
/// [`crate::solver_adapter::EngineLoopAdapter`] shares its setting slot.
#[derive(Clone, Default)]
pub struct CandidateSlots {
    /// Phase-1 engine setting (map + composition + budgets).
    pub setting: Arc<Mutex<Option<engine::model::MatchSetting>>>,
    /// Per-day live wire state, refreshed in Phase 5.
    pub live_state: Arc<Mutex<Option<WireMatchState>>>,
}

/// Wire-facing wrapper around one offline candidate solver.
pub struct CandidateBridge {
    inner: Box<dyn engine::model::Solver>,
    candidate_id: String,
    slots: CandidateSlots,
}

impl CandidateBridge {
    /// Wrap a built candidate. `candidate_id` must already be the
    /// canonical registry id (what `RunRecord.candidate_id` would carry).
    #[must_use]
    pub fn new(candidate_id: String, inner: Box<dyn engine::model::Solver>) -> Self {
        Self { inner, candidate_id, slots: CandidateSlots::default() }
    }

    /// Build from a registry id, resolving through
    /// [`bench::CandidateRegistry::builtin`] so the live run plays exactly
    /// the configuration the offline evaluator ranks.
    ///
    /// # Errors
    /// The registry's parse/resolve error for an unknown or malformed id.
    pub fn from_registry_id(id: &str) -> Result<Self, bench::CandidateError> {
        let registry = bench::CandidateRegistry::builtin();
        let candidate = registry.build(id)?;
        if registry
            .factory(candidate.algorithm())
            .is_some_and(|factory| factory.params().iter().any(|p| p.key == "budget"))
            && candidate.spec().param("budget") != Some("official")
        {
            return Err(bench::CandidateError::Build("live candidates require explicit budget=official; evaluate that exact id offline first".to_owned()));
        }
        let solver = candidate.build_solver()?;
        Ok(Self::new(candidate.id(), solver))
    }

    /// The canonical candidate id this bridge plays.
    #[must_use]
    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    /// Clone of the shared slots (hand both halves to the loop config).
    #[must_use]
    pub fn slots(&self) -> CandidateSlots {
        self.slots.clone()
    }

    /// Degrade to a well-shaped `wait_all` plan, tagged so the snapshot /
    /// replay metadata shows why.
    fn fallback(&self, ctx: &SolverContext, reason: &str) -> SolverDecision {
        tracing::warn!(candidate = %self.candidate_id, reason, "candidate bridge: degrading to wait_all");
        SolverDecision::wait_all(ctx).with_metadata(
            json!({ "candidate": self.candidate_id, "fallback": "wait_all", "reason": reason }),
        )
    }
}

impl Solver for CandidateBridge {
    fn name(&self) -> &'static str {
        "candidate"
    }

    fn init(&mut self, _ctx: &SolverContext) {
        // State comes from the coordinator's slots per day; nothing to
        // precompute here.
    }

    fn decide(&self, ctx: &SolverContext) -> SolverDecision {
        // A poisoned slot means some other thread panicked while holding it;
        // the guarded value is a plain `Option` with no invariant a panic can
        // break, so take it rather than turning one earlier panic into a hard
        // crash of every later `decide` — the path these `fallback` arms below
        // exist to keep alive.
        let setting =
            self.slots.setting.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let Some(setting) = setting else {
            return self.fallback(ctx, "engine setting not injected yet");
        };
        let live =
            self.slots.live_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();

        let mut state = match ModelMatchState::new(setting) {
            Ok(s) => s,
            Err(e) => return self.fallback(ctx, &format!("engine setting invalid: {e}")),
        };
        state.day = ctx.day;
        state.day_step = 0;
        if let Some(wire) = live.as_ref() {
            if let Err(e) = apply_live_state(&mut state, wire) {
                return self.fallback(ctx, &e);
            }
        }

        let model_ctx = ModelSolverContext {
            state: &state,
            team: &ctx.my_team_id,
            day: ctx.day,
            day_steps: ctx.day_steps,
            agent_ids: ctx.my_agent_ids.clone(),
            // The coordinator's answer window (Q&A Q3), so a time-capped
            // candidate can never out-stay the day deadline (Issue #311).
            remaining_ms: u32::try_from(ctx.remaining_time_ms).unwrap_or(u32::MAX),
        };
        let plan = self.inner.decide(&model_ctx);
        // Log plan structure for rejection diagnosis (Issue #331).
        {
            let n = ctx.my_agent_ids.len();
            let ds = ctx.day_steps as usize;
            let expected = n * ds;
            if plan.actions.len() == expected {
                let mut moves = vec![0usize; n];
                let mut waits = vec![0usize; n];
                for step in 0..ds {
                    for (i, a) in plan.actions[step * n..step * n + n].iter().enumerate() {
                        match a.kind {
                            ModelActionKind::Move(_) => moves[i] += 1,
                            ModelActionKind::Wait => waits[i] += 1,
                        }
                    }
                }
                tracing::info!(
                    candidate = %self.candidate_id,
                    n_agents = n,
                    day_steps = ds,
                    moves = ?moves,
                    waits = ?waits,
                    "plan generated"
                );
            } else {
                tracing::warn!(
                    candidate = %self.candidate_id,
                    n_agents = n,
                    day_steps = ds,
                    actual_len = plan.actions.len(),
                    expected_len = expected,
                    "plan has wrong length"
                );
            }
        }
        if !plan.is_well_shaped(&ctx.my_agent_ids, ctx.day_steps) {
            return self.fallback(ctx, "candidate returned a malformed plan");
        }
        let actions = plan.actions.into_iter().map(wire_action).collect();
        SolverDecision::new(actions)
            .with_metadata(json!({ "candidate": self.candidate_id, "solver": self.inner.name() }))
    }
}

/// Overlay the day's live wire facts onto the freshly built model state.
///
/// - agent positions and fuel come from the server (authoritative);
/// - road traffic comes from the wire `traffics` list;
/// - everything else (map, spots, composition, day budgets) is the Phase-1
///   setting, which is static for the match.
///
/// # Errors
/// A descriptive message when the wire payload contradicts the setting
/// (agent-count mismatch, out-of-board cell index, negative fuel) — the
/// caller degrades to `wait_all` rather than guessing.
fn apply_live_state(state: &mut ModelMatchState, wire: &WireMatchState) -> Result<(), String> {
    apply_live_agents(&mut state.agents, state.setting.map.size, wire)?;
    apply_live_traffic(&mut state.setting.map, wire)
}

/// Overlay the live agent positions / fuel onto an agent list (strict count
/// check — see [`apply_live_state`]).
///
/// Split out so the submission-boundary normalizer
/// (`engine::wire_plan::normalize_day_plan` via `coordinator_loop`) can
/// rebuild the same live view from a bare `MatchSetting` — without paying
/// for (or being rejected by) `MatchState::new`'s full validation.
pub(crate) fn apply_live_agents(
    agents: &mut [engine::model::Agent],
    size: engine::model::MapSize,
    wire: &WireMatchState,
) -> Result<(), String> {
    if wire.agents.len() != agents.len() {
        return Err(format!(
            "server reports {} agents but the setting declared {}",
            wire.agents.len(),
            agents.len()
        ));
    }
    let width = i32::from(size.width);
    let height = i32::from(size.height);
    for (agent, live) in agents.iter_mut().zip(&wire.agents) {
        agent.pos = cell_index(live.pos, width, height)?;
        agent.fuel = u32::try_from(live.fuel)
            .map_err(|_| format!("negative fuel {} for agent {}", live.fuel, agent.id))?;
    }
    Ok(())
}

/// Overlay the live road traffic onto a map (no agent-count requirement —
/// an empty `traffics` list is a no-op, which is what pre-start and
/// test-double payloads carry).
pub(crate) fn apply_live_traffic(
    map: &mut engine::model::MapGrid,
    wire: &WireMatchState,
) -> Result<(), String> {
    let width = i32::from(map.size.width);
    let height = i32::from(map.size.height);
    for traffic in &wire.traffics {
        let coord = cell_index(traffic.pos, width, height)?;
        if let Some(cell) = map.get_mut(coord) {
            if let engine::model::Terrain::Road(current) = &mut cell.terrain {
                *current = traffic_state(traffic.status);
            }
        }
    }
    Ok(())
}

/// Row-major cell index (wire `pos`) → board coordinate.
fn cell_index(pos: i32, width: i32, height: i32) -> Result<engine::model::HexCoord, String> {
    let width = width.max(1);
    let height = height.max(1);
    if !(0..width * height).contains(&pos) {
        return Err(format!("cell index {pos} outside a {width}x{height} board"));
    }
    let row = pos / width;
    let col = pos % width;
    Ok(engine::model::HexCoord::new(col, row))
}

/// Wire traffic status → engine traffic state (`0/1/2` ⇄ smooth/busy/jammed).
const fn traffic_state(status: TrafficStatus) -> TrafficState {
    match status.value() {
        1 => TrafficState::Busy,
        2 => TrafficState::Jammed,
        _ => TrafficState::Smooth,
    }
}

/// model action → wire (solver-trait) action. Both enums declare the same
/// variants in the same order; the mapping is mechanical.
const fn wire_action(action: ModelAction) -> WireAction {
    WireAction {
        agent_id: action.agent_id,
        kind: match action.kind {
            ModelActionKind::Wait => WireActionKind::Wait,
            ModelActionKind::Move(direction) => WireActionKind::Move(wire_direction(direction)),
        },
    }
}

const fn wire_direction(direction: ModelDirection) -> WireDirection {
    match direction {
        ModelDirection::UpperLeft => WireDirection::UpperLeft,
        ModelDirection::UpperRight => WireDirection::UpperRight,
        ModelDirection::Right => WireDirection::Right,
        ModelDirection::LowerRight => WireDirection::LowerRight,
        ModelDirection::LowerLeft => WireDirection::LowerLeft,
        ModelDirection::Left => WireDirection::Left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::model::{AgentActions, Solver as ModelSolver};
    use engine::solver::TeamId;
    use protocol::match_config::{AgentKind as WireAgentKind, AgentState};

    /// A candidate that echoes a deterministic one-step-per-agent move so
    /// the bridge's conversion can be asserted exactly.
    struct EchoCandidate;

    impl ModelSolver for EchoCandidate {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn decide(&self, ctx: &ModelSolverContext<'_>) -> AgentActions {
            // Move every agent Right for every step (well-shaped by len).
            let mut actions = Vec::with_capacity(ctx.agent_ids.len() * ctx.day_steps as usize);
            for _ in 0..ctx.day_steps {
                for &id in &ctx.agent_ids {
                    actions.push(ModelAction::step(id, ModelDirection::Right));
                }
            }
            AgentActions::new(actions)
        }
    }

    /// A candidate that returns a short (malformed) plan; the bridge must
    /// never forward it to the wire.
    struct MalformedCandidate;

    impl ModelSolver for MalformedCandidate {
        fn name(&self) -> &'static str {
            "malformed"
        }

        fn decide(&self, _ctx: &ModelSolverContext<'_>) -> AgentActions {
            AgentActions::new(vec![ModelAction::wait(0)])
        }
    }

    fn wire_setting() -> protocol::match_config::MatchSetting {
        use protocol::match_config::{CellType, Map, Spot};
        const SIDE: usize = 8;
        let mut cells = vec![vec![CellType::TYPE_0; SIDE]; SIDE];
        // A road at index 4 (row 0, col 4) so the traffic overlay has
        // something to actually change. Official table (Issue #226): 1 = road.
        cells[0][4] = CellType::TYPE_1;
        protocol::match_config::MatchSetting {
            starts_at: 1,
            day_seconds: vec![30, 30],
            day_steps: vec![16, 16],
            map: Map { height: 8, width: 8, cells },
            spots: vec![Spot { brand: 0, pos: 12, stocks: 4 }],
            agents: vec![0, 8, 16],
            fuel_limits: 30,
            players: 2,
            busy_threshold: 1,
            jammed_threshold: 2,
        }
    }

    fn engine_setting() -> engine::model::MatchSetting {
        engine::from_wire::protocol_setting_to_engine(
            &wire_setting(),
            TeamId::new("mvp-player").expect("team id"),
        )
        .expect("wire setting converts")
    }

    /// Day-1 context over a 3-agent, 16-step day.
    fn wire_ctx() -> SolverContext {
        SolverContext::new(
            crate::solver_adapter::dummy_match_snapshot(),
            TeamId::new("mvp-player").expect("team id"),
            vec![0, 1, 2],
            1,
            16,
            30_000,
        )
    }

    /// Live wire state for the day: agents moved, one road busy.
    fn live_wire_state() -> WireMatchState {
        WireMatchState {
            ends_at: 0,
            day: 1,
            agents: vec![
                AgentState { kind: WireAgentKind::KIND_0, pos: 3, fuel: 21 },
                AgentState { kind: WireAgentKind::KIND_1, pos: 11, fuel: 30 },
                AgentState { kind: WireAgentKind::KIND_1, pos: 19, fuel: 30 },
            ],
            others: vec![],
            traffics: vec![protocol::match_config::TrafficInfo {
                pos: 4,
                status: TrafficStatus::STATUS_1,
            }],
        }
    }

    /// Bridge over `EchoCandidate` with the setting slot filled in.
    fn bridged(candidate_id: &str, inner: Box<dyn ModelSolver>) -> CandidateBridge {
        let bridge = CandidateBridge::new(candidate_id.to_owned(), inner);
        *bridge.slots.setting.lock().expect("setting slot") = Some(engine_setting());
        bridge
    }

    #[test]
    fn bridge_converts_candidate_plan_into_wire_actions() {
        let ctx = wire_ctx();
        let bridge = bridged("echo:right", Box::new(EchoCandidate));
        *bridge.slots.live_state.lock().expect("live slot") = Some(live_wire_state());

        let decision = bridge.decide(&ctx);
        assert_eq!(decision.actions.len(), 3 * 16, "step-major: one action per agent per step");
        assert!(
            decision.actions.iter().all(|a| a.kind == WireActionKind::Move(WireDirection::Right)),
            "every action converted to a wire Move"
        );
        assert_eq!(
            decision.metadata.get("candidate").and_then(serde_json::Value::as_str),
            Some("echo:right")
        );
        assert_eq!(
            decision.metadata.get("solver").and_then(serde_json::Value::as_str),
            Some("echo")
        );
    }

    #[test]
    fn live_state_overlay_moves_agents_and_traffic() {
        let mut state = ModelMatchState::new(engine_setting()).expect("state");
        apply_live_state(&mut state, &live_wire_state()).expect("overlay applies");

        // Row-major index 3 → (col 3, row 0); fuel came from the wire.
        assert_eq!(state.agents[0].pos, engine::model::HexCoord::new(3, 0));
        assert_eq!(state.agents[0].fuel, 21);
        // Index 11 → (col 3, row 1).
        assert_eq!(state.agents[1].pos, engine::model::HexCoord::new(3, 1));

        // The busy traffic status landed on the road cell at index 4.
        let cell =
            state.setting.map.get(engine::model::HexCoord::new(4, 0)).expect("cell (4,0) exists");
        assert_eq!(cell.terrain, engine::model::Terrain::Road(TrafficState::Busy));
    }

    #[test]
    fn malformed_candidate_plan_degrades_to_wait_all() {
        let ctx = wire_ctx();
        let bridge = bridged("bad:plan", Box::new(MalformedCandidate));

        let decision = bridge.decide(&ctx);
        assert_eq!(decision.actions.len(), ctx.expected_action_count());
        assert!(
            decision.actions.iter().all(|a| a.kind == WireActionKind::Wait),
            "fallback must be wait_all"
        );
        assert_eq!(
            decision.metadata.get("fallback").and_then(serde_json::Value::as_str),
            Some("wait_all")
        );
    }

    #[test]
    fn missing_setting_degrades_to_wait_all() {
        let ctx = wire_ctx();
        let bridge = CandidateBridge::new("echo".to_owned(), Box::new(EchoCandidate));
        let decision = bridge.decide(&ctx);
        assert_eq!(decision.actions.len(), ctx.expected_action_count());
        assert_eq!(
            decision.metadata.get("reason").and_then(serde_json::Value::as_str),
            Some("engine setting not injected yet")
        );
    }

    #[test]
    fn agent_count_mismatch_is_rejected_not_guessed() {
        let mut state = ModelMatchState::new(engine_setting()).expect("state");
        let wire = WireMatchState {
            ends_at: 0,
            day: 0,
            agents: vec![AgentState { kind: WireAgentKind::KIND_0, pos: 0, fuel: 30 }],
            others: vec![],
            traffics: vec![],
        };
        let err = apply_live_state(&mut state, &wire).expect_err("count mismatch must fail");
        assert!(err.contains('3'), "{err}");
    }

    #[test]
    fn cell_index_rejects_out_of_board_positions() {
        assert_eq!(cell_index(11, 8, 8).expect("in bounds"), engine::model::HexCoord::new(3, 1));
        assert!(cell_index(64, 8, 8).is_err());
        assert!(cell_index(-1, 8, 8).is_err());
    }

    #[test]
    fn registry_ids_resolve_through_the_bridge() {
        // One of the Issue #179 shortlist ids — the exact configuration
        // the arena run must be able to play.
        let bridge =
            CandidateBridge::from_registry_id("multiloop:kmeans=4:seed=42:budget=official")
                .expect("shortlist candidate resolves");
        assert_eq!(bridge.candidate_id(), "multiloop:kmeans=4:seed=42:budget=official");
        assert!(CandidateBridge::from_registry_id("multiloop:kmeans=4:seed=42").is_err());
        assert!(CandidateBridge::from_registry_id("loop:budget=element").is_err());
        assert!(CandidateBridge::from_registry_id("no-such-algo").is_err());
    }
    #[test]
    fn campaign_candidate_matches_live_bridge_on_every_day() {
        use bench::adapters::walk::BudgetMode;
        use bench::case::CaseSpec;
        use engine::model::{AgentActions, AgentKind};
        let case: CaseSpec =
            serde_json::from_str(include_str!("../../bench/tests/fixtures/arena-701.json"))
                .expect("701");
        for pattern in ["P,S", "P,S,S,P,S"] {
            let kinds = bench::kinds::resolve_kinds(
                Some(&bench::kinds::parse_kinds(pattern).expect("kinds")),
                case.setting.agents.len(),
            );
            let team = TeamId::new("player-a").expect("team");
            let setting = case.to_engine_setting_with_kinds(&team, Some(&kinds)).expect("setting");
            let mut state = ModelMatchState::new(setting.clone()).expect("state");
            let id = "coverage:budget=official";
            let bridge = CandidateBridge::from_registry_id(id).expect("candidate");
            *bridge.slots.setting.lock().expect("slot") = Some(setting);
            let offline = bench::CandidateRegistry::builtin()
                .build(id)
                .expect("candidate")
                .build_solver()
                .expect("solver");
            let sim = engine::MatchSim::new(case.sim_config().expect("config"));
            while !state.is_finished() {
                let width = i32::from(state.setting.map.size.width);
                let agents: Vec<_> = state
                    .agents
                    .iter()
                    .map(|a| {
                        json!({
                            "kind": i32::from(a.kind != AgentKind::Patrol),
                            "pos": a.pos.row * width + a.pos.col, "fuel": a.fuel.min(i32::MAX as u32)
                        })
                    })
                    .collect();
                let traffics: Vec<_> = state
                    .setting
                    .map
                    .roads()
                    .map(|c| {
                        let status = match c.terrain {
                            engine::model::Terrain::Road(TrafficState::Busy) => 1,
                            engine::model::Terrain::Road(TrafficState::Jammed) => 2,
                            _ => 0,
                        };
                        json!({"pos": c.coord.row * width + c.coord.col, "status": status})
                    })
                    .collect();
                *bridge.slots.live_state.lock().expect("slot") = Some(serde_json::from_value(json!({
                    "endsAt": 0, "day": state.day, "agents": agents, "others": [], "traffics": traffics
                })).expect("wire state"));
                let ctx = ModelSolverContext::from_state(&state);
                let plan = offline.decide(&ctx);
                assert_eq!(BudgetMode::Official.apply(&ctx, plan.clone()), plan);
                let wire = SolverContext::new(
                    crate::solver_adapter::dummy_match_snapshot(),
                    team.clone(),
                    ctx.agent_ids.clone(),
                    ctx.day,
                    ctx.day_steps,
                    60_000,
                );
                let decision = bridge.decide(&wire);
                assert!(decision.metadata.get("fallback").is_none());
                let expected: Vec<_> = plan.actions.iter().copied().map(wire_action).collect();
                assert_eq!(decision.actions, expected, "live parity day {} {pattern}", state.day);
                for chunk in plan.actions.chunks(ctx.agent_ids.len()) {
                    sim.step(&mut state, &AgentActions::new(chunk.to_vec()));
                }
            }
            assert_eq!(state.illegal_action_count, 0);
        }
    }
}
