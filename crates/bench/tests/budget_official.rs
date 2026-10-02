//! Issue #255 — `budget=official` on the imported arena board **match 701**.
//!
//! Match 701 is where the element/wire clock gap showed up live: the
//! candidate planned 146 elements on day0 while the server accepted only the
//! terrain-weighted prefix (98 moves), so offline evaluation ran a day the
//! live match never executed. These tests pin the fix on the real board,
//! every day, for both tour shapes that ship:
//!
//! 1. **AC1/AC2** — a `budget=official` plan already fits the official
//!    weighted budget, so `engine::wire_plan::normalize_day_plan` over it
//!    keeps every planned code verbatim (only a trailing wait run may be
//!    shortened to the budget that is left): no rewrite, no appended pad
//!    (`pad == 0`), and every move survives. Byte equality would be the
//!    wrong pin — the engine day is `day_steps` *elements*, so the plan
//!    ends with idle waits; the wire keeps only the budget-paying prefix of
//!    that run, and dropping the rest is not a truncation.
//! 2. the same property does **not** hold for the default `budget=element`
//!    plan on this board — otherwise the fixture would prove nothing.
//!
//! The board lives in `tests/fixtures/arena-701.json`
//! (`procon arena import-case` output, `case_id = fixture:701`). The agent
//! kinds are the composition match 701 actually fielded (`P,S,S,P,S`) so the
//! day plans describe the live run, not a differently-shaped team.

use bench::candidate::CandidateRegistry;
use bench::case::CaseSpec;
use bench::kinds::{parse_kinds, resolve_kinds};
use engine::MatchSim;
use engine::model::{ActionKind, AgentActions, MatchState, SolverContext};
use engine::wire_plan::normalize_day_plan;

/// One day's plan as the wire sees it: `(planned body, normalized body)`,
/// both per-agent code lists.
type DayWire = (Vec<Vec<i32>>, Vec<Vec<i32>>);

/// The live match-701 team composition (5 agents: 2 Patrol, 3 Supply).
const KINDS_701: &str = "P,S,S,P,S";

/// `assign=all` is what match 701 ran live (#253 / #254).
const CANDIDATE_ALL: &str = "multiloop:grid=2x2:walk=astar:assign=all:budget=official";
/// `assign=escort` is the other shipping tour shape (#248).
const CANDIDATE_ESCORT: &str = "multiloop:grid=2x2:walk=astar:assign=escort:budget=official";
/// The pre-#255 clock, for the contrast assertion in the second test.
const CANDIDATE_ELEMENT: &str = "multiloop:grid=2x2:walk=astar:assign=all";

/// The match-701 case, imported through `procon arena import-case`.
///
/// Read as a raw [`CaseSpec`] — the import format `procon-eval
/// --case-files` accepts — not as a bundled `MatchFixture`: the import
/// carries `source`, not the fixture loader's `name`.
fn case_701() -> CaseSpec {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/arena-701.json");
    let text = std::fs::read_to_string(&path).expect("match 701 fixture reads");
    let case: CaseSpec = serde_json::from_str(&text).expect("match 701 fixture is a CaseSpec");
    case.sim_config().expect("match 701 declares a sim config");
    case
}

/// Advance the sim by one day, replaying the plan step-major the way the
/// evaluator does (one block of `agent_count` actions per step). The
/// day's shape is passed by value so the caller's `SolverContext` borrow
/// can end before the mutable borrow of the state begins.
fn step_day(
    state: &mut MatchState,
    sim: &MatchSim,
    plan: &AgentActions,
    agent_ids: &[u8],
    day_steps: u32,
) {
    let agent_count = agent_ids.len();
    for step in 0..day_steps as usize {
        if state.is_finished() {
            break;
        }
        let start = step * agent_count;
        let block = if plan.actions.len() >= start + agent_count {
            plan.actions[start..start + agent_count].to_vec()
        } else {
            AgentActions::wait_all(agent_ids, 1).actions
        };
        sim.step(state, &AgentActions::new(block));
    }
}

/// Re-encode a step-major plan as per-agent wire bodies (run-length waits),
/// the encoding `cli::action_adapter::decision_to_actions` builds before
/// `POST /`.
fn wire_body(plan: &AgentActions, agent_count: usize, day_steps: usize) -> Vec<Vec<i32>> {
    let mut body = Vec::with_capacity(agent_count);
    for index in 0..agent_count {
        let mut codes: Vec<i32> = Vec::new();
        let mut wait_run = 0i32;
        for step in 0..day_steps {
            match plan.actions[step * agent_count + index].kind {
                ActionKind::Wait => wait_run += 1,
                ActionKind::Move(dir) => {
                    if wait_run > 0 {
                        codes.push(-wait_run);
                        wait_run = 0;
                    }
                    codes.push(usize::from(dir as u8).try_into().expect("dir fits in i32"));
                }
            }
        }
        if wait_run > 0 {
            codes.push(-wait_run);
        }
        body.push(codes);
    }
    body
}

