//! Heuristic rollout for MCTS.
//!
//! Per `docs/contest/strategy.md` §5.3 Phase 3 the rollout favours
//! variety: collect distinct action indices for as long as the depth
//! budget allows, then break ties by `immediate_value`. The policy is
//! intentionally cheap — it runs millions of times per planner call.
//!
//! This module also hosts [`RolloutPolicy`] (Issue #40): a pluggable
//! MCTS variant that combines UCB1 tree selection at the top with a
//! deterministic greedy rollout at the leaves. The greedy step is
//! supplied by anything implementing the [`GreedyRollout`] trait — the
//! default [`NearestSpotHeuristic`] picks the legal action with the
//! largest `immediate_value`.

use crate::node::MctsNode;
use crate::types::{Action, DayState};

/// Deterministic, allocation-light PRNG. xorshift64* is good enough for
/// tie-breaking inside the MCTS loop; the search itself is robust under
/// reseeding, so we don't need cryptographic quality.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Construct a PRNG from a non-zero seed. Zero is rewritten to a fixed
    /// fallback so callers can use `0` as "give me anything reproducible".
    pub fn from_seed(seed: u64) -> Self {
        let mut s = seed;
        if s == 0 {
            s = 0x9E37_79B9_7F4A_7C15;
        }
        Self(splitmix64(s))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform float in `[0, 1)`.
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    pub fn next_f64(&mut self) -> f64 {
        // Take the top 43 bits so the value fits comfortably inside the
        // f64 mantissa and divide by 2^43. Cast precision-loss is fine
        // here because the result lives in `[0, 1)`.
        let mantissa = (self.next_u64() >> 21) as f64;
        let denom = (1u64 << 43) as f64;
        mantissa / denom
    }

    /// Flip a biased coin. `p` is the probability of `true`.
    pub fn gen_bool(&mut self, p: f64) -> bool {
        if !(0.0..=1.0).contains(&p) {
            return false;
        }
        self.next_f64() < p
    }

    /// Integer in `[lo, hi)`. Panics if `lo >= hi`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn gen_range(&mut self, lo: usize, hi: usize) -> usize {
        assert!(lo < hi, "gen_range requires lo < hi");
        let span = hi - lo;
        lo + (self.next_u64() as usize % span)
    }

    /// Pick one element uniformly at random.
    pub fn choose<'a, T>(&mut self, slice: &'a [T]) -> Option<&'a T> {
        if slice.is_empty() { None } else { Some(&slice[self.gen_range(0, slice.len())]) }
    }
}

/// `SplitMix64` — used to scramble the seed so consecutive seeds don't
/// produce correlated xorshift streams.
fn splitmix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE8_9E5D);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Variety-aware heuristic rollout.
///
/// Walks the tree from `state` for at most `depth` steps. Each step picks
/// the greedy best action with probability `1 - epsilon` and a uniformly
/// random legal action with probability `epsilon`. The accumulated reward
/// is a discounted sum of `immediate_value` (`discount = 0.5`) so that
/// taking a high-value action in the near term dominates the result, plus
/// a tiny variety bonus proportional to the number of distinct actions
/// visited.
/// Discount factor applied to each rollout step beyond the immediate child
/// transition. A value well below `1.0` makes the immediate action dominate
/// the rollout reward (mirroring the spec's "near-term plans dominate" rule).
const ROLLOUT_DISCOUNT: f64 = 0.5;

