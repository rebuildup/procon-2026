use bench::CandidateRegistry;
#[test]
fn search_is_registered_and_bounded() {
    let r = CandidateRegistry::builtin();
    assert!(r.build("search:trials=8:seed=42:budget=official").is_ok());
    assert!(r.build("search:trials=0:seed=42:budget=official").unwrap().build_solver().is_err());
    assert!(r.build("search:trials=65:seed=42:budget=official").unwrap().build_solver().is_err());
}
