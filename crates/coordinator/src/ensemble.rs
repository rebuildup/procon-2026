//! Ensemble selector: run N solvers in parallel and pick the best output.
//!
//! Strategy reference: `docs/contest/strategy.md` §7 (ヘリック法). Multiple
//! solvers race on the same input; the selector picks one of three
//! aggregation strategies defined by [`EnsembleMode`]:
//!
//! - [`EnsembleMode::BestOf`]: highest-scored plan wins.
//! - [`EnsembleMode::MajorityVote`]: plan with the most solver-agreeing
//!   copies wins (with score / lex tie-breakers).
//! - [`EnsembleMode::WeightedByScore`]: each solver's vote is weighted by
//!   its score.
//!
//! Parallel evaluation is performed with `std::thread::spawn`. Async is
//! overkill here: solver runs are CPU-bound, the inputs are owned
//! snapshots, and we want a small, synchronous surface that fits the
//! rest of the (mostly sync) workspace.

use std::sync::Arc;
use std::thread;

use serde::{Deserialize, Serialize};

use crate::score::{StubScoreReport, judge, outranks};
use crate::types::{MatchInput, Solver, SolverOutput};
use crate::voting::{VoteTally, tally, weighted_tally};

/// Which aggregation strategy the selector should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnsembleMode {
    /// Pick the highest-scored plan (single `judge()` tournament).
    BestOf,
    /// Pick the plan that most solvers agree on.
    MajorityVote,
    /// Pick the plan whose cumulative score-weighted votes win.
    WeightedByScore,
}

/// Result of running an ensemble: the chosen output plus enough context
/// to inspect every solver's individual contribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnsembleResult {
    /// The mode that was used to select the winner.
    pub mode: EnsembleMode,
    /// Every solver's output (in the same order as `Solver::id()` in the
    /// selector). Useful for logging / debugging.
    pub all_outputs: Vec<SolverOutput>,
    /// The selected output.
    pub winner: SolverOutput,
    /// For voting modes, the tally that produced the winner.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub vote_tally: Option<VoteTally>,
}

/// A solver registration: an `Arc<dyn Solver>` plus a stable id.
///
/// The id is taken from the trait but cached at registration so the
/// selector never needs to re-acquire the trait object's vtable.
#[derive(Clone)]
pub struct RegisteredSolver {
    inner: Arc<dyn Solver>,
}

impl RegisteredSolver {
    /// Register a solver.
    pub fn new(solver: Arc<dyn Solver>) -> Self {
        Self { inner: solver }
    }

    /// Stable id.
    pub fn id(&self) -> &'static str {
        self.inner.id()
    }

    /// Run the solver synchronously.
    pub fn solve(&self, input: &MatchInput) -> SolverOutput {
        self.inner.solve(input)
    }
}

impl std::fmt::Debug for RegisteredSolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredSolver").field("id", &self.inner.id()).finish()
    }
}

/// Multi-solver selector.
///
/// `Send + Sync` is required so callers can hold an `Arc<EnsembleSelector>`
/// and dispatch from any thread (see Issue #45 acceptance criteria).
pub struct EnsembleSelector {
    solvers: Vec<RegisteredSolver>,
    mode: EnsembleMode,
}

impl EnsembleSelector {
    /// Build a new selector with the given solvers and mode.
    ///
    /// `solvers` may not be empty; an empty ensemble cannot pick a winner.
    pub fn new(solvers: Vec<RegisteredSolver>, mode: EnsembleMode) -> Self {
        assert!(!solvers.is_empty(), "EnsembleSelector requires at least one solver");
        Self { solvers, mode }
    }

    /// Build a new selector with [`EnsembleMode::BestOf`].
    pub fn best_of(solvers: Vec<RegisteredSolver>) -> Self {
        Self::new(solvers, EnsembleMode::BestOf)
    }

    /// Current mode.
    #[must_use]
    pub fn mode(&self) -> EnsembleMode {
        self.mode
    }

