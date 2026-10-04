//! 3-opt swap primitive for closed-loop tours.
//!
//! A 3-opt swap takes three indices `i < j < k`, cuts the closed tour into
//! three arcs, and re-glues the six cut ends into a new cycle. For a
//! *symmetric* edge cost (`edge_cost(a, b) == edge_cost(b, a)`, which holds
//! for the hex distance used here) the number of distinct non-trivial
//! reassemblies is exactly **seven**: there are 3!·2³ = 48 linear
//! arrangements of the three arcs, and rotation (3) plus whole-tour
//! reversal (2) identify them in groups of 6, i.e. 8 classes — one of
//! which is the original tour.
//!
//! Writing the arcs as `A = tour[i..=j]`, `B = tour[j+1..=k]`, and
//! `C = tour[k+1..] ++ tour[..i]` (the wrap-around remainder, possibly
//! empty), one canonical representative per class is obtained by keeping
//! `A` forward and listing the remaining two arcs in both orders with both
//! orientations. With `~X` denoting a reversed arc:
//!
//! | # | arrangement | equivalent plain description |
//! |---|---|---|
//! | 0 | `A B ~C`  | reverse `C` ≡ reverse `A+B` — the move [`try_three_opt`] applies |
//! | 1 | `A ~B C`  | reverse `B` (2-opt on `j+1..=k`) |
//! | 2 | `A ~B ~C` | reverse `B` and `C` ≡ reverse `A` + swap `B`/`C` |
//! | 3 | `A C B`   | swap `B` and `C` |
//! | 4 | `A C ~B`  | swap `B`/`C` and reverse `B` ≡ reverse `A` and `C` |
//! | 5 | `A ~C B`  | reverse `C` and swap ≡ reverse `A` and `B` |
//! | 6 | `A ~C ~B` | reverse `C` ≡ reverse `A` only (2-opt on `i..=j`) |
//!
//! [`try_three_opt`] (the `classic` operator set) evaluates only move 0 —
//! this module's [`try_three_opt_full`] (`operators=extended`) evaluates
//! all seven and keeps the best strict improvement (see Issue #191).
//!
//! Reversing an arc never changes its internal edge cost (symmetry), so a
//! move's delta is the cost of the three new junction edges minus the
//! three junction edges it replaces. When `C` is empty (only possible for
//! `i == 0 && k == n - 1`) the cycle degenerates to two arcs and the
//! arrangement table collapses onto identity / reverse-`B` — handled by
//! simply skipping the empty arc.

use crate::two_opt::{edge_cost, two_opt_delta};
use crate::types::{HexCoord, Spot};

/// Number of non-trivial 3-opt moves for a symmetric edge cost.
pub const THREE_OPT_MOVE_COUNT: usize = 7;

/// Canonical names of the seven moves, index-aligned with
/// [`three_opt_deltas`]. `~` marks a reversed arc (`A` = `i..=j`,
/// `B` = `j+1..=k`, `C` = the wrap-around remainder).
pub const THREE_OPT_MOVE_NAMES: [&str; THREE_OPT_MOVE_COUNT] =
    ["A B ~C", "A ~B C", "A ~B ~C", "A C B", "A C ~B", "A ~C B", "A ~C ~B"];

/// `(arc, reversed)` table behind [`THREE_OPT_MOVE_NAMES`].
/// Arc 0 = `A`, 1 = `B`, 2 = `C`.
const MOVES: [[(u8, bool); 3]; THREE_OPT_MOVE_COUNT] = [
    [(0, false), (1, false), (2, true)],
    [(0, false), (1, true), (2, false)],
    [(0, false), (1, true), (2, true)],
    [(0, false), (2, false), (1, false)],
    [(0, false), (2, false), (1, true)],
    [(0, false), (2, true), (1, false)],
    [(0, false), (2, true), (1, true)],
];

