//! Telemetry wiring on the fleet search (Issue #429): counters move during
//! a real iters-mode search, stay inert while disabled, and a manual cutoff
//! request breaks the loop early without breaking the answer.
//!
//! One sequential test: the counters are a process-global (`SEARCH`), and
//! the point is the order disable → enable → cut, which parallel tests
//! would race on. The day boundary belongs to `decide` itself (nested
//! convoy/fleet calls keep the outer one), so the tests never bracket a
//! `decide` with `day_begin` / `day_end`.

use std::time::Instant;

use bench::candidate::CandidateRegistry;
use bench::telemetry::{CutoffError, Telemetry, TelemetryMeta};
use domain::PlayerId;
use engine::model::{MatchState, Solver, SolverContext};

fn solver(id: &str) -> Box<dyn Solver> {
    CandidateRegistry::builtin().build(id).and_then(|c| c.build_solver()).expect("id builds")
}

fn state_701() -> MatchState {
    let case: bench::case::CaseSpec =
        serde_json::from_str(include_str!("fixtures/arena-701.json")).unwrap();
    let kinds = bench::kinds::resolve_kinds(
        Some(&bench::kinds::parse_kinds("S,P,P,P,P").unwrap()),
        case.setting.agents.len(),
    );
    let setting = case
        .to_engine_setting_with_kinds(&PlayerId::new("player-a").unwrap(), Some(&kinds))
        .unwrap();
    MatchState::new(setting).unwrap()
}

#[test]
fn telemetry_lifecycle_over_real_searches() {
    let tel: &'static Telemetry = &bench::telemetry::SEARCH;

    // 1. Disabled (the default): a real search leaves every counter alone.
    {
        let state = state_701();
        let ctx = SolverContext::from_state(&state);
        solver("fleet:budget=official:iters=400:seed=42").decide(&ctx);
        let snap = tel.snapshot();
        assert!(!snap.enabled);
        assert_eq!(snap.iters, 0, "no iteration samples while off");
        assert_eq!(snap.plans, 0, "no plan counts while off");
    }

    // 2. Enabled: an iters-mode search samples iterations, improvements
    //    and the plan built. `decide` itself opens and closes the day.
    tel.enable(TelemetryMeta {
        run_id: "test-run".into(),
        candidate: "fleet:budget=official:iters=400".into(),
    });
    {
        let state = state_701();
        let ctx = SolverContext::from_state(&state);
        solver("fleet:budget=official:iters=400:seed=42:chains=2").decide(&ctx);
        let snap = tel.snapshot();
        assert!(snap.iters > 0, "iteration samples recorded: {}", snap.iters);
        assert!(snap.best > 0, "best model score recorded: {}", snap.best);
        assert!(snap.improvements > 0, "improvements recorded: {}", snap.improvements);
        assert!(snap.plans >= 1, "plan count recorded: {}", snap.plans);
        assert!(!snap.curve.is_empty(), "improvement curve has points");
        assert!(snap.curve.iter().all(|(_, s)| *s <= snap.best), "curve never exceeds best");
        assert_eq!(snap.iters_target, 400, "decide publishes the iteration target");
        assert_eq!(snap.run_id, "test-run");
        assert!(!snap.cutoff);
        assert_eq!(snap.phase, "done");
    }

    // 3. The guard rails, on a private instance (no global pollution): a
    //    stale day is refused, telemetry-off is refused, idle is refused.
    {
        let tel = Telemetry::new();
        assert_eq!(tel.request_cutoff(Some(0)), Err(CutoffError::Disabled));
        tel.enable(TelemetryMeta { run_id: "r".into(), candidate: "c".into() });
        assert_eq!(tel.request_cutoff(None), Err(CutoffError::Idle));
        tel.day_begin(2, Some(10_000), 0);
        assert_eq!(tel.request_cutoff(Some(1)), Err(CutoffError::DayMismatch { current: 2 }));
        assert!(!tel.cutoff_pending());
        assert!(tel.request_cutoff(Some(2)).is_ok());
        assert!(tel.cutoff_pending());
        tel.day_end();
        assert!(!tel.cutoff_pending(), "day_end clears the request");
    }

    // 4. Wall-clock mode honours the cutoff: a 20 s window ends as soon as
    //    a watcher thread cuts the day in flight, and the answer stays a
    //    legal day.
    {
        let state = state_701();
        let ctx = SolverContext::from_state(&state);
        let watcher = std::thread::spawn(|| {
            let tel: &'static Telemetry = &bench::telemetry::SEARCH;
            for _ in 0..400 {
                if tel.day() == 0 && tel.snapshot().phase == "planning" {
                    return tel.request_cutoff(None).is_ok();
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        });
        let began = Instant::now();
        let plan = solver("fleet:budget=official:budget_ms=20000:seed=42:adapt=off").decide(&ctx);
        let took = began.elapsed();
        assert!(watcher.join().expect("watcher"), "the request was accepted");
        assert!(
            plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
            "the cut-short plan is still a legal day"
        );
        assert!(
            took < std::time::Duration::from_secs(5),
            "the cutoff must end the search long before the 20 s window: {took:?}"
        );
        assert!(!tel.cutoff_pending(), "the day's end consumed the request");
    }

    // 5. Without a pending request the offline answer is untouched:
    //    identical searches, identical plans.
    {
        let state = state_701();
        let ctx = SolverContext::from_state(&state);
        let a = solver("fleet:budget=official:iters=300:seed=42").decide(&ctx);
        let b = solver("fleet:budget=official:iters=300:seed=42").decide(&ctx);
        assert_eq!(a, b, "no cutoff pending: identical searches, identical plans");
        assert!(!tel.cutoff_pending());
    }
}
