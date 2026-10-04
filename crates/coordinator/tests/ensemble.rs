//! Integration tests for the coordinator crate.
//!
//! These exercise the full public surface (`EnsembleSelector` +
//! `EnsembleMode` + `StubScoreReport`) end-to-end, including:
//!
//! - 3-solver scenarios (greedy / random / loop)
//! - best-of / majority-vote / weighted-voting selection
//! - Send + Sync round-trip via `Arc<EnsembleSelector>`
//! - Parallel dispatch through `std::thread::spawn`

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use coordinator::types::Solver;
use coordinator::{
    ActionSequence, EnsembleMode, EnsembleSelector, MatchInput, RegisteredSolver, SolverOutput,
    StubScoreReport,
};

struct GreedySolver;
impl Solver for GreedySolver {
    fn id(&self) -> &'static str {
        "greedy"
    }
    fn solve(&self, _: &MatchInput) -> SolverOutput {
        SolverOutput {
            score: StubScoreReport::new("greedy", 7, 12, 50, 8_000, 2),
            plan: ActionSequence::from_actions(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
        }
    }
}

struct RandomSolver;
impl Solver for RandomSolver {
    fn id(&self) -> &'static str {
        "random"
    }
    fn solve(&self, _: &MatchInput) -> SolverOutput {
        SolverOutput {
            score: StubScoreReport::new("random", 1, 1, 3, 500, 1),
            plan: ActionSequence::from_actions(vec![42, 42, 42]),
        }
    }
}

struct LoopSolver(Arc<AtomicU32>);
impl Solver for LoopSolver {
    fn id(&self) -> &'static str {
        "loop"
    }
    fn solve(&self, _: &MatchInput) -> SolverOutput {
        let i = self.0.fetch_add(1, Ordering::SeqCst);
        SolverOutput {
            score: StubScoreReport::new("loop", 3, 5, 12, 2_500, 1),
            plan: ActionSequence::from_actions(vec![i, i.wrapping_add(1), i.wrapping_add(2)]),
        }
    }
}

fn three_solvers() -> Vec<RegisteredSolver> {
    vec![
        RegisteredSolver::new(Arc::new(GreedySolver)),
        RegisteredSolver::new(Arc::new(RandomSolver)),
        RegisteredSolver::new(Arc::new(LoopSolver(Arc::new(AtomicU32::new(100))))),
    ]
}

#[test]
fn best_of_picks_greedy_over_random_and_loop() {
    let sel = EnsembleSelector::best_of(three_solvers());
    let res = sel.run(&MatchInput::new("match-best-of", 0));

    assert_eq!(res.mode, EnsembleMode::BestOf);
    assert_eq!(res.winner.score.solver_id, "greedy");
    assert_eq!(res.winner.score.distinct_series, 7);
    assert_eq!(res.all_outputs.len(), 3);
    assert!(res.vote_tally.is_none());
}

#[test]
fn majority_vote_picks_consensus_plan() {
    // Two solvers return [9,9,9] and one returns [1,2,3]; consensus is [9,9,9].
    struct PlanA;
    struct PlanB;
    struct PlanC;
    impl Solver for PlanA {
        fn id(&self) -> &'static str {
            "plan-a"
        }
        fn solve(&self, _: &MatchInput) -> SolverOutput {
            SolverOutput {
                score: StubScoreReport::new("plan-a", 4, 4, 4, 100, 0),
                plan: ActionSequence::from_actions(vec![9, 9, 9]),
            }
        }
    }
    impl Solver for PlanB {
        fn id(&self) -> &'static str {
            "plan-b"
        }
        fn solve(&self, _: &MatchInput) -> SolverOutput {
            SolverOutput {
                score: StubScoreReport::new("plan-b", 4, 4, 4, 100, 0),
                plan: ActionSequence::from_actions(vec![9, 9, 9]),
            }
        }
    }
    impl Solver for PlanC {
        fn id(&self) -> &'static str {
            "plan-c"
        }
        fn solve(&self, _: &MatchInput) -> SolverOutput {
            SolverOutput {
                score: StubScoreReport::new("plan-c", 4, 4, 4, 100, 0),
                plan: ActionSequence::from_actions(vec![1, 2, 3]),
            }
        }
    }

    let sel = EnsembleSelector::new(
        vec![
            RegisteredSolver::new(Arc::new(PlanA)),
            RegisteredSolver::new(Arc::new(PlanB)),
            RegisteredSolver::new(Arc::new(PlanC)),
        ],
        EnsembleMode::MajorityVote,
    );
    let res = sel.run(&MatchInput::new("match-vote", 0));

    assert_eq!(res.winner.plan.actions, vec![9, 9, 9]);
    let tally = res.vote_tally.expect("vote tally present");
    assert_eq!(tally.votes, 2);
    assert_eq!(tally.total_votes, 3);
    assert!(tally.has_majority());
}