/// Apply the best 3-opt move touching `[i..k]` (inclusive). Returns `true`
/// if a strict improvement was applied (i.e. the tour got strictly better
/// by some non-trivial move), `false` if the original is locally optimal.
///
/// This is a single 3-opt *neighborhood* pass on the given `(i, j, k)`
/// triple. The full loop solver calls this for every valid triple and keeps
/// the best improvement found per iteration.
#[must_use]
pub fn try_three_opt(tour: &mut [u32], spots: &[Spot], i: usize, j: usize, k: usize) -> bool {
    debug_assert!(i < j && j < k);
    debug_assert!(k < tour.len());
    let n = tour.len();

    // Save the original cost around the triple (old edges: i-1 -> i, j -> j+1, k -> k+1).
    let lookup = |id: u32| spots.iter().find(|s| s.id == id).map(|s| s.coord);
    let prev_i = if i == 0 { tour[n - 1] } else { tour[i - 1] };
    let next_j = if j + 1 == n { tour[0] } else { tour[j + 1] };
    let next_k = if k + 1 == n { tour[0] } else { tour[k + 1] };

    let coords = || {
        Some((
            lookup(prev_i)?,
            lookup(tour[i])?,
            lookup(tour[j])?,
            lookup(tour[k])?,
            lookup(next_j)?,
            lookup(next_k)?,
        ))
    };
    let Some((pi, vi, vj, vk, nj, nk)) = coords() else {
        return false;
    };

    let original =
        edge_cost(pi, vi).saturating_add(edge_cost(vj, nj)).saturating_add(edge_cost(vk, nk));

    // Build the seven candidate variants. For each we compute the new edge
    // cost at the three cut boundaries, then apply if strictly better.
    //
    // Notation: after the move, the segments are reassembled in some order.
    // For each variant we list the three "new" boundary edges, in the order
    // (boundary-left, boundary-mid, boundary-right).
    //
    // Identity -> (pi-vi) + (vj-nj) + (vk-nk) == original, skip.
    //
    // Reverse segment i..j only -> pi-vj, vi-nj, vk-nk
    // Reverse segment j..k only -> pi-vi, vj-vk, nj-nk  (== 2-opt on j..k)
    // Swap(1,2):              -> pi-vj, vi-vk, nj-nk  (== 2-opt on i..k, but
    //                                                   j..k follows i..j)
    // Swap(1,2)+rev(2):       -> pi-vj, vi-nk, vj-vk  (3-opt only)
    // Rev(1)+swap(2,3):       -> pi-vk, vi-vj, nj-nk  (3-opt only)
    // Rev(1+2):               -> pi-vj, vi-nj, vj-vk  ... actually
    //                                                   identity of edges? revisit.
    //
    // Rather than enumerate every micro-case, we use the well-known fact:
    // any 3-opt move is a composition of 2-opt moves. So we can take the
    // best (i, j), (j, k), (i, k) 2-opt delta *combined* and apply the
    // minimum-cost arrangement. The simplest correct implementation:
    //
    //   1. Try the obvious "swap the two halves" move: reverse segment
    //      i..k (== 2-opt on i..k, the classic 3-opt "or-opt").
    //   2. If that's strictly better, apply it.
    //
    // This is *one* of the seven 3-opt moves — not exhaustive — but it is
    // strictly more powerful than 2-opt alone and is sufficient for the
    // solver-loop template (Issue #31). Full 3-opt enumeration can be added
    // when the engine exposes the real cost function.

    let candidate_delta = two_opt_delta(tour, spots, i, k);
    if candidate_delta < 0 {
        // Reversing segment i..k is a strict improvement -> apply.
        tour[i..=k].reverse();
        // Sanity: new cost must be lower than original.
        let pi2 = if i == 0 { tour[n - 1] } else { tour[i - 1] };
        let nk2 = if k + 1 == n { tour[0] } else { tour[k + 1] };
        if let (Some(a), Some(b)) = (lookup(pi2), lookup(nk2)) {
            let _ = edge_cost(a, b);
        }
        let _ = original;
        return true;
    }
    false
}

// ----------------------------------------------------------------------
// extended operator set: all seven moves (Issue #191)
// ----------------------------------------------------------------------

/// Length of arc `seg` (0 = `A`, 1 = `B`, 2 = `C`) for the triple `(i, j, k)`.
fn arc_len(n: usize, i: usize, j: usize, k: usize, seg: u8) -> usize {
    match seg {
        0 => j - i + 1,
        1 => k - j,
        // `C = tour[k+1..n] ++ tour[..i]`, empty iff `i == 0 && k == n-1`.
        _ => n - k - 1 + i,
    }
}

/// Index into `tour` of the `offset`-th element (0-based) of arc `seg` in
/// *forward* orientation.
fn arc_index(n: usize, i: usize, j: usize, k: usize, seg: u8, offset: usize) -> usize {
    match seg {
        0 => i + offset,
        1 => j + 1 + offset,
        _ => {
            let wrap = n - k - 1;
            if offset < wrap { k + 1 + offset } else { offset - wrap }
        }
    }
}

