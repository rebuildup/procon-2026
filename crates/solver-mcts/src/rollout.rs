//! Heuristic rollout for MCTS.
//!
//! Per `docs/contest/strategy.md` §5.3 Phase 3 the rollout favours
//! variety: collect distinct action indices for as long as the depth
//! budget allows, then break ties by `immediate_value`. The policy is
//! intentionally cheap — it runs millions of times per planner call.

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
