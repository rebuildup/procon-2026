//! Issue #235 — the agent-kind axis: visibility, the `--kinds` knob's
//! byte-identical default, and the `assign` parameter's kind-aware routing.
//!
//! Three acceptance criteria live here:
//!
//! 1. **Kind visibility** — a candidate reaches `MatchState.agents[].kind`
//!    through `SolverContext`, observed end to end (probe candidate →
//!    `Evaluator` → `RunRecord`).
//! 2. **Byte-identical default** — the omitted knob fields exactly the
//!    pre-#235 alternation, both as a board and as a record.
//! 3. **`assign=patrol`** — a Supply agent receives no tour target and waits,
//!    while `assign=agents` (the default) is unchanged.

use std::sync::{Arc, Mutex};

use bench::adapters::walk::AssignMode;
use bench::candidate::{
    CandidateError, CandidateFactory, CandidateRegistry, CandidateSpec, ParamSpec,
};
use bench::case::CaseSpec;
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation, RunStatus};
use bench::kinds::{parse_kinds, resolve_kinds};
use domain::PlayerId;
use engine::model::{ActionKind, AgentActions, AgentKind, Solver, SolverContext};

fn team() -> PlayerId {
    PlayerId::new("player-a").unwrap()
}

fn config(kinds: Option<Vec<AgentKind>>) -> EvalConfig {
    EvalConfig {
        source_commit_sha: "test-commit".to_owned(),
        team: team(),
        harness_deadline: HarnessDeadline::default(),
        isolation: Isolation::Thread,
        kinds,
        oracle_refill: false,
    }
}

/// The live board a candidate would be handed for `case` under `kinds`.
fn state_for(case: &CaseSpec, kinds: Option<&[AgentKind]>) -> engine::model::MatchState {
    let expanded = resolve_kinds(kinds, case.setting.agents.len());
    let setting =
        case.to_engine_setting_with_kinds(&team(), Some(&expanded)).expect("board converts");
    engine::model::MatchState::new(setting).expect("board validates")
}

// ---------------------------------------------------------------------------
// Deliverable 2 — a candidate can read each agent's kind
// ---------------------------------------------------------------------------

/// Records `(id, kind)` for every agent it is asked to plan for, then waits.
struct KindProbe {
    seen: Arc<Mutex<Vec<(u8, AgentKind)>>>,
}

impl Solver for KindProbe {
    fn name(&self) -> &'static str {
        "kind-probe"
    }

    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let mut seen = self.seen.lock().expect("probe lock");
        seen.clear();
        for id in &ctx.agent_ids {
            // Exactly the read Issue #235 asks to have pinned:
            // SolverContext -> MatchState.agents[].kind
            let agent = ctx
                .state
                .agents
                .iter()
                .find(|a| a.id == *id)
                .expect("every controlled agent exists in the state");
            seen.push((agent.id, agent.kind));
        }
        AgentActions::wait_all(&ctx.agent_ids, ctx.day_steps)
    }
}

struct KindProbeFactory {
    seen: Arc<Mutex<Vec<(u8, AgentKind)>>>,
}

impl CandidateFactory for KindProbeFactory {
    fn algorithm(&self) -> &'static str {
        "kind-probe"
    }

    fn params(&self) -> &'static [ParamSpec] {
        &[]
    }

    fn describe(&self) -> &'static str {
        "test probe: reads agent kinds from SolverContext and waits"
    }

    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        if let Some((key, _)) = spec.params.first() {
            return Err(CandidateError::UnknownParameter {
                algorithm: spec.algorithm.clone(),
                key: key.clone(),
            });
        }
        Ok(Box::new(KindProbe { seen: self.seen.clone() }))
    }
}