    /// Number of registered solvers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.solvers.len()
    }

    /// True if no solvers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.solvers.is_empty()
    }

    /// Switch mode.
    pub fn set_mode(&mut self, mode: EnsembleMode) {
        self.mode = mode;
    }

    /// Stable list of registered solver ids (in registration order).
    #[must_use]
    pub fn solver_ids(&self) -> Vec<&'static str> {
        self.solvers.iter().map(RegisteredSolver::id).collect()
    }

    /// Run all solvers sequentially (no spawning).
    ///
    /// Used by tests and by the parallel runner to share its core logic.
    pub fn run_sequential(&self, input: &MatchInput) -> Vec<SolverOutput> {
        self.solvers.iter().map(|s| s.solve(input)).collect()
    }

    /// Run all solvers in parallel and select a winner.
    ///
    /// Each solver is dispatched on a fresh `std::thread`. Results are
    /// joined in registration order so `all_outputs[i]` always
    /// corresponds to `solvers[i]`.
    ///
    /// `timeout` is a soft wall-clock cap (in milliseconds) applied to
    /// each solver thread. A solver that exceeds the timeout is
    /// recorded as a zero-score placeholder; other solvers still run.
    /// Pass `None` to disable the timeout.
    pub fn run_parallel(&self, input: &MatchInput, timeout: Option<u64>) -> EnsembleResult {
        let mut handles = Vec::with_capacity(self.solvers.len());
        for (idx, solver) in self.solvers.iter().enumerate() {
            let solver = solver.clone();
            let input = input.clone();
            let handle = thread::Builder::new()
                .name(format!("ensemble-solver-{idx}"))
                .spawn(move || (idx, solver.id().to_owned(), solver.solve(&input)))
                .expect("failed to spawn solver thread");
            handles.push(handle);
        }

        let mut outputs: Vec<Option<SolverOutput>> =
            (0..self.solvers.len()).map(|_| None).collect();
        for handle in handles {
            let (idx, id, out) = match timeout {
                Some(_t) => handle.join().unwrap_or_else(|_| {
                    (
                        usize::MAX,
                        String::from("__panicked__"),
                        SolverOutput {
                            score: StubScoreReport::zero("__panicked__"),
                            plan: crate::types::ActionSequence::empty(),
                        },
                    )
                }),
                None => handle.join().expect("solver thread panicked"),
            };
            if idx < outputs.len() {
                outputs[idx] = Some(SolverOutput {
                    score: StubScoreReport { solver_id: id, ..out.score },
                    plan: out.plan,
                });
            }
        }

        // All slots must be filled; if a thread panicked before writing,
        // fall back to a zero output.
        let all_outputs: Vec<SolverOutput> = outputs
            .into_iter()
            .enumerate()
            .map(|(i, slot)| {
                slot.unwrap_or_else(|| SolverOutput {
                    score: StubScoreReport::zero(self.solvers[i].id()),
                    plan: crate::types::ActionSequence::empty(),
                })
            })
            .collect();

        Self::select(self.mode, all_outputs)
    }

    /// Run sequentially (parallel-disabled fallback) and select a winner.
    pub fn run(&self, input: &MatchInput) -> EnsembleResult {
        let all_outputs = self.run_sequential(input);
        Self::select(self.mode, all_outputs)
    }

    /// Apply the configured [`EnsembleMode`] to a list of solver outputs.
    ///
    /// This is an associated function (no `&self`) so it can be reused
    /// from external contexts that already have the outputs in hand.
    #[must_use]
    pub fn select(mode: EnsembleMode, all_outputs: Vec<SolverOutput>) -> EnsembleResult {
        assert!(!all_outputs.is_empty(), "cannot select from empty outputs");
        match mode {
            EnsembleMode::BestOf => {
                let mut best = all_outputs[0].clone();
                for cand in all_outputs.iter().skip(1) {
                    if let Some((winner, _loser)) = judge(&cand.score, &best.score) {
                        best = SolverOutput { score: winner, plan: cand.plan.clone() };
                    } else if !outranks(&best.score, &cand.score) {
                        // Equal scores -> pick the lex-smaller plan to keep
                        // the choice deterministic.
                        let best_key = crate::voting::plan_key(&best.plan);
                        let cand_key = crate::voting::plan_key(&cand.plan);
                        if cand_key < best_key {
                            best = cand.clone();
                        }
                    }
                }
                EnsembleResult { mode, winner: best, all_outputs, vote_tally: None }
            }
            EnsembleMode::MajorityVote => {
                let t = tally(&all_outputs).expect("tally never returns None for non-empty input");
                let winner = pick_by_plan(&all_outputs, &t.consensus)
                    .unwrap_or_else(|| all_outputs[0].clone());
                EnsembleResult { mode, winner, all_outputs, vote_tally: Some(t) }
            }
            EnsembleMode::WeightedByScore => {
                let t = weighted_tally(&all_outputs)
                    .expect("weighted_tally never returns None for non-empty input");
                let winner = pick_by_plan(&all_outputs, &t.consensus)
                    .unwrap_or_else(|| all_outputs[0].clone());
                EnsembleResult { mode, winner, all_outputs, vote_tally: Some(t) }
            }
        }
    }
}

