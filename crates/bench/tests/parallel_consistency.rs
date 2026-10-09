//! Anytime candidate registry validation (Issue #340).
//!
//! Canonical id `anytime:budget=official` must build, and invalid
//! `budget_ms` / `seed` values must be rejected explicitly instead of
//! being silently defaulted.
use bench::CandidateRegistry;

#[test]
fn anytime_budget_official_constructs() {
    let registry = CandidateRegistry::builtin();
    assert!(registry.build("anytime:budget=official").and_then(|c| c.build_solver()).is_ok());
}

#[test]
fn anytime_rejects_invalid_budget_ms() {
    let registry = CandidateRegistry::builtin();
    // Invalid budget_ms must be rejected, not silently defaulted.
    assert!(
        registry
            .build("anytime:budget_ms=abc:budget=official")
            .and_then(|c| c.build_solver())
            .is_err()
    );
    assert!(
        registry
            .build("anytime:budget_ms=-1:budget=official")
            .and_then(|c| c.build_solver())
            .is_err()
    );
    assert!(
        registry
            .build("anytime:budget_ms=60000:budget=official")
            .and_then(|c| c.build_solver())
            .is_err()
    );
    // Valid values must work.
    assert!(
        registry
            .build("anytime:budget_ms=0:budget=official")
            .and_then(|c| c.build_solver())
            .is_ok()
    );
    assert!(
        registry
            .build("anytime:budget_ms=5000:budget=official")
            .and_then(|c| c.build_solver())
            .is_ok()
    );
    assert!(
        registry
            .build("anytime:budget_ms=59000:budget=official")
            .and_then(|c| c.build_solver())
            .is_ok()
    );
}

#[test]
fn anytime_rejects_invalid_seed() {
    let registry = CandidateRegistry::builtin();
    assert!(
        registry.build("anytime:seed=abc:budget=official").and_then(|c| c.build_solver()).is_err()
    );
    assert!(
        registry.build("anytime:seed=-1:budget=official").and_then(|c| c.build_solver()).is_err()
    );
    assert!(
        registry.build("anytime:seed=42:budget=official").and_then(|c| c.build_solver()).is_ok()
    );
}