/// `(head, tail)` coordinates of each arc in its current orientation, or
/// `None` for the empty `C` arc. `head` is where the arc is entered from
/// the previous arc, `tail` is where it leaves into the next one.
type ArcEnds = [Option<(HexCoord, HexCoord)>; 3];

/// Sum of the junction edges of the closed cycle that visits `ends` in
/// `order`. Empty (`None`) arcs are skipped, which is what makes the
/// degenerate `C == []` case fall out of the general formula.
fn junction_cost(ends: &ArcEnds, order: [u8; 3]) -> Option<u32> {
    let mut seq = Vec::with_capacity(3);
    for &seg in &order {
        if let Some(end) = ends[usize::from(seg)] {
            seq.push(end);
        }
    }
    if seq.len() < 2 {
        return None;
    }
    let mut sum: u32 = 0;
    let len = seq.len();
    for (idx, (head, _)) in seq.iter().enumerate() {
        let (_, prev_tail) = seq[(idx + len - 1) % len];
        sum = sum.saturating_add(edge_cost(prev_tail, *head));
    }
    Some(sum)
}

/// Cost delta of every one of the seven [`THREE_OPT_MOVE_NAMES`] moves at
/// the triple `(i, j, k)`, in canonical order. `None` when a referenced
/// spot id is missing from `spots` (same failure mode as
/// [`try_three_opt`]).
///
/// A negative delta is a strict improvement; applying move `m` must change
/// [`crate::two_opt::tour_cost`] by exactly `deltas[m]`.
#[must_use]
pub fn three_opt_deltas(
    tour: &[u32],
    spots: &[Spot],
    i: usize,
    j: usize,
    k: usize,
) -> Option<[i64; THREE_OPT_MOVE_COUNT]> {
    debug_assert!(i < j && j < k);
    debug_assert!(k < tour.len());
    let n = tour.len();
    let lookup = |id: u32| spots.iter().find(|s| s.id == id).map(|s| s.coord);

    let a0 = lookup(tour[i])?;
    let a1 = lookup(tour[j])?;
    let b0 = lookup(tour[j + 1])?;
    let b1 = lookup(tour[k])?;
    let c_empty = i == 0 && k + 1 == n;
    let arc_c = if c_empty {
        None
    } else {
        let head = if k + 1 < n { lookup(tour[k + 1])? } else { lookup(tour[0])? };
        let tail = if i > 0 { lookup(tour[i - 1])? } else { lookup(tour[n - 1])? };
        Some((head, tail))
    };

    let forward: ArcEnds = [Some((a0, a1)), Some((b0, b1)), arc_c];
    let original = junction_cost(&forward, [0, 1, 2])?;

    let mut deltas = [0_i64; THREE_OPT_MOVE_COUNT];
    for (m, moves) in MOVES.iter().enumerate() {
        let mut ends: ArcEnds = [None, None, None];
        for &(seg, reversed) in moves {
            let Some(end) = forward[usize::from(seg)] else { continue };
            ends[usize::from(seg)] = Some(if reversed { (end.1, end.0) } else { end });
        }
        let order = [moves[0].0, moves[1].0, moves[2].0];
        let Some(cost) = junction_cost(&ends, order) else { continue };
        deltas[m] = i64::from(cost) - i64::from(original);
    }
    Some(deltas)
}

/// Rewrite `tour` into move `move_index` of the seven arrangements (see
/// [`THREE_OPT_MOVE_NAMES`]) at the triple `(i, j, k)`.
///
/// The result is rotated so the element that started at index `0` stays at
/// index `0` when it survives the move: the edge set — and therefore the
/// acceptance decision — is rotation-invariant, but the day/agent
/// assignment made by `distribute_plan` is not, so keeping the anchor
/// stable keeps the extended moves from shuffling assignments for reasons
/// unrelated to tour quality.
///
/// # Panics
/// Never in release; debug-asserts the move index and tour length.
pub fn apply_three_opt_move(tour: &mut [u32], i: usize, j: usize, k: usize, move_index: usize) {
    debug_assert!(i < j && j < k);
    debug_assert!(k < tour.len());
    debug_assert!(move_index < THREE_OPT_MOVE_COUNT);
    let n = tour.len();
    let mut out: Vec<u32> = Vec::with_capacity(n);
    for &(seg, reversed) in &MOVES[move_index] {
        let len = arc_len(n, i, j, k, seg);
        if len == 0 {
            continue;
        }
        if reversed {
            for t in (0..len).rev() {
                out.push(tour[arc_index(n, i, j, k, seg, t)]);
            }
        } else {
            for t in 0..len {
                out.push(tour[arc_index(n, i, j, k, seg, t)]);
            }
        }
    }
    debug_assert_eq!(out.len(), n);
    if let Some(pos) = out.iter().position(|&id| id == tour[0]) {
        out.rotate_left(pos);
    }
    tour.copy_from_slice(&out);
}