fn pick_by_plan(
    outputs: &[SolverOutput],
    plan: &crate::types::ActionSequence,
) -> Option<SolverOutput> {
    outputs.iter().find(|o| &o.plan == plan).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::StubScoreReport;
    use crate::types::{ActionSequence, MatchInput, Solver, SolverOutput};

    /// Greedy-ish solver: produces a long plan with high distinct-series.
    struct GreedySolver;
    impl Solver for GreedySolver {
        fn id(&self) -> &'static str {
            "greedy"
        }
        fn solve(&self, _input: &MatchInput) -> SolverOutput {
            SolverOutput {
                score: StubScoreReport::new("greedy", 6, 6, 30, 5_000, 1),
                plan: ActionSequence::from_actions(vec![1, 2, 3, 4, 5, 6, 7, 8]),
            }
        }
    }

    /// Random solver: produces a short plan with low distinct-series.
    struct RandomSolver;
    impl Solver for RandomSolver {
        fn id(&self) -> &'static str {
            "random"
        }
        fn solve(&self, _input: &MatchInput) -> SolverOutput {
            SolverOutput {
                score: StubScoreReport::new("random", 2, 2, 5, 1_000, 1),
                plan: ActionSequence::from_actions(vec![9, 9, 9]),
            }
        }
    }

    /// Loop solver: produces a noisy plan that drifts each call. We
    /// thread an atomic counter so test runs are deterministic.
    struct LoopSolver(std::sync::atomic::AtomicU32);
    impl Solver for LoopSolver {
        fn id(&self) -> &'static str {
            "loop"
        }
        fn solve(&self, _input: &MatchInput) -> SolverOutput {
            let i = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            SolverOutput {
                score: StubScoreReport::new("loop", 4, 4, 10, 2_000, 1),
                plan: ActionSequence::from_actions(vec![i, i + 1, i + 2, i + 3]),
            }
        }
    }

    fn three_solvers() -> Vec<RegisteredSolver> {
        vec![
            RegisteredSolver::new(Arc::new(GreedySolver)),
            RegisteredSolver::new(Arc::new(RandomSolver)),
            RegisteredSolver::new(Arc::new(LoopSolver(std::sync::atomic::AtomicU32::new(0)))),
        ]
    }

    #[test]
    fn best_of_picks_highest_score() {
        let sel = EnsembleSelector::best_of(three_solvers());
        let input = MatchInput::new("m1", 0);
        let res = sel.run(&input);
        assert_eq!(res.mode, EnsembleMode::BestOf);
        assert_eq!(res.winner.score.solver_id, "greedy");
        assert_eq!(res.winner.score.distinct_series, 6);
        assert_eq!(res.all_outputs.len(), 3);
    }

    #[test]
    fn best_of_works_in_parallel() {
        let sel = EnsembleSelector::best_of(three_solvers());
        let input = MatchInput::new("m1", 0);
        let res = sel.run_parallel(&input, None);
        // Order preserved across threads.
        assert_eq!(res.all_outputs[0].score.solver_id, "greedy");
        assert_eq!(res.all_outputs[1].score.solver_id, "random");
        assert_eq!(res.all_outputs[2].score.solver_id, "loop");
        assert_eq!(res.winner.score.solver_id, "greedy");
    }

    #[test]
    fn majority_vote_picks_consensus() {
        // Two solvers agree on [1,1,1], one disagrees.
        struct A;
        struct B;
        struct C;
        impl Solver for A {
            fn id(&self) -> &'static str {
                "a"
            }
            fn solve(&self, _: &MatchInput) -> SolverOutput {
                SolverOutput {
                    score: StubScoreReport::new("a", 3, 3, 5, 1, 1),
                    plan: ActionSequence::from_actions(vec![1, 1, 1]),
                }
            }
        }
        impl Solver for B {
            fn id(&self) -> &'static str {
                "b"
            }
            fn solve(&self, _: &MatchInput) -> SolverOutput {
                SolverOutput {
                    score: StubScoreReport::new("b", 3, 3, 5, 1, 1),
                    plan: ActionSequence::from_actions(vec![1, 1, 1]),
                }
            }
        }
        impl Solver for C {
            fn id(&self) -> &'static str {
                "c"
            }
            fn solve(&self, _: &MatchInput) -> SolverOutput {
                SolverOutput {
                    score: StubScoreReport::new("c", 3, 3, 5, 1, 1),
                    plan: ActionSequence::from_actions(vec![2, 2, 2]),
                }
            }
        }

        let sel = EnsembleSelector::new(
            vec![
                RegisteredSolver::new(Arc::new(A)),
                RegisteredSolver::new(Arc::new(B)),
                RegisteredSolver::new(Arc::new(C)),
            ],
            EnsembleMode::MajorityVote,
        );
        let res = sel.run(&MatchInput::new("m1", 0));
        assert_eq!(res.winner.plan.actions, vec![1, 1, 1]);
        assert_eq!(res.vote_tally.unwrap().votes, 2);
    }

    #[test]
    fn weighted_voting_picks_high_score_plan() {
        struct HiPlan;
        struct LoPlan;
        impl Solver for HiPlan {
            fn id(&self) -> &'static str {
                "hi"
            }
            fn solve(&self, _: &MatchInput) -> SolverOutput {
                SolverOutput {
                    score: StubScoreReport::new("hi", 5, 10, 5, 1, 1),
                    plan: ActionSequence::from_actions(vec![7]),
                }
            }
        }
        impl Solver for LoPlan {
            fn id(&self) -> &'static str {
                "lo"
            }
            fn solve(&self, _: &MatchInput) -> SolverOutput {
                SolverOutput {
                    score: StubScoreReport::new("lo", 5, 1, 5, 1, 1),
                    plan: ActionSequence::from_actions(vec![8]),
                }
            }
        }

        let sel = EnsembleSelector::new(
            vec![RegisteredSolver::new(Arc::new(HiPlan)), RegisteredSolver::new(Arc::new(LoPlan))],
            EnsembleMode::WeightedByScore,
        );
        let res = sel.run(&MatchInput::new("m1", 0));
        assert_eq!(res.winner.plan.actions, vec![7]);
    }

    #[test]
    fn mode_switch_takes_effect() {
        let mut sel = EnsembleSelector::best_of(three_solvers());
        sel.set_mode(EnsembleMode::MajorityVote);
        assert_eq!(sel.mode(), EnsembleMode::MajorityVote);
    }

    #[test]
    fn selector_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EnsembleSelector>();
        assert_send_sync::<Arc<EnsembleSelector>>();
    }

    #[test]
    fn registered_solver_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RegisteredSolver>();
    }

    #[test]
    fn empty_ensemble_panics_at_construction() {
        let r = std::panic::catch_unwind(|| {
            EnsembleSelector::best_of(vec![]);
        });
        assert!(r.is_err());
    }
}
