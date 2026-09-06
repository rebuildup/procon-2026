//! Neighbor operations on a route (ordered list of spot ids).
//!
//! The three operators form the candidate-move alphabet for the
//! local-search solver:
//!
//! * **swap** — exchange two positions in the route. Cheap, useful for
//!   re-ordering distant visits without changing adjacency.
//! * **2-opt** — reverse a contiguous segment. The classic TSP operator;
//!   fixes crossed edges in the implicit path graph.
//! * **or-opt** — relocate a contiguous segment of length 1..=`MAX_OR_OPT`
//!   to another position. Combines a small move with re-ordering.
//!
//! Each operator is exposed in two forms:
//!
//! 1. A pure `apply_*` helper that mutates a `&mut [u32]` (or `&mut Vec<u32>`
//!    for or-opt) in place and is easy to unit-test.
//! 2. A [`Move`] enum + [`propose`] / [`apply`] / [`inverse`] trio that the
//!    SA loop uses. `propose` picks a uniformly random legal move; the loop
//!    calls [`apply`] to commit, or [`inverse`] followed by [`apply`] to
//!    roll back.
//!
//! # or-opt semantics
//!
//! `Move::OrOpt { src, insert_at }` means:
//!
//! * `src` — the contiguous segment to relocate (indices in the *current*
//!   route).
//! * `insert_at` — the index in the *current* route where the segment
//!   should land. Must lie *outside* `[src.start, src.end]`; values inside
//!   the segment are degenerate no-ops and are rejected by [`apply_or_opt`].
//!
//! Application:
//!
//! 1. Drain `route[src.start..src.end]` (length = `len`).
//! 2. If `insert_at > src.end`, the target index in the shrunken array is
//!    `insert_at - len`. Otherwise it stays at `insert_at`.
//! 3. Splice the drained segment back in at the adjusted index.
//!
//! The [`inverse`] of an or-opt is another or-opt that puts the segment
//! back where it started. Because both forward and inverse use the same
//! `Move` shape, the SA loop never needs a separate "revert" code path —
//! it just calls [`apply`] with the inverse move.

// `usize -> u32` casts: route lengths come from spot counts and are
// bounded by the contest map size (32x32 = 1024 spots max), well within
// `u32`. The narrow loop arithmetic stays in `usize`.
#![allow(
    clippy::cast_possible_truncation,
    reason = "route lengths are bounded by map size and trivially fit in u32"
)]

use rand::{Rng as RandRng, RngCore, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

/// Maximum segment length considered by or-opt. 3 is the classical value
/// and matches the small step budget in our stub (`day_steps` ~ 60-200).
const MAX_OR_OPT: usize = 3;

/// Source of randomness for neighbor sampling. Wrapped so we can swap the
/// RNG family once the dependency footprint settles. Named
/// `NeighborhoodRng` to avoid clashing with the `rand::Rng` trait.
#[derive(Debug, Clone)]
pub struct NeighborhoodRng(Xoshiro256PlusPlus);

impl NeighborhoodRng {
    /// Deterministic constructor from a seed. Used by [`crate::solver`].
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        Self(Xoshiro256PlusPlus::seed_from_u64(seed))
    }

    /// Draw an unsigned integer in `[0, upper)`. Panics when `upper == 0`.
    pub(crate) fn next_u32(&mut self, upper: u32) -> u32 {
        self.0.gen_range(0..upper)
    }

    /// Draw a uniform `f64` in `[0, 1)`.
    pub(crate) fn next_unit_f64(&mut self) -> f64 {
        self.0.r#gen::<f64>()
    }
}

impl RngCore for NeighborhoodRng {
    fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.fill_bytes(dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.0.try_fill_bytes(dest)
    }
}

/// Pick one of [`Move::Swap`], [`Move::TwoOpt`], [`Move::OrOpt`] uniformly
/// at random. Returns `None` when the route is too short for any
/// operator to apply.
pub fn propose(route: &[u32], rng: &mut NeighborhoodRng) -> Option<Move> {
    if route.len() < 2 {
        return None;
    }
    // TwoOpt and OrOpt need at least 3 positions; swap only needs 2.
    // We bias toward 2-opt + or-opt when available because swap alone is a
    // weak operator, but we still keep swap in the mix for variety.
    let variant = rng.next_u32(3);
    match variant {
        0 => swap_move(route, rng),
        1 => {
            if route.len() < 3 {
                swap_move(route, rng)
            } else {
                two_opt_move(route, rng)
            }
        }
        _ => {
            if route.len() < 3 {
                swap_move(route, rng)
            } else {
                or_opt_move(route, rng)
            }
        }
    }
}

/// Apply a [`Move`] to `route` in place.
pub fn apply(route: &mut Vec<u32>, mv: Move) {
    match mv {
        Move::Swap { i, j } => apply_swap(route.as_mut_slice(), i, j),
        Move::TwoOpt { i, j } => apply_two_opt(route.as_mut_slice(), i, j),
        Move::OrOpt { src, insert_at } => apply_or_opt(route, src, insert_at),
    }
}