/// AC: a candidate reads each agent's kind from `SolverContext`, and what it
/// read is exactly what the `RunRecord` reports.
///
/// `fixture:road-heavy` fields 5 agents, so the `P,S,P,S,P` pattern resolves
/// to the Issue's own example composition (3 Patrol / 2 Supply).
#[test]
fn a_candidate_reads_agent_kinds_through_the_solver_context() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut registry = CandidateRegistry::builtin();
    registry.register(Arc::new(KindProbeFactory { seen: seen.clone() }));

    let candidate = registry.build("kind-probe").expect("probe registers");
    let case = CaseSpec::from_bundled_fixture("road-heavy").expect("fixture loads");
    assert_eq!(case.setting.agents.len(), 5, "the pinned fixture must field 5 agents");

    let pattern = parse_kinds("P,S,P,S,P").expect("pattern parses");
    let record = Evaluator::new(config(Some(pattern))).evaluate_one(&candidate, &case);
    assert_eq!(record.status, RunStatus::Success, "{}", record.status_detail.unwrap_or_default());

    let observed = seen.lock().expect("probe lock").clone();
    let expected: Vec<(u8, AgentKind)> = vec![
        (0, AgentKind::Patrol),
        (1, AgentKind::Supply),
        (2, AgentKind::Patrol),
        (3, AgentKind::Supply),
        (4, AgentKind::Patrol),
    ];
    assert_eq!(observed, expected, "the probe must see the composition it was configured with");

    // The record is the durable half of the AC: what the candidate saw is
    // what the evidence says.
    assert_eq!(
        record.agent_kinds,
        Some(expected.iter().map(|(_, k)| *k).collect::<Vec<_>>()),
        "RunRecord.agent_kinds must carry the composition the run fielded"
    );
}

// ---------------------------------------------------------------------------
// AC: the default is byte-identical to the pre-#235 world
// ---------------------------------------------------------------------------

/// The omitted knob and `P,S` produce the same board the un-parameterised
/// converter always produced — for every team size in the official 3..=8.
#[test]
fn the_default_kinds_match_the_historical_board() {
    for name in ["small-8x8", "medium-16x16", "series-few", "road-heavy", "large-32x32"] {
        let case = CaseSpec::from_bundled_fixture(name).expect("fixture loads");
        let n = case.setting.agents.len();
        let historical = case.to_engine_setting(&team()).expect("converts");
        let with_default = case
            .to_engine_setting_with_kinds(&team(), Some(&resolve_kinds(None, n)))
            .expect("converts");
        assert_eq!(historical, with_default, "{name}: the default board must not move");

        for (index, agent) in historical.agents.iter().enumerate() {
            let want = if index % 2 == 0 { AgentKind::Patrol } else { AgentKind::Supply };
            assert_eq!(agent.kind, want, "{name} slot {index}");
        }
    }
}

/// An unscored knob (`kinds: None`) records the same composition an explicit
/// `P,S` does — the record proves the default rather than assuming it.
#[test]
fn the_default_record_carries_the_alternation() {
    let registry = CandidateRegistry::builtin();
    let candidate = registry.build("wait").expect("wait is builtin");
    let case = CaseSpec::from_bundled_fixture("road-heavy").expect("fixture loads");

    let implicit = Evaluator::new(config(None)).evaluate_one(&candidate, &case);
    let explicit = Evaluator::new(config(Some(parse_kinds("P,S").expect("pattern"))))
        .evaluate_one(&candidate, &case);
    assert_eq!(implicit.agent_kinds, explicit.agent_kinds, "omitted flag == explicit P,S");
    assert_eq!(
        implicit.agent_kinds,
        Some(vec![
            AgentKind::Patrol,
            AgentKind::Supply,
            AgentKind::Patrol,
            AgentKind::Supply,
            AgentKind::Patrol
        ])
    );
}

// ---------------------------------------------------------------------------
// Deliverable 3 — the `assign` axis
// ---------------------------------------------------------------------------

