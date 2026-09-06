//! Voting: action-sequence consensus across multiple solver outputs.
//!
//! Strategy reference: `docs/contest/strategy.md` §7 (ヘリック法, multi-solver
//! race). Voting is one of the three ensemble modes (alongside
//! `BestOf` and `WeightedByScore`) described in Issue #45.
//!
//! Voting metric: two [`ActionSequence`]s are considered the same plan when
//! every element matches (exact prefix match). Ties on the count fall back
//! to the higher-scored plan, then the lexicographically smaller plan id.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::score::StubScoreReport;
use crate::types::{ActionSequence, SolverOutput};

/// Result of a voting tally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteTally {
    /// The consensus action sequence.
    pub consensus: ActionSequence,
    /// Number of solvers that produced this plan (out of `total_votes`).
    pub votes: usize,
    /// Total number of solvers that contributed.
    pub total_votes: usize,
    /// Score of the winning plan (taken from the highest-scored solver
    /// that emitted this plan).
    pub score: StubScoreReport,
}

impl VoteTally {
    /// True if the consensus reached an absolute majority of the votes.
    #[must_use]
    pub fn has_majority(&self) -> bool {
        self.votes * 2 > self.total_votes
    }
}

/// Tally a slice of solver outputs and return the consensus plan.
///
/// Behaviour:
/// - Groups outputs by their [`ActionSequence`].
/// - Picks the group with the most votes.
/// - On ties (two groups with the same vote count), picks the group whose
///   member has the highest score under
///   [`crate::score::outranks`] (using the official priority order).
/// - On score ties, picks the lexicographically smaller plan id.
///
/// If `outputs` is empty, returns `None`.
#[must_use]
pub fn tally(outputs: &[SolverOutput]) -> Option<VoteTally> {
    if outputs.is_empty() {
        return None;
    }

    // Group (plan -> list of (score, solver_id)).
    let mut groups: HashMap<ActionSequence, Vec<&StubScoreReport>> = HashMap::new();
    for out in outputs {
        groups.entry(out.plan.clone()).or_default().push(&out.score);
    }

    let total_votes = outputs.len();

    // Pick the winner: most votes, then highest in-group score, then
    // lexicographically smallest plan key.
    let mut best: Option<(ActionSequence, usize, StubScoreReport)> = None;
    for (plan, scores) in &groups {
        let votes = scores.len();
        // Choose the highest-scoring member as the group's representative.
        let mut rep = scores[0].clone();
        for s in scores.iter().skip(1) {
            if crate::score::outranks(s, &rep) {
                rep = (*s).clone();
            }
        }
        match &best {
            None => best = Some((plan.clone(), votes, rep)),
            Some((best_plan, best_votes, best_rep)) => {
                if votes > *best_votes
                    || (votes == *best_votes && crate::score::outranks(&rep, best_rep))
                    || (votes == *best_votes
                        && !crate::score::outranks(&rep, best_rep)
                        && !crate::score::outranks(best_rep, &rep)
                        && plan_key(plan) < plan_key(best_plan))
                {
                    best = Some((plan.clone(), votes, rep));
                }
            }
        }
    }

    let (consensus, votes, score) = best?;
    Some(VoteTally { consensus, votes, total_votes, score })
}

/// Produce a stable string key for an [`ActionSequence`].
///
/// Used as a tie-breaker so that the choice between tied groups is
/// deterministic across runs and platforms.
#[must_use]
pub fn plan_key(plan: &ActionSequence) -> String {
    let mut s = String::new();
    for (i, a) in plan.actions.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&a.to_string());
    }
    s
}