/// Drive one candidate over the whole match-701 board and return, per day,
/// the wire body the solver planned plus what `normalize_day_plan` makes of
/// it — the evaluator's own path (`SolverContext::from_state` +
/// `solver.decide`), so what is asserted is what live would submit.
fn normalize_per_day(candidate_id: &str) -> Vec<DayWire> {
    let case = case_701();
    let team = domain::PlayerId::new("player-a").expect("valid team id");
    let kinds = resolve_kinds(Some(&parse_kinds(KINDS_701).expect("kinds parse")), 5);
    let setting = case
        .to_engine_setting_with_kinds(&team, Some(&kinds))
        .expect("case converts with the live kind composition");
    let sim_config = case.sim_config().expect("case declares a sim config");
    let candidate = CandidateRegistry::builtin().build(candidate_id).expect("candidate builds");
    let solver = candidate.build_solver().expect("solver builds");

    let mut state = MatchState::new(setting).expect("validated setting");
    let sim = MatchSim::new(sim_config);
    let mut out = Vec::new();
    for _ in 0..state.num_days() {
        if state.is_finished() {
            break;
        }
        let pair = {
            let ctx = SolverContext::from_state(&state);
            let plan = solver.decide(&ctx);
            let body = wire_body(&plan, ctx.agent_ids.len(), ctx.day_steps as usize);
            let normalized = normalize_day_plan(
                &state.setting.map,
                &state.agents,
                &ctx.agent_ids,
                ctx.day_steps,
                &body,
            )
            .expect("a plan the candidate produced is submittable");
            let agent_ids = ctx.agent_ids.clone();
            let day_steps = ctx.day_steps;
            drop(ctx);
            step_day(&mut state, &sim, &plan, &agent_ids, day_steps);
            (body, normalized)
        };
        out.push(pair);
    }
    out
}

/// Only the budget-consuming moves — waits never distinguish the two clocks.
fn moves(body: &[Vec<i32>]) -> Vec<Vec<i32>> {
    body.iter().map(|codes| codes.iter().copied().filter(|c| *c >= 0).collect()).collect()
}

/// Pin what "no-op at the wire" means for one agent's code list:
/// `kept` must be `planned` **verbatim**, with exactly one legal
/// difference — a trailing wait run may be shortened (the wire keeps only
/// the part the weighted budget can still pay for).
///
/// Anything else is a failure mode:
/// * a code replaced/inserted mid-list → the wire rewrote the plan;
/// * `kept` longer than `planned`, or a new final wait run next to a
///   move → `normalize` **appended a pad**, i.e. the plan under-filled the
///   official budget (the `pad != 0` case AC2 rules out);
/// * a missing move → the tail was truncated (caught more legibly by the
///   moves assertion, but checked here too).
fn assert_verbatim_prefix(planned: &[i32], kept: &[i32], what: &str) {
    let common = planned.iter().zip(kept.iter()).take_while(|(a, b)| a == b).count();
    if common == kept.len() {
        return; // `kept` is an exact prefix of `planned`: nothing added, nothing rewritten.
    }
    assert_eq!(
        common,
        kept.len() - 1,
        "{what}: normalize rewrote the plan mid-list (diverges at code {common}): \
         planned {planned:?} kept {kept:?}"
    );
    let boundary = planned[common];
    let last = kept[common];
    assert!(
        boundary <= -1 && last <= -1 && last.abs() <= boundary.abs(),
        "{what}: normalize must only shorten a trailing wait run — anything else appends \
         a pad or drops a move (pad != 0): planned {planned:?} kept {kept:?}"
    );
}

/// Issue #255 AC1/AC2 on match 701 — `budget=official` produces a plan the
/// official rule already accepts verbatim: `normalize_day_plan` returns the
/// body unchanged (no move dropped, `pad == 0`) on **every day**, for both
/// `assign=all` and `assign=escort`.
#[test]
fn budget_official_is_a_noop_at_the_wire_on_match_701_every_day() {
    for candidate_id in [CANDIDATE_ALL, CANDIDATE_ESCORT] {
        let days = normalize_per_day(candidate_id);
        assert!(!days.is_empty(), "{candidate_id}: the 701 board must plan at least one day");
        for (day, (body, normalized)) in days.iter().enumerate() {
            for (agent, (planned, kept)) in body.iter().zip(normalized).enumerate() {
                assert_verbatim_prefix(
                    planned,
                    kept,
                    &format!("{candidate_id}: day {day} agent {agent}"),
                );
            }
            assert_eq!(
                moves(normalized),
                moves(body),
                "{candidate_id}: day {day} lost moves at the wire — the plan is not \
                 within the official weighted budget"
            );
        }
    }
}

/// The contrast half: on this board the default `budget=element` plan is
/// **not** accepted verbatim — the wire drops its tail. Without this the
/// first test would pass even if `budget=official` silently did nothing.
#[test]
fn budget_element_is_truncated_on_match_701() {
    let days = normalize_per_day(CANDIDATE_ELEMENT);
    assert!(!days.is_empty(), "the 701 board must plan at least one day");
    let truncated_days =
        days.iter().filter(|(body, normalized)| moves(normalized) != moves(body)).count();
    assert!(
        truncated_days > 0,
        "budget=element plans fit the weighted budget on every day of match 701 — this \
         fixture no longer exercises the truncation Issue #255 fixes"
    );
}

/// The day contract itself: whatever `budget=official` emits, the plan is
/// well-shaped for `MatchSim` — one action per controlled agent per step,
/// in `ctx.agent_ids` order.
#[test]
fn budget_official_plans_are_well_shaped_for_the_engine() {
    let case = case_701();
    let team = domain::PlayerId::new("player-a").expect("valid team id");
    let kinds = resolve_kinds(Some(&parse_kinds(KINDS_701).expect("kinds parse")), 5);
    let setting = case
        .to_engine_setting_with_kinds(&team, Some(&kinds))
        .expect("case converts with the live kind composition");
    let candidate = CandidateRegistry::builtin().build(CANDIDATE_ALL).expect("candidate builds");
    let solver = candidate.build_solver().expect("solver builds");
    let state = MatchState::new(setting).expect("validated setting");
    let ctx = SolverContext::from_state(&state);
    let plan = solver.decide(&ctx);
    assert!(
        plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
        "budget=official must hand the engine a well-shaped day ({} actions, expected {})",
        plan.actions.len(),
        ctx.agent_ids.len() * ctx.day_steps as usize
    );
}