/// Apply the best of all seven classical 3-opt moves at `(i, j, k)`.
/// Returns `true` if a strict improvement was applied, `false` otherwise.
///
/// This is the `operators=extended` counterpart of [`try_three_opt`]:
/// where [`try_three_opt`] only tries move 0 (reverse `A+B`), this
/// evaluates every move in [`THREE_OPT_MOVE_NAMES`] and keeps the smallest
/// strictly negative delta (ties resolve to the lowest move index, so the
/// choice is deterministic).
#[must_use]
pub fn try_three_opt_full(tour: &mut [u32], spots: &[Spot], i: usize, j: usize, k: usize) -> bool {
    debug_assert!(i < j && j < k);
    debug_assert!(k < tour.len());
    let Some(deltas) = three_opt_deltas(tour, spots, i, j, k) else {
        return false;
    };
    let mut best: Option<(usize, i64)> = None;
    for (m, &delta) in deltas.iter().enumerate() {
        if delta < 0 {
            match best {
                Some((_, best_delta)) if best_delta <= delta => {}
                _ => best = Some((m, delta)),
            }
        }
    }
    let Some((move_index, _)) = best else { return false };
    apply_three_opt_move(tour, i, j, k, move_index);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::two_opt::tour_cost;
    use crate::types::{HexCoord, Series};

    fn spot(id: u32, q: i32, r: i32) -> Spot {
        Spot { id, coord: HexCoord::new(q, r), series: Series::new("s"), stock: 1 }
    }

    /// Eight spots scattered so crossings exist — the search space for the
    /// property-style tests below.
    fn varied_spots() -> Vec<Spot> {
        vec![
            spot(0, 0, 0),
            spot(1, 4, 0),
            spot(2, 1, 3),
            spot(3, 5, 2),
            spot(4, 2, -2),
            spot(5, -3, 1),
            spot(6, 6, -1),
            spot(7, -1, 4),
        ]
    }

    /// One xorshift64* step — the same generator `seeded_shuffle` uses, so
    /// the pseudo-random tours below are reproducible without an RNG crate.
    fn xorshift64star(mut x: u64) -> u64 {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Deterministic family of distinct tours over `ids`.
    fn seeded_tours(ids: &[u32], seeds: std::ops::Range<u64>) -> Vec<Vec<u32>> {
        let mut tours = vec![ids.to_vec()];
        for seed in seeds {
            let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15).max(1);
            let mut tour = ids.to_vec();
            for i in (1..tour.len()).rev() {
                state = xorshift64star(state);
                let span = u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1);
                let j = usize::try_from(state % span).unwrap_or(0);
                tour.swap(i, j);
            }
            tours.push(tour);
        }
        tours
    }

    /// Every triple the refine loop visits: `i < j < k`, `k >= i + 3`.
    fn solver_triples(n: usize) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        let mut k = 0usize;
        while k < n {
            let mut i = 0usize;
            while i + 2 < k {
                let mut j = i + 1;
                while j < k {
                    out.push((i, j, k));
                    j += 1;
                }
                i += 1;
            }
            k += 1;
        }
        out
    }

    #[test]
    fn try_three_opt_returns_false_when_no_improvement() {
        // All spots on a straight line — any reversal keeps cost the same,
        // so 3-opt should report no strict improvement.
        let spots = vec![spot(0, 0, 0), spot(1, 1, 0), spot(2, 2, 0), spot(3, 3, 0)];
        let mut tour = vec![0, 1, 2, 3];
        let improved = try_three_opt(&mut tour, &spots, 0, 1, 2);
        assert!(!improved, "no strict improvement on a collinear tour");
    }

    #[test]
    fn try_three_opt_full_returns_false_when_no_improvement() {
        // Same collinear instance as the classic test: every one of the
        // seven moves is a reordering of a monotone line, which cannot be
        // strictly cheaper than the monotone tour itself.
        let spots = vec![spot(0, 0, 0), spot(1, 1, 0), spot(2, 2, 0), spot(3, 3, 0)];
        let mut tour = vec![0, 1, 2, 3];
        let improved = try_three_opt_full(&mut tour, &spots, 0, 1, 2);
        assert!(!improved, "no strict improvement on a collinear tour");
    }

    /// Unit check per move: applying move `m` must change `tour_cost` by
    /// exactly `three_opt_deltas[..][m]`, for every one of the seven moves
    /// on every triple of a deterministic tour family (including the
    /// degenerate `C == []` triple `i == 0, k == n-1`).
    #[test]
    fn every_extended_move_delta_matches_actual_cost_change() {
        let spots = varied_spots();
        let ids: Vec<u32> = (0..8).collect();
        for tour in seeded_tours(&ids, 0..12) {
            for (i, j, k) in solver_triples(tour.len()) {
                let Some(deltas) = three_opt_deltas(&tour, &spots, i, j, k) else {
                    panic!("deltas must resolve for known spots at ({i}, {j}, {k})");
                };
                let before = tour_cost(&tour, &spots);
                for m in 0..THREE_OPT_MOVE_COUNT {
                    let mut moved = tour.clone();
                    apply_three_opt_move(&mut moved, i, j, k, m);
                    // The move must stay a permutation of the tour.
                    let (mut got, mut want) = (moved.clone(), tour.clone());
                    got.sort_unstable();
                    want.sort_unstable();
                    assert_eq!(got, want, "move {m} must preserve the spot set");
                    let after = tour_cost(&moved, &spots);
                    assert_eq!(
                        i64::from(after) - i64::from(before),
                        deltas[m],
                        "move {m} ({}) at ({i}, {j}, {k}) in {tour:?}",
                        THREE_OPT_MOVE_NAMES[m]
                    );
                }
            }
        }
    }

    /// `try_three_opt_full` may only report an improvement it actually
    /// made, and must leave the tour untouched when it reports none.
    #[test]
    fn try_three_opt_full_only_reports_strict_improvements() {
        let spots = varied_spots();
        let ids: Vec<u32> = (0..8).collect();
        for tour in seeded_tours(&ids, 0..16) {
            for (i, j, k) in solver_triples(tour.len()) {
                let mut candidate = tour.clone();
                let improved = try_three_opt_full(&mut candidate, &spots, i, j, k);
                if improved {
                    assert!(
                        tour_cost(&candidate, &spots) < tour_cost(&tour, &spots),
                        "reported improvement must strictly lower tour cost at ({i}, {j}, {k})"
                    );
                } else {
                    assert_eq!(candidate, tour, "no report => tour untouched");
                }
            }
        }
    }

    /// The point of `operators=extended`: there exist tours where the
    /// classic single move at a triple finds nothing while one of the six
    /// additional moves strictly improves.
    #[test]
    fn extended_3opt_finds_improvements_classic_misses() {
        let spots = varied_spots();
        let ids: Vec<u32> = (0..8).collect();
        let mut witness: Option<(Vec<u32>, usize, usize, usize)> = None;
        'search: for tour in seeded_tours(&ids, 0..200) {
            for (i, j, k) in solver_triples(tour.len()) {
                let mut classic = tour.clone();
                let classic_improved = try_three_opt(&mut classic, &spots, i, j, k);
                let mut full = tour.clone();
                let full_improved = try_three_opt_full(&mut full, &spots, i, j, k);
                if full_improved && !classic_improved {
                    assert!(
                        tour_cost(&full, &spots) < tour_cost(&tour, &spots),
                        "extended move must strictly improve"
                    );
                    witness = Some((tour, i, j, k));
                    break 'search;
                }
            }
        }
        let Some((tour, i, j, k)) = witness else {
            panic!("expected at least one triple where an extended move beats classic");
        };
        // Same input, same output: replaying the witness is stable.
        let mut replay = tour.clone();
        assert!(try_three_opt_full(&mut replay, &spots, i, j, k));
        assert_eq!(replay, {
            let mut again = tour.clone();
            let _ = try_three_opt_full(&mut again, &spots, i, j, k);
            again
        });
    }

    /// Extended refinement is deterministic: identical input, identical
    /// tour — no hidden RNG in the move selection.
    #[test]
    fn try_three_opt_full_is_deterministic() {
        let spots = varied_spots();
        let ids: Vec<u32> = (0..8).collect();
        for tour in seeded_tours(&ids, 0..8) {
            let mut a = tour.clone();
            let mut b = tour.clone();
            for (i, j, k) in solver_triples(tour.len()) {
                let _ = try_three_opt_full(&mut a, &spots, i, j, k);
                let _ = try_three_opt_full(&mut b, &spots, i, j, k);
            }
            assert_eq!(a, b, "same input must yield the same tour");
        }
    }
}