pub fn rollout(state: &DayState, depth: u32, epsilon: f64, rng: &mut Rng) -> f64 {
    let mut current = state.clone();
    // Seed the reward with whatever the state already accumulated. The
    // rollout contributes *additional* discounted value on top; if we
    // started from 0 an action that greedily clears the pool would look
    // strictly worse than one that leaves a more lucrative sub-tree to
    // the rollouts.
    let mut reward = current.cumulative_value;
    let mut multiplier = 1.0;
    let mut distinct = 0u32;
    let mut last: Option<u16> = None;

    for _ in 0..depth {
        if current.is_terminal() {
            break;
        }
        let legal = current.legal_actions();
        if legal.is_empty() {
            break;
        }
        let action = if rng.gen_bool(epsilon) {
            rng.choose(&legal).copied().unwrap_or(legal[0])
        } else {
            greedy_best(&current, &legal)
        };
        reward += immediate_value(&current, action) * multiplier;
        multiplier *= ROLLOUT_DISCOUNT;
        if Some(action.0) != last {
            distinct = distinct.saturating_add(1);
            last = Some(action.0);
        }
        current = current.apply(action);
    }

    // Variety bonus: tiny weight that nudges rollouts toward visiting
    // distinct actions when reward margins are small.
    reward + f64::from(distinct) * 1e-3
}

/// Greedy best action within the legal set, breaking ties by lowest index.
fn greedy_best(state: &DayState, legal: &[Action]) -> Action {
    let mut best: Option<(Action, f64)> = None;
    for &a in legal {
        let v = immediate_value(state, a);
        match best {
            None => best = Some((a, v)),
            Some((_, bv)) if v > bv => best = Some((a, v)),
            _ => {}
        }
    }
    best.map_or(legal[0], |(a, _)| a)
}

fn immediate_value(state: &DayState, a: Action) -> f64 {
    state.pool.iter().find(|s| s.index == a.0).map_or(0.0, |s| s.immediate_value)
}

// =============================================================================
// Issue #40: UCB + greedy-rollout policy.
// =============================================================================

/// Plug-in greedy step heuristic used by [`RolloutPolicy`].
///
/// `greedy_step` is invoked once per rollout step after the tree has
/// expanded a fresh child. The trait accepts any "nearest-spot" style
/// heuristic; the canonical implementation is [`NearestSpotHeuristic`],
/// which simply picks the legal action with the largest `immediate_value`.
///
/// Implementations must return one of the legal actions in `state` when
/// the state is non-terminal. The caller guarantees this via the
/// `is_terminal` check in the rollout loop.
pub trait GreedyRollout {
    /// Pick the best greedy action from `state`.
    fn greedy_step(&self, state: &DayState) -> Action;
}

/// Default nearest-spot greedy heuristic.
///
/// Selects the legal action with the largest `immediate_value` from the
/// state's pool, breaking ties by lowest action index for determinism.
/// Mirrors the per-step objective in `docs/contest/strategy.md` §5.3.
#[derive(Debug, Clone, Copy, Default)]
pub struct NearestSpotHeuristic;

impl GreedyRollout for NearestSpotHeuristic {
    fn greedy_step(&self, state: &DayState) -> Action {
        let legal = state.legal_actions();
        let mut best: Option<(Action, f64)> = None;
        for a in legal {
            let v = state
                .pool
                .iter()
                .find(|s| s.index == a.0)
                .map_or(f64::NEG_INFINITY, |s| s.immediate_value);
            match best {
                None => best = Some((a, v)),
                Some((_, bv)) if v > bv => best = Some((a, v)),
                _ => {}
            }
        }
        best.map_or(Action(0), |(a, _)| a)
    }
}

/// Outcome of a single [`RolloutPolicy::search`] call.
///
/// Mirrors the telemetry surface of [`crate::solver::PickedAction`] but
/// is kept separate so the rollout-policy module does not have to depend
/// on the [`crate::solver`] module's internal types.
#[derive(Debug, Clone, PartialEq)]
pub struct RolloutOutcome {
    /// Action the policy recommends for the current step.
    pub action: Action,
    /// Number of times the policy visited the winning root child.
    pub visits: u32,
    /// Mean greedy-rollout reward across those visits.
    pub mean_reward: f64,
    /// Total number of `select -> expand -> rollout -> backprop` cycles
    /// that ran during the search.
    pub iterations_run: u32,
}

/// Errors a [`RolloutPolicy::search`] call may surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RolloutError {
    /// The state admits no legal action — nothing to plan.
    EmptyActionSet,
}

impl std::fmt::Display for RolloutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyActionSet => f.write_str("no legal actions available"),
        }
    }
}

