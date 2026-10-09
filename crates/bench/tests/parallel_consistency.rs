//! Regression test: Axis 1-3 and status must be identical across --jobs
//! levels (Issue #335). This is the invariant that makes parallel evaluation
//! safe for exploration.
use bench::evaluate::{EvalConfig, Evaluator, HarnessDeadline, Isolation};
use bench::{CandidateRegistry, CaseSpec, Corpus};
use domain::PlayerId;

fn run_batch(jobs: usize) -> Vec<(String, String, u32, u32, u32, String)> {
    let cases = Corpus::bundled().expect("bundled corpus");
    let registry = CandidateRegistry::builtin();
    let candidates = vec![
        registry
            .build("coverage:budget=official:park=none")
            .expect("coverage registered"),
        registry
            .build("coverage:budget=official:park=far")
            .expect("park registered"),
    ];
    let config = EvalConfig {
        source_commit_sha: "test-sha".to_owned(),
        team: PlayerId::new("player-a").unwrap(),
        harness_deadline: HarnessDeadline::default(),
        isolation: Isolation::Thread,
        kinds: None,
        oracle_refill: false,
        jobs,
    };
    let evaluator = Evaluator::new(config);
    let records = evaluator.evaluate_all(&candidates, &cases.cases);
    let mut out: Vec<_> = records
        .iter()
        .map(|r| {
            (
                r.candidate_id.clone(),
                r.case_id.clone(),
                r.variety,
                r.daily_variety_sum,
                r.total_balls,
                format!("{:?}", r.status),
            )
        })
        .collect();
    // Sort for deterministic comparison.
    out.sort();
    out
}

#[test]
fn axes_1_3_and_status_match_across_jobs() {
    let sequential = run_batch(1);
    for jobs in [2, 4, 8] {
        let parallel = run_batch(jobs);
        assert_eq!(
            sequential, parallel,
            "Axis 1-3 or status mismatch between jobs=1 and jobs={jobs}"
        );
    }
}

#[test]
fn jobs_zero_is_auto() {
    // jobs=0 must resolve to some positive parallelism and produce the same
    // results as sequential.
    let sequential = run_batch(1);
    let auto = run_batch(0);
    assert_eq!(sequential, auto, "jobs=0 (auto) must match jobs=1 results");
}
