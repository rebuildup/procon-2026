//! Simulated-annealing temperature schedule and Metropolis acceptance.
//!
//! The schedule is geometric decay:
//!
//! ```text
//! T(iter) = max(T_min, T_init * alpha^iter)
//! ```
//!
//! where `alpha = (T_min / T_init) ^ (1 / max_iter)` so that the temperature
//! reaches `T_min` exactly at `iter == max_iter`. This keeps the
//! user-facing knobs (`T_init`, `T_min`, `max_iter`) sufficient and avoids
//! having to specify a separate cooling rate.
//!
//! Acceptance uses the standard Metropolis rule:
//!
//! ```text
//! accept = delta >= 0 || uniform() < exp(delta / T)
//! ```
//!
//! where `delta = new_energy - old_energy` is the *improvement* (positive
//! means the new solution is worse). For score-based objectives (where
//! higher is better) we negate the energy when feeding it into
//! [`accept`], or expose [`accept_score`] directly to keep the
//! "improvement = positive" convention ergonomic.

use serde::{Deserialize, Serialize};

use crate::neighbors::NeighborhoodRng;

/// Linear-cooling and geometric-cooling share a small surface — a
/// [`Schedule`] holds the three knobs the issue mandates (`T_init`,
/// `T_min`, `max_iter`) and computes the per-iteration temperature.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    /// Initial temperature. Higher values accept more uphill moves early.
    pub t_init: f64,
    /// Floor temperature. The schedule never drops below this.
    pub t_min: f64,
    /// Iteration budget. The schedule reaches `t_min` at this iteration.
    pub max_iter: u32,
}

impl Schedule {
    /// Construct a schedule. Panics (in `debug_assert`) if the parameters
    /// are nonsensical so that misconfigurations surface during testing.
    #[must_use]
    pub fn new(t_init: f64, t_min: f64, max_iter: u32) -> Self {
        debug_assert!(t_init > 0.0, "t_init must be positive");
        debug_assert!(t_min >= 0.0, "t_min must be non-negative");
        debug_assert!(t_min <= t_init, "t_min must be <= t_init");
        debug_assert!(max_iter > 0, "max_iter must be > 0");
        Self { t_init, t_min, max_iter }
    }

    /// Cooling rate `alpha` such that `t_init * alpha^max_iter == t_min`.
    #[must_use]
    pub fn alpha(&self) -> f64 {
        if self.max_iter == 0 {
            return 1.0;
        }
        if self.t_min <= 0.0 || self.t_init <= 0.0 {
            return 1.0;
        }
        // Equivalent to (t_min/t_init)^(1/max_iter). Clamp the base to a
        // small positive number so degenerate inputs do not produce NaN.
        let base = (self.t_min / self.t_init).clamp(f64::MIN_POSITIVE, 1.0);
        base.powf(1.0 / f64::from(self.max_iter))
    }

    /// Temperature at iteration `iter` (0-indexed). Returns `t_min` once
    /// `iter >= max_iter`.
    #[must_use]
    pub fn temperature(&self, iter: u32) -> f64 {
        if iter >= self.max_iter {
            return self.t_min;
        }
        // `powi` requires i32; iter is bounded by max_iter which is u32.
        // Saturate the cast to keep the math safe even for implausible
        // inputs (max_iter > i32::MAX).
        let exp = i32::try_from(iter).unwrap_or(i32::MAX);
        let t = self.t_init * self.alpha().powi(exp);
        t.max(self.t_min)
    }
}

/// Energy form used by [`accept`]. When [`Delta::Score`] is used the
/// caller passes a positive value for improvement and a negative value
/// for worsening; this matches the SA literature and lets us share the
/// same math with both score-maximisation and cost-minimisation callers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Delta {
    /// Direct energy delta: positive = better, negative = worse.
    Energy(i64),
    /// Score delta: positive = worse, negative = better.
    /// (Use this for contest scoring where `score.energy()` is the value
    /// we want to maximise; the SA loop negates the score energy so the
    /// "worse = positive" convention still applies.)
    Score(i64),
}

