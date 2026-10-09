//! Regression test: Axis 1-3 and status must be identical across --jobs
//! levels (Issue #335). This is the invariant that makes parallel evaluation
//! safe for exploration.
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation};
use bench::{CandidateRegistry, CaseSpec, Corpus};
use domain::PlayerId;


#[test]
fn anytime_rejects_invalid_budget_ms() {
    let registry = CandidateRegistry::builtin();
    // Invalid budget_ms must be rejected, not silently defaulted.
    assert!(registry.build("anytime:budget_ms=abc:budget=official").and_then(|c| c.build_solver()).is_err());
    assert!(registry.build("anytime:budget_ms=-1:budget=official").and_then(|c| c.build_solver()).is_err());
    assert!(registry.build("anytime:budget_ms=60000:budget=official").and_then(|c| c.build_solver()).is_err());
    // Valid values must work.
    assert!(registry.build("anytime:budget_ms=0:budget=official").and_then(|c| c.build_solver()).is_ok());
    assert!(registry.build("anytime:budget_ms=5000:budget=official").and_then(|c| c.build_solver()).is_ok());
    assert!(registry.build("anytime:budget_ms=59000:budget=official").and_then(|c| c.build_solver()).is_ok());
}

#[test]
fn anytime_rejects_invalid_seed() {
    let registry = CandidateRegistry::builtin();
    assert!(registry.build("anytime:seed=abc:budget=official").and_then(|c| c.build_solver()).is_err());
    assert!(registry.build("anytime:seed=-1:budget=official").and_then(|c| c.build_solver()).is_err());
    assert!(registry.build("anytime:seed=42:budget=official").and_then(|c| c.build_solver()).is_ok());
}