/// `assign=patrol` is the whole kind-aware assignment switch this Issue adds:
/// Supply agents get no tour target and wait all day.
#[test]
fn assign_patrol_makes_supply_agents_wait() {
    let registry = CandidateRegistry::builtin();
    let case = CaseSpec::from_bundled_fixture("road-heavy").expect("fixture loads");
    let kinds = resolve_kinds(Some(&parse_kinds("P,S,P,S,P").expect("pattern")), 5);
    let state = state_for(&case, Some(kinds.as_slice()));
    let ctx = SolverContext::from_state(&state);
    let supply: Vec<u8> =
        state.agents.iter().filter(|a| a.kind == AgentKind::Supply).map(|a| a.id).collect();
    let patrol: Vec<u8> =
        state.agents.iter().filter(|a| a.kind == AgentKind::Patrol).map(|a| a.id).collect();
    assert_eq!(supply, vec![1, 3], "3P2S on this fixture");
    assert_eq!(patrol, vec![0, 2, 4]);

    let plan_for = |id: &str| {
        let candidate = registry.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        candidate.build_solver().expect("solver builds").decide(&ctx)
    };

    let patrol_only = plan_for("loop:iterations=32:initial=greedy:assign=patrol");
    assert!(patrol_only.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
    for id in &supply {
        let mine: Vec<ActionKind> =
            patrol_only.actions.iter().filter(|a| a.agent_id == *id).map(|a| a.kind).collect();
        assert!(!mine.is_empty(), "agent {id} has actions");
        assert!(
            mine.iter().all(|k| matches!(k, ActionKind::Wait)),
            "assign=patrol must make Supply agent {id} wait, got {mine:?}"
        );
    }
    // The patrol agents are untouched — at least one of them is walking.
    assert!(
        patrol.iter().any(|id| patrol_only
            .actions
            .iter()
            .filter(|a| a.agent_id == *id)
            .any(|a| matches!(a.kind, ActionKind::Move(_)))),
        "assign=patrol must not stop the patrol agents"
    );

    // And the mode really is a switch: the all-agents assignment differs.
    let all_agents = plan_for("loop:iterations=32:initial=greedy");
    assert_ne!(patrol_only, all_agents, "assign=patrol must change the plan");
    let some_supply_moves = supply.iter().any(|id| {
        all_agents
            .actions
            .iter()
            .filter(|a| a.agent_id == *id)
            .any(|a| matches!(a.kind, ActionKind::Move(_)))
    });
    assert!(some_supply_moves, "the default must still hand Supply a tour");
}

/// AC: the default is byte-identical. `assign=agents` and the omitted
/// parameter are the same plan, action for action — for both tour candidates.
#[test]
fn assign_agents_is_byte_identical_to_the_omitted_parameter() {
    let registry = CandidateRegistry::builtin();
    let case = CaseSpec::from_bundled_fixture("road-heavy").expect("fixture loads");
    let state = state_for(&case, None);
    let ctx = SolverContext::from_state(&state);

    for bare in ["loop:iterations=32:initial=greedy", "multiloop:kmeans=4:seed=42"] {
        let a = registry.build(bare).expect("bare id builds").build_solver().expect("solver");
        let explicit_id = format!("{bare}:assign=agents");
        let b = registry
            .build(explicit_id.as_str())
            .expect("explicit default builds")
            .build_solver()
            .expect("solver");
        assert_eq!(
            a.decide(&ctx),
            b.decide(&ctx),
            "{bare}: assign=agents must be exactly the pre-#235 plan"
        );
    }
}

/// The `assign` value is schema-gated: an unknown value never builds, on
/// either candidate, so a typo can never silently fall back to `agents`.
#[test]
fn assign_rejects_unknown_values_on_both_tour_candidates() {
    let registry = CandidateRegistry::builtin();
    for id in [
        "loop:iterations=32:initial=greedy:assign=escort",
        "loop:assign=PATROL",
        "multiloop:kmeans=4:seed=42:assign=tour",
        "multiloop:grid=2x2:assign=none",
    ] {
        match registry.build(id) {
            Err(CandidateError::InvalidParameter { key, .. }) => assert_eq!(key, "assign"),
            other => panic!("{id} must be rejected as an invalid assign value, got {other:?}"),
        }
    }
}

/// The mode enum itself is a two-valued switch with `agents` as the default.
#[test]
fn assign_mode_defaults_to_agents() {
    assert_eq!(AssignMode::default(), AssignMode::Agents);
    assert_ne!(AssignMode::default(), AssignMode::Patrol);
}