impl std::error::Error for RolloutError {}

/// UCB1 tree selection + deterministic greedy rollout.
///
/// Each iteration of [`RolloutPolicy::search`] runs the classical MCTS
/// loop, with a fully deterministic rollout at the leaves (no ε-random):
///
/// 1. **Select**: pick a root child via UCB1
///    (`ucb_c * sqrt(ln(N_parent) / N_child)`), or expand an
///    un-visited legal action when any remain.
/// 2. **Expand**: append the chosen action as a new child of the root.
/// 3. **Rollout**: starting from the post-action state, walk forward
///    for at most `rollout_max_steps` steps calling
///    [`GreedyRollout::greedy_step`] each iteration. The reward is the
///    `cumulative_value` of the resulting state.
/// 4. **Back-propagate**: credit the rollout reward to the root and to
///    the expanded child. The tree is depth-1, so the path is two
///    nodes.
///
/// At the end of the search the most-visited child wins (the standard
/// "robust child" rule — lower variance than max-mean).
#[derive(Debug, Clone, Copy)]
pub struct RolloutPolicy {
    /// UCB1 exploration constant. Mirrors the `c` term in
    /// `exploit + c * sqrt(ln(N_parent) / N_child)`. Defaults to `√2`.
    pub ucb_c: f64,
    /// Maximum number of steps the greedy rollout may take from the
    /// newly-expanded leaf before terminating.
    pub rollout_max_steps: usize,
    /// PRNG seed. `0` is rewritten to a fixed fallback by
    /// [`Rng::from_seed`]; the rollout itself is deterministic so the
    /// seed is mostly bookkeeping for future ε-random extensions.
    pub seed: u64,
    /// Number of `select -> expand -> rollout -> backprop` cycles to
    /// run per search call.
    pub iterations: u32,
}

impl Default for RolloutPolicy {
    fn default() -> Self {
        Self { ucb_c: core::f64::consts::SQRT_2, rollout_max_steps: 32, seed: 0, iterations: 1_000 }
    }
}

impl RolloutPolicy {
    /// Construct a policy with the UCB constant, rollout cap, and seed
    /// exactly as the Issue #40 spec requests. Iteration count defaults
    /// to `1_000`; override with [`RolloutPolicy::with_iterations`].
    pub fn new(ucb_c: f64, rollout_max_steps: usize, seed: u64) -> Self {
        Self { ucb_c, rollout_max_steps, seed, iterations: 1_000 }
    }

    /// Set the iteration budget. Builder-style for fluent setup.
    #[must_use]
    pub fn with_iterations(mut self, iterations: u32) -> Self {
        self.iterations = iterations;
        self
    }

    /// Run the UCB + greedy-rollout search against `state` using
    /// `greedy` as the per-step heuristic. Returns the most-visited
    /// root child once `iterations` cycles have completed.
    ///
    /// Returns `Err(RolloutError::EmptyActionSet)` if the state admits
    /// no legal action.
    pub fn search<G: GreedyRollout>(
        &self,
        state: &DayState,
        greedy: &G,
    ) -> Result<RolloutOutcome, RolloutError> {
        let legal = state.legal_actions();
        if legal.is_empty() {
            return Err(RolloutError::EmptyActionSet);
        }
        // Seed the PRNG for API parity with MctsSolver; the rollout is
        // deterministic so the stream is unused inside the hot loop.
        let _rng = Rng::from_seed(self.seed);
        let mut root = MctsNode::root();
        let mut explored: Vec<Action> = Vec::with_capacity(legal.len());

        for _ in 0..self.iterations {
            // 1) Pick a child: expand a new action, or descend via UCB1.
            let (action, is_new) = if explored.len() < legal.len() {
                let next =
                    legal.iter().copied().find(|a| !explored.contains(a)).unwrap_or(legal[0]);
                explored.push(next);
                (next, true)
            } else {
                let idx = root.best_child_ucb1(root.visits.max(1), self.ucb_c).unwrap_or(0);
                let child = &root.children[idx];
                (child.parent_action.unwrap_or(legal[0]), false)
            };

            // 2) Expand.
            if is_new {
                root.expand(action);
            }

            // 3) Greedy rollout from the post-action state.
            let reward = greedy_rollout_reward(state, action, greedy, self.rollout_max_steps);

            // 4) Backprop. Depth-1 tree → root + one child.
            apply_root_update(&mut root, action, reward);
        }

        let idx = root.most_visited_child().ok_or(RolloutError::EmptyActionSet)?;
        let child = &root.children[idx];
        let visits = child.visits;
        Ok(RolloutOutcome {
            action: child.parent_action.unwrap_or(legal[0]),
            visits,
            mean_reward: if visits == 0 { 0.0 } else { child.total_reward / f64::from(visits) },
            iterations_run: root.visits,
        })
    }
}