#[test]
fn weighted_voting_breaks_majority_tie_by_score() {
    // Both plans appear once; the higher cumulative-distinct-series wins.
    struct HiPlan;
    struct LoPlan;
    impl Solver for HiPlan {
        fn id(&self) -> &'static str {
            "hi-plan"
        }
        fn solve(&self, _: &MatchInput) -> SolverOutput {
            SolverOutput {
                score: StubScoreReport::new("hi-plan", 5, 20, 5, 100, 0),
                plan: ActionSequence::from_actions(vec![1]),
            }
        }
    }
    impl Solver for LoPlan {
        fn id(&self) -> &'static str {
            "lo-plan"
        }
        fn solve(&self, _: &MatchInput) -> SolverOutput {
            SolverOutput {
                score: StubScoreReport::new("lo-plan", 5, 1, 5, 100, 0),
                plan: ActionSequence::from_actions(vec![2]),
            }
        }
    }

    let sel = EnsembleSelector::new(
        vec![RegisteredSolver::new(Arc::new(HiPlan)), RegisteredSolver::new(Arc::new(LoPlan))],
        EnsembleMode::WeightedByScore,
    );
    let res = sel.run(&MatchInput::new("match-weighted", 0));
    assert_eq!(res.winner.plan.actions, vec![1]);
}

#[test]
fn parallel_dispatch_returns_results_in_registration_order() {
    let sel = EnsembleSelector::best_of(three_solvers());
    let res = sel.run_parallel(&MatchInput::new("match-parallel", 1), None);
    // Order preserved across threads.
    assert_eq!(res.all_outputs[0].score.solver_id, "greedy");
    assert_eq!(res.all_outputs[1].score.solver_id, "random");
    assert_eq!(res.all_outputs[2].score.solver_id, "loop");
    assert_eq!(res.winner.score.solver_id, "greedy");
}

#[test]
fn ensemble_selector_is_send_and_sync_via_arc() {
    let sel = Arc::new(EnsembleSelector::best_of(three_solvers()));
    let s2 = Arc::clone(&sel);
    let handle = std::thread::spawn(move || {
        // Just dispatch a run on the shared selector from another thread
        // to exercise Send + Sync round-trip.
        let res = s2.run(&MatchInput::new("match-thread", 0));
        (res.winner.score.solver_id, res.all_outputs.len())
    });
    let (winner_id, count) = handle.join().expect("thread join ok");
    assert_eq!(winner_id, "greedy");
    assert_eq!(count, 3);
}

#[test]
fn mode_is_switchable_at_runtime() {
    let mut sel = EnsembleSelector::best_of(three_solvers());
    assert_eq!(sel.mode(), EnsembleMode::BestOf);
    sel.set_mode(EnsembleMode::MajorityVote);
    assert_eq!(sel.mode(), EnsembleMode::MajorityVote);
    sel.set_mode(EnsembleMode::WeightedByScore);
    assert_eq!(sel.mode(), EnsembleMode::WeightedByScore);
}

#[test]
fn empty_ensemble_panics_at_construction() {
    let r = std::panic::catch_unwind(|| EnsembleSelector::best_of(vec![]));
    assert!(r.is_err());
}

struct HangingSolver;
impl Solver for HangingSolver {
    fn id(&self) -> &'static str {
        "hanging"
    }
    fn solve(&self, _: &MatchInput) -> SolverOutput {
        // Never reports. Sleep in slices so the thread is interruptible by
        // process exit rather than spinning.
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// `timeout` used to be dead: the `Some(_t)` arm read `_t` and then joined
/// the handle anyway, so a hung solver blocked `run_parallel` forever and
/// callers relying on the documented deadline protection had none.
#[test]
fn timeout_stops_waiting_on_a_hung_solver() {
    let sel = EnsembleSelector::best_of(vec![
        RegisteredSolver::new(Arc::new(GreedySolver)),
        RegisteredSolver::new(Arc::new(HangingSolver)),
    ]);

    let started = std::time::Instant::now();
    let res = sel.run_parallel(&MatchInput::new("match-timeout", 0), Some(120));
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "the deadline must end the wait, took {elapsed:?}"
    );
    assert_eq!(res.all_outputs.len(), 2, "every slot is reported on");
    let hung = res
        .all_outputs
        .iter()
        .find(|o| o.score.solver_id == "hanging")
        .expect("the hung solver still has a slot");
    assert_eq!(hung.score.distinct_series, 0, "a straggler scores zero");
    assert_eq!(hung.score.total_balls, 0);
    assert_eq!(res.winner.score.solver_id, "greedy", "the healthy solver wins");
}