/// Weighted voting: each solver contributes a vote whose weight is its
/// score (higher score -> larger weight).
///
/// Weights are taken from the four-metric
/// [`StubScoreReport`](crate::score::StubScoreReport) using
/// `cumulative_distinct_series` as the weight (matches official priority
/// #2 — close enough for the stub, easy to swap for the real metric when
/// `bench::ScoreReport::judge` lands).
///
/// Returns the same [`VoteTally`] shape as [`tally`]; `votes` here is the
/// **sum of weights** that landed on the winning group.
#[must_use]
pub fn weighted_tally(outputs: &[SolverOutput]) -> Option<VoteTally> {
    if outputs.is_empty() {
        return None;
    }

    let mut groups: HashMap<ActionSequence, (u64, StubScoreReport)> = HashMap::new();
    for out in outputs {
        let weight = u64::from(out.score.cumulative_distinct_series) + 1;
        groups
            .entry(out.plan.clone())
            .and_modify(|(w, rep)| {
                *w += weight;
                if crate::score::outranks(&out.score, rep) {
                    *rep = out.score.clone();
                }
            })
            .or_insert((weight, out.score.clone()));
    }

    let total_weight: u64 =
        outputs.iter().map(|o| u64::from(o.score.cumulative_distinct_series) + 1).sum();

    let mut best: Option<(ActionSequence, u64, StubScoreReport)> = None;
    for (plan, (weight, rep)) in &groups {
        match &best {
            None => best = Some((plan.clone(), *weight, rep.clone())),
            Some((best_plan, best_weight, best_rep)) => {
                if weight > best_weight
                    || (weight == best_weight && crate::score::outranks(rep, best_rep))
                    || (weight == best_weight
                        && !crate::score::outranks(rep, best_rep)
                        && !crate::score::outranks(best_rep, rep)
                        && plan_key(plan) < plan_key(best_plan))
                {
                    best = Some((plan.clone(), *weight, rep.clone()));
                }
            }
        }
    }

    let (consensus, weight, score) = best?;
    Some(VoteTally {
        consensus,
        // For weighted tally, `votes` carries the winning group's weight
        // so callers can recover the relative strength.
        votes: usize::try_from(weight).unwrap_or(usize::MAX),
        total_votes: usize::try_from(total_weight).unwrap_or(usize::MAX),
        score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::StubScoreReport;
    use crate::types::SolverOutput;

    fn make_output(id: &str, plan: Vec<u32>, distinct: u32, cumulative: u32) -> SolverOutput {
        SolverOutput {
            score: StubScoreReport::new(id, distinct, cumulative, 0, 0, 0),
            plan: ActionSequence::from_actions(plan),
        }
    }

    #[test]
    fn tally_empty_returns_none() {
        assert!(tally(&[]).is_none());
        assert!(weighted_tally(&[]).is_none());
    }

    #[test]
    fn unanimous_consensus() {
        let outputs =
            vec![make_output("a", vec![1, 2, 3], 5, 5), make_output("b", vec![1, 2, 3], 5, 5)];
        let tally = tally(&outputs).unwrap();
        assert_eq!(tally.consensus.actions, vec![1, 2, 3]);
        assert_eq!(tally.votes, 2);
        assert_eq!(tally.total_votes, 2);
        assert!(tally.has_majority());
    }

    #[test]
    fn simple_majority_wins() {
        let outputs = vec![
            make_output("a", vec![1, 1, 1], 3, 3),
            make_output("b", vec![1, 1, 1], 3, 3),
            make_output("c", vec![2, 2, 2], 3, 3),
        ];
        let tally = tally(&outputs).unwrap();
        assert_eq!(tally.consensus.actions, vec![1, 1, 1]);
        assert_eq!(tally.votes, 2);
    }

    #[test]
    fn ties_break_by_score() {
        // Two plans tied at 2 votes each; the one with higher cumulative
        // distinct-series wins.
        let outputs = vec![
            make_output("a", vec![1], 3, 9),
            make_output("b", vec![1], 3, 9),
            make_output("c", vec![2], 3, 1),
            make_output("d", vec![2], 3, 1),
        ];
        let tally = tally(&outputs).unwrap();
        assert_eq!(tally.consensus.actions, vec![1]);
    }

    #[test]
    fn ties_break_by_lex_order_when_score_identical() {
        let outputs = vec![make_output("a", vec![9, 9], 1, 1), make_output("b", vec![1, 1], 1, 1)];
        let tally = tally(&outputs).unwrap();
        // 1 vote each -> identical score -> lex smaller plan id wins.
        assert_eq!(tally.consensus.actions, vec![1, 1]);
    }

    #[test]
    fn weighted_tally_breaks_on_score_weight() {
        // Plan [1] appears once with high score; plan [2] appears once
        // with low score. Weights: [1] gets 10+1, [2] gets 1+1.
        let outputs = vec![make_output("hi", vec![1], 5, 10), make_output("lo", vec![2], 5, 1)];
        let tally = weighted_tally(&outputs).unwrap();
        assert_eq!(tally.consensus.actions, vec![1]);
    }

    #[test]
    fn plan_key_is_stable_and_sorted() {
        let a = ActionSequence::from_actions(vec![1, 2, 3]);
        let b = ActionSequence::from_actions(vec![1, 2, 4]);
        assert!(plan_key(&a) < plan_key(&b));
    }

    #[test]
    fn empty_plan_still_groups() {
        let outputs = vec![
            make_output("a", vec![], 1, 1),
            make_output("b", vec![], 1, 1),
            make_output("c", vec![1], 1, 1),
        ];
        let tally = tally(&outputs).unwrap();
        assert!(tally.consensus.is_empty());
        assert_eq!(tally.votes, 2);
    }
}