impl Delta {
    /// Internal: the sign-corrected delta used by the Metropolis test.
    /// Positive delta = improvement, always accepted. Negative delta =
    /// worsening, accepted with probability `exp(delta / T)`.
    fn improvement(&self) -> i64 {
        match self {
            Self::Energy(d) => *d,
            Self::Score(d) => -*d,
        }
    }
}

/// Metropolis acceptance test. `delta` is a [`Delta`], `temperature` is
/// the current schedule value, and `rng` provides the uniform draw.
///
/// Returns `true` when the move should be accepted. At `temperature <= 0`
/// the function collapses to a strict improvement test — no uphill moves
/// are ever accepted — which makes the "frozen" tail of the schedule
/// behave like hill climbing.
#[must_use]
pub fn accept(delta: Delta, temperature: f64, rng: &mut NeighborhoodRng) -> bool {
    let improvement = delta.improvement();
    if improvement >= 0 {
        return true;
    }
    if temperature <= f64::EPSILON {
        return false;
    }
    // exp(delta/T): delta is negative, T is positive, ratio is negative,
    // result in (0, 1). Use an explicit cast because `f64::from(i64)` is
    // not implemented; the score delta is bounded by `score_delta` units.
    let ratio = f64::from(i32::try_from(improvement).unwrap_or(i32::MIN)) / temperature;
    let p = ratio.exp();
    rng.next_unit_f64() < p
}

/// Convenience helper: same as [`accept`] with a score delta.
#[must_use]
pub fn accept_score(score_delta: i64, temperature: f64, rng: &mut NeighborhoodRng) -> bool {
    accept(Delta::Score(score_delta), temperature, rng)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> NeighborhoodRng {
        NeighborhoodRng::from_seed(1)
    }

    #[test]
    fn schedule_decreases_monotonically() {
        let s = Schedule::new(100.0, 1.0, 100);
        let mut prev = s.t_init;
        for i in 1..=s.max_iter {
            let t = s.temperature(i);
            assert!(t <= prev + 1e-9, "non-monotonic at iter {i}: {t} > {prev}");
            prev = t;
        }
        assert!((s.temperature(s.max_iter) - s.t_min).abs() < 1e-6);
    }

    #[test]
    fn schedule_clamps_at_min() {
        let s = Schedule::new(50.0, 2.0, 10);
        // Past the budget we should never go below t_min.
        assert!(s.temperature(100) >= s.t_min - 1e-9);
    }

    #[test]
    fn schedule_alpha_zero_when_t_min_zero() {
        let s = Schedule::new(10.0, 0.0, 50);
        // alpha = (0/10)^(1/50) = 0; temperature drops to 0 immediately.
        assert!(s.temperature(50) >= 0.0);
    }

    #[test]
    fn accept_always_true_for_improvement() {
        let mut r = rng();
        for _ in 0..20 {
            assert!(accept(Delta::Score(-100), 1.0, &mut r));
        }
    }

    #[test]
    fn accept_rejects_worsening_at_zero_temperature() {
        let mut r = rng();
        for _ in 0..20 {
            assert!(!accept(Delta::Score(50), 0.0, &mut r));
        }
    }

    #[test]
    fn accept_accepts_some_worsening_at_high_temperature() {
        let mut r = rng();
        let mut accepted = 0u32;
        for _ in 0..200 {
            if accept(Delta::Score(1), 1000.0, &mut r) {
                accepted += 1;
            }
        }
        // At very high T, nearly every uphill move is accepted.
        assert!(accepted > 150, "expected most moves accepted, got {accepted}/200");
    }

    #[test]
    fn accept_accepts_few_worsening_at_low_temperature() {
        let mut r = rng();
        let mut accepted = 0u32;
        for _ in 0..200 {
            if accept(Delta::Score(1), 0.1, &mut r) {
                accepted += 1;
            }
        }
        // At very low T, almost no uphill moves are accepted.
        assert!(accepted < 30, "expected few moves accepted, got {accepted}/200");
    }

    #[test]
    fn delta_inverts_sign_for_score_vs_energy() {
        let mut r = rng();
        // Same numerical value: score +1 means worse, energy +1 means better.
        assert!(accept(Delta::Energy(1), 0.0, &mut r));
        assert!(!accept(Delta::Score(1), 0.0, &mut r));
    }
}