/// Invert a [`Move`] so the SA loop can roll back a rejected step cheaply.
/// All three operators are self-inverse in the sense that applying the
/// inverse twice returns the original route — see the per-variant
/// definitions for the exact math.
#[must_use]
pub fn inverse(mv: Move) -> Move {
    match mv {
        Move::Swap { i, j } => Move::Swap { i, j },
        Move::TwoOpt { i, j } => Move::TwoOpt { i, j },
        Move::OrOpt { src, insert_at } => {
            let len = src.len();
            // Post-forward location of the segment:
            let new_src_start = if insert_at > src.end { insert_at - len } else { insert_at };
            // Where to insert back: the pre-forward src.start is at
            // `src.start` if `insert_at > src.end` (no shift before it),
            // and at `src.start + len` if `insert_at < src.start` (the
            // segment was inserted earlier, pushing pre-forward
            // [insert_at, src.start) down by `len`).
            let new_insert_at = if insert_at > src.end { src.start } else { src.start + len };
            Move::OrOpt {
                src: Segment { start: new_src_start, end: new_src_start + len },
                insert_at: new_insert_at,
            }
        }
    }
}

/// A concrete neighbor, identified only by indices into the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Swap { i: usize, j: usize },
    TwoOpt { i: usize, j: usize },
    OrOpt { src: Segment, insert_at: usize },
}

/// A half-open segment `[start, end)` used by or-opt. `start <= end` and
/// `end <= route.len()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
}

