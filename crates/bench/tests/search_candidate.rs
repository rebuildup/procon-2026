use bench::CandidateRegistry;
#[test]
fn search_is_registered_and_bounded() {
    let r = CandidateRegistry::builtin();
    assert!(r.build("search:trials=8:seed=42:budget=official").is_ok());
    assert!(r.build("search:trials=0:seed=42:budget=official").unwrap().build_solver().is_err());
    // The 64-trial cap was Issue #315's exploration bound; the answer-time
    // gap (Issue #311) needs hundreds+ trials or a wall clock, so the cap
    // moved to 1048576 and 65 is now a legal deterministic run.
    assert!(r.build("search:trials=65:seed=42:budget=official").is_ok());
    assert!(
        r.build("search:trials=1048577:seed=42:budget=official").unwrap().build_solver().is_err()
    );
}

#[test]
fn search_accepts_a_wall_clock_budget_and_rejects_an_overlong_one() {
    let r = CandidateRegistry::builtin();
    assert!(r.build("search:trials=8:seed=42:time_budget_ms=20000:budget=official").is_ok());
    assert!(
        r.build("search:trials=8:seed=42:time_budget_ms=59001:budget=official")
            .unwrap()
            .build_solver()
            .is_err()
    );
}
