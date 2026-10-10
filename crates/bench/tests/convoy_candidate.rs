//! Convoy candidate (Issue #432): parameter validation, the `bail` early
//! exit, `cluster` label-space partitioning, and default-behaviour identity
//! on the realistic 701 board.
use bench::CandidateRegistry;
use bench::adapters::convoy_solver::ConvoyCandidate;
use bench::adapters::fleet_solver::FleetCandidate;
use bench::adapters::walk::BudgetMode;
use bench::case::CaseSpec;
use bench::kinds::{parse_kinds, resolve_kinds};
use domain::PlayerId;
use engine::model::{AgentActions, MatchState, Solver, SolverContext};

fn solver(id: &str) -> Box<dyn Solver> {
    CandidateRegistry::builtin().build(id).and_then(|c| c.build_solver()).expect("id builds")
}

fn state_701(pattern: &str) -> (CaseSpec, MatchState) {
    let case: CaseSpec = serde_json::from_str(include_str!("fixtures/arena-701.json")).unwrap();
    let kinds = resolve_kinds(Some(&parse_kinds(pattern).unwrap()), case.setting.agents.len());
    let setting = case
        .to_engine_setting_with_kinds(&PlayerId::new("player-a").unwrap(), Some(&kinds))
        .unwrap();
    let state = MatchState::new(setting).unwrap();
    (case, state)
}

/// FNV-1a over the plan's actions in wire order: a compact stand-in for
/// byte-identity of a day plan.
fn plan_hash(plan: &AgentActions) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for action in &plan.actions {
        hash ^= u64::from(action.agent_id);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        let (tag, dir) = match action.kind {
            engine::model::ActionKind::Wait => (0u8, None),
            engine::model::ActionKind::Move(d) => {
                (1u8, engine::model::Direction::ALL.iter().position(|&x| x == d).map(u8::try_from))
            }
        };
        hash ^= u64::from(tag);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        if let Some(Ok(dir)) = dir {
            hash ^= u64::from(dir);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

#[test]
fn convoy_requires_official_budget_and_rejects_bad_params() {
    let build =
        |id: &str| CandidateRegistry::builtin().build(id).and_then(|c| c.build_solver()).is_ok();
    assert!(build("convoy:budget=official"));
    assert!(build("convoy:budget=official:bail=on"));
    assert!(build("convoy:budget=official:bail=off"));
    assert!(!build("convoy"));
    assert!(!build("convoy:budget=element"));
    assert!(!build("convoy:budget=official:bail=maybe"));
}

/// A pinned deterministic day-0 answer on the 701 board: `cluster=off` +
/// `bail=off` must reproduce the pre-#432 behaviour byte for byte.
#[test]
fn default_params_keep_the_pre_432_day_plan() {
    let (_, state) = state_701("S,P,P,P,P");
    let ctx = SolverContext::from_state(&state);
    let id = "convoy:budget=official:seed=42:depth=1:width=2:forecast=persist";
    let plan = solver(id).decide(&ctx);
    assert_eq!(plan_hash(&plan), 5_676_638_542_188_020_946, "{id} day-0 plan changed");
}

/// A deterministic candidate answers the same plan twice.
#[test]
fn convoy_is_reproducible() {
    let (_, state) = state_701("S,P,P,P,P");
    let ctx = SolverContext::from_state(&state);
    let id = "convoy:budget=official:seed=42:depth=1:width=2:forecast=persist";
    assert_eq!(solver(id).decide(&ctx), solver(id).decide(&ctx));
}

fn bare_candidate(bail: bool, labels: usize) -> ConvoyCandidate {
    ConvoyCandidate {
        fleet: FleetCandidate { iters: 300, budget_ms: 0, seed: 42, chains: 1, adapt: false },
        ahead: FleetCandidate { iters: 200, budget_ms: 0, seed: 42, chains: 1, adapt: true },
        budget_ms: 0,
        scenarios: vec![false],
        depth: 1,
        width: 1,
        labels,
        bail,
    }
}

/// With a label cap no search can survive, every day plan is `fleet`'s
/// alone. `bail=on` must then answer that plan without playing the beam:
/// same answer the default path eventually returns, in a fraction of the
/// time.
#[test]
fn bail_on_returns_the_fleet_plan_when_the_label_search_fails() {
    let (_, state) = state_701("S,P,P,P,P");
    let ctx = SolverContext::from_state(&state);
    let fleet = FleetCandidate { iters: 300, budget_ms: 0, seed: 42, chains: 1, adapt: false };
    let expected = fleet.decide(&ctx);
    let started = std::time::Instant::now();
    let plan = bare_candidate(true, 1).decide(&ctx);
    let elapsed = started.elapsed();
    assert_eq!(plan, expected, "bail=on answers the embedded fleet's plan");
    // The beam this skips would replay every remaining day twice over; the
    // bailed search must stay far below that on one fleet search alone.
    assert!(elapsed < std::time::Duration::from_secs(10), "bail=on took {elapsed:?}");
}

#[test]
fn bail_off_still_answers_when_the_label_search_fails() {
    let (_, state) = state_701("S,P,P,P,P");
    let ctx = SolverContext::from_state(&state);
    let plan = bare_candidate(false, 1).decide(&ctx);
    assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
    assert_eq!(BudgetMode::Official.apply(&ctx, plan.clone()), plan, "normalization fixed point");
}