/// Plug [`RolloutPolicy`] into the local [`crate::Solver`] trait using
/// [`NearestSpotHeuristic`] as the per-step heuristic. Callers needing
/// a custom heuristic should call [`RolloutPolicy::search`] directly.
impl crate::Solver for RolloutPolicy {
    type State = DayState;
    type Action = Action;
    type Error = RolloutError;

    fn plan_step(
        &self,
        state: &Self::State,
        _time_budget: std::time::Duration,
    ) -> Result<Self::Action, Self::Error> {
        let picked = self.search(state, &NearestSpotHeuristic)?;
        Ok(picked.action)
    }
}

/// Greedy rollout starting from the post-`first_action` state.
///
/// Applies `first_action` once, then calls `greedy.greedy_step` for at
/// most `max_steps` further steps (or until the state is terminal).
/// The returned reward is the `cumulative_value` of the final state —
/// this makes the rollout's credit assignment straightforwardly equal to
/// the per-step `immediate_value` sum along the walked path.
fn greedy_rollout_reward<G: GreedyRollout>(
    initial: &DayState,
    first_action: Action,
    greedy: &G,
    max_steps: usize,
) -> f64 {
    let mut current = initial.apply(first_action);
    let mut steps_left = max_steps;
    while steps_left > 0 && !current.is_terminal() {
        let action = greedy.greedy_step(&current);
        current = current.apply(action);
        steps_left -= 1;
    }
    current.cumulative_value
}

/// Local backprop helper. Mirrors the one in [`crate::solver`]; kept
/// here so this module does not depend on the solver module's
/// internals.
fn apply_root_update(root: &mut MctsNode, action: Action, reward: f64) {
    root.visits = root.visits.saturating_add(1);
    for child in &mut root.children {
        if child.parent_action == Some(action) {
            child.visits = child.visits.saturating_add(1);
            child.total_reward += reward;
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ActionSpec, DayState};

    fn fixture() -> DayState {
        DayState::new(
            vec![
                ActionSpec { index: 0, immediate_value: 1.0 },
                ActionSpec { index: 1, immediate_value: 5.0 },
                ActionSpec { index: 2, immediate_value: 2.0 },
                ActionSpec { index: 3, immediate_value: 3.0 },
            ],
            16,
        )
    }

    #[test]
    fn greedy_best_picks_max_value() {
        let s = fixture();
        let legal = s.legal_actions();
        let a = greedy_best(&s, &legal);
        assert_eq!(a, Action(1));
    }

    #[test]
    fn rollout_with_zero_epsilon_is_greedy() {
        let s = fixture();
        let mut rng = Rng::from_seed(7);
        let score = rollout(&s, 4, 0.0, &mut rng);
        // Discounted (DISCOUNT=0.5): 5*1 + 3*0.5 + 2*0.25 + 1*0.125 = 7.125,
        // plus tiny variety bonus ≈ 7.13.
        assert!((7.05..=7.2).contains(&score), "score={score}");
    }

    #[test]
    fn rng_zero_seed_is_deterministic() {
        let mut a = Rng::from_seed(0);
        let mut b = Rng::from_seed(0);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }
}