impl Segment {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

// -------------------------------------------------------------------------
// swap
// -------------------------------------------------------------------------

/// Generate a uniformly random `Swap` move. `route.len() >= 2` is assumed
/// by the caller (see [`propose`]).
pub fn swap_move(route: &[u32], rng: &mut NeighborhoodRng) -> Option<Move> {
    if route.len() < 2 {
        return None;
    }
    let i = rng.next_u32(route.len() as u32) as usize;
    let mut j = rng.next_u32(route.len() as u32) as usize;
    // Reject degenerate self-swaps cheaply.
    if i == j {
        j = (j + 1) % route.len();
    }
    Some(Move::Swap { i, j })
}

pub fn apply_swap(route: &mut [u32], i: usize, j: usize) {
    debug_assert!(i < route.len() && j < route.len(), "swap indices out of range");
    if i == j {
        return;
    }
    route.swap(i, j);
}

// -------------------------------------------------------------------------
// 2-opt
// -------------------------------------------------------------------------

/// Generate a uniformly random `TwoOpt` move. `route.len() >= 3` is assumed.
pub fn two_opt_move(route: &[u32], rng: &mut NeighborhoodRng) -> Option<Move> {
    if route.len() < 3 {
        return None;
    }
    let n = route.len();
    // Pick `i` in [0, n-2] and `j` in [i+1, n-1]. Doing this in a single
    // `gen_range` keeps the distribution uniform.
    let i = rng.next_u32((n - 1) as u32) as usize;
    let j = i + 1 + rng.next_u32((n - 1 - i) as u32) as usize;
    Some(Move::TwoOpt { i, j })
}

pub fn apply_two_opt(route: &mut [u32], i: usize, j: usize) {
    debug_assert!(i < j && j < route.len(), "two_opt indices must satisfy i < j < n");
    if i >= j || j >= route.len() {
        return;
    }
    route[i..=j].reverse();
}

// -------------------------------------------------------------------------
// or-opt
// -------------------------------------------------------------------------

/// Generate a uniformly random `OrOpt` move. `route.len() >= 3` is assumed.
///
/// The distribution is uniform over (segment, `insert_at`) pairs with
/// `insert_at` strictly outside `[src.start, src.end]`.
pub fn or_opt_move(route: &[u32], rng: &mut NeighborhoodRng) -> Option<Move> {
    if route.len() < 3 {
        return None;
    }
    let n = route.len();
    let max_len = MAX_OR_OPT.min(n);
    let seg_len = 1 + rng.next_u32(max_len as u32) as usize;
    let start = rng.next_u32((n - seg_len + 1) as u32) as usize;
    let src = Segment { start, end: start + seg_len };
    // Number of valid insertion positions: n + 1 (0..=n) minus seg_len
    // positions inside the segment.
    let n_outside = n + 1 - seg_len;
    if n_outside == 0 {
        return None;
    }
    let choice = rng.next_u32(n_outside as u32) as usize;
    // Map `choice` (0..n_outside) to an actual `insert_at` by skipping
    // over the `seg_len` "blocked" positions inside `[src.start, src.end]`.
    let insert_at = if choice <= src.start { choice } else { choice + seg_len };
    debug_assert!(
        insert_at <= src.start || insert_at >= src.end,
        "or_opt generated an insert_at inside the segment"
    );
    Some(Move::OrOpt { src, insert_at })
}

/// Apply an or-opt move. Degenerate cases (`len == 0`, `insert_at` inside
/// `[src.start, src.end]`, out-of-range indices) become no-ops. The
/// `debug_assert!`s document the preconditions; release builds degrade
/// gracefully.
pub fn apply_or_opt(route: &mut Vec<u32>, src: Segment, insert_at: usize) {
    debug_assert!(
        src.end <= route.len() && insert_at <= route.len(),
        "or_opt indices must lie within the route"
    );
    if src.end > route.len() || insert_at > route.len() {
        return;
    }
    let len = src.len();
    if len == 0 {
        return;
    }
    // Reject insert_at inside the segment — those moves would land the
    // segment at the same place, which is a no-op and we cannot invert.
    if insert_at >= src.start && insert_at <= src.end {
        return;
    }
    let drained: Vec<u32> = route[src.start..src.end].to_vec();
    route.splice(src.start..src.end, std::iter::empty());
    let adj = if insert_at > src.end { insert_at - len } else { insert_at };
    // `splice(adj..adj, drained)` is an insertion (not an overwrite), so
    // the only constraint is `adj <= route.len()`. After splicing the
    // array grows back to `n`.
    debug_assert!(adj <= route.len(), "insert position out of range");
    route.splice(adj..adj, drained);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route() -> Vec<u32> {
        vec![1, 2, 3, 4, 5, 6]
    }

    #[test]
    fn swap_exchanges_two_positions() {
        let mut r = route();
        apply_swap(&mut r, 1, 4);
        assert_eq!(r, vec![1, 5, 3, 4, 2, 6]);
    }

    #[test]
    fn swap_is_no_op_on_same_index() {
        let mut r = route();
        apply_swap(&mut r, 2, 2);
        assert_eq!(r, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn two_opt_reverses_segment() {
        let mut r = route();
        apply_two_opt(&mut r, 1, 4);
        assert_eq!(r, vec![1, 5, 4, 3, 2, 6]);
    }

    #[test]
    fn two_opt_full_route_is_a_palindrome() {
        let mut r = vec![1, 2, 3, 4];
        apply_two_opt(&mut r, 0, 3);
        assert_eq!(r, vec![4, 3, 2, 1]);
    }

    #[test]
    fn or_opt_relocates_segment_to_later_position() {
        let mut r = route();
        // Take [1..3) = [2, 3], insert before position 5 in the *current*
        // route (i.e. before `6`).
        apply_or_opt(&mut r, Segment { start: 1, end: 3 }, 5);
        // After removal: [1, 4, 5, 6]. Insert at 5 - 2 = 3: [1, 4, 5, 2, 3, 6].
        assert_eq!(r, vec![1, 4, 5, 2, 3, 6]);
    }

    #[test]
    fn or_opt_relocates_segment_to_earlier_position() {
        let mut r = vec![10, 20, 30, 40, 50];
        // Take [3..5) = [40, 50], insert at position 0.
        apply_or_opt(&mut r, Segment { start: 3, end: 5 }, 0);
        assert_eq!(r, vec![40, 50, 10, 20, 30]);
    }

    #[test]
    fn or_opt_to_insert_at_start_works() {
        let mut r = vec![1, 2, 3, 4, 5];
        apply_or_opt(&mut r, Segment { start: 3, end: 5 }, 0);
        assert_eq!(r, vec![4, 5, 1, 2, 3]);
    }

    #[test]
    fn or_opt_noop_when_insert_at_inside_segment() {
        let mut r = route();
        apply_or_opt(&mut r, Segment { start: 1, end: 4 }, 2);
        assert_eq!(r, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn propose_returns_none_for_trivial_route() {
        let mut rng = NeighborhoodRng::from_seed(0);
        assert!(propose(&[], &mut rng).is_none());
        assert!(propose(&[1], &mut rng).is_none());
    }

    #[test]
    fn propose_then_apply_round_trips_via_inverse() {
        let mut rng = NeighborhoodRng::from_seed(42);
        for _ in 0..200 {
            let mut r = vec![1, 2, 3, 4, 5, 6, 7, 8];
            let original = r.clone();
            if let Some(mv) = propose(&r, &mut rng) {
                apply(&mut r, mv);
                apply(&mut r, inverse(mv));
                assert_eq!(r, original, "inverse must restore route: move = {mv:?}");
            }
        }
    }

    #[test]
    fn rng_distributes_moves_reasonably() {
        let mut rng = NeighborhoodRng::from_seed(7);
        let mut counts = [0u32; 3];
        for _ in 0..300 {
            if let Some(mv) = propose(&[1, 2, 3, 4, 5, 6, 7, 8], &mut rng) {
                match mv {
                    Move::Swap { .. } => counts[0] += 1,
                    Move::TwoOpt { .. } => counts[1] += 1,
                    Move::OrOpt { .. } => counts[2] += 1,
                }
            }
        }
        for c in counts {
            assert!(c > 30, "variant under-sampled: {counts:?}");
        }
    }

    #[test]
    fn or_opt_move_never_returns_insert_at_inside_segment() {
        let mut rng = NeighborhoodRng::from_seed(13);
        for _ in 0..200 {
            let r = vec![1, 2, 3, 4, 5, 6, 7, 8];
            if let Some(Move::OrOpt { src, insert_at }) = or_opt_move(&r, &mut rng) {
                assert!(
                    insert_at <= src.start || insert_at >= src.end,
                    "invalid or-opt: src={src:?} insert_at={insert_at}"
                );
            }
        }
    }
}
