//! Or-opt: relocate a short segment of a closed tour to another position.
//!
//! An or-opt move cuts a contiguous run of `len` spots (`1 <= len <= 3`)
//! out of the tour and re-inserts the run — same internal order, not
//! reversed — between two other consecutive spots. Unlike 3-opt it does not
//! re-glue arcs with flipped orientation; it *moves* them.
//!
//! For a symmetric edge cost the internal edges of the moved segment cost
//! the same before and after, so a move's delta only involves the two
//! removed junction edges and the three new ones:
//!
//! ```text
//! before:  prev -> head .. tail -> next        x -> y
//! after:   prev -> next                         x -> head .. tail -> y
//!
//! delta = [d(prev, next) + d(x, head) + d(tail, y)]
//!       - [d(prev, head) + d(tail, next) + d(x, y)]
//! ```
//!
//! Only insertion gaps *outside* the segment are legal; gaps touching the
//! segment are no-ops (delta 0) and are skipped by [`gap_is_legal`].
//!
//! The pass is the `operators=extended` counterpart of the 3-opt pass: a
//! deterministic first-improvement scan in `(start, len, gap)` order that
//! applies every strictly improving move it meets (Issue #191). Every
//! application strictly lowers the tour cost, so the scan terminates.

use crate::two_opt::edge_cost;
use crate::types::{HexCoord, Spot};

/// Longest segment [`apply_or_opt_pass`] relocates (the `seg <= 3` bound
/// from Issue #191).
pub const OR_OPT_MAX_LEN: usize = 3;

fn coord_of(spots: &[Spot], id: u32) -> Option<HexCoord> {
    spots.iter().find(|s| s.id == id).map(|s| s.coord)
}

/// Is `gap` (the space *after* `tour[gap]`) a legal insertion point for a
/// segment starting at `start` of length `len`?
///
/// Legal gaps are all gaps of the cycle except the `len + 1` gaps that
/// touch the segment (`start - 1 ..= start + len - 1`, modular): inserting
/// there would put the segment back where it came from.
#[must_use]
pub fn gap_is_legal(n: usize, start: usize, len: usize, gap: usize) -> bool {
    if n == 0 || len == 0 || len >= n {
        return false;
    }
    // Distance from the gap *before* the segment (`start - 1`) to `gap`,
    // wrapping. `0 ..= len` are exactly the touching gaps.
    let offset = (gap + 1 + n - (start % n)) % n;
    offset > len
}

/// Cost delta of removing `len` spots starting at `start` (indices wrap
/// modulo the tour length) and re-inserting them after `gap`.
///
/// Returns `0` for a degenerate request (`len == 0` or `len >= n`, i.e. a
/// segment covering the whole cycle). `gap` must satisfy
/// [`gap_is_legal`]; the value for an illegal gap is meaningless.
#[must_use]
pub fn or_opt_delta(tour: &[u32], spots: &[Spot], start: usize, len: usize, gap: usize) -> i64 {
    let n = tour.len();
    if n == 0 || len == 0 || len >= n {
        return 0;
    }
    let at = |t: usize| tour[t % n];
    let head = at(start);
    let tail = at(start + len - 1);
    let prev = at(start + n - 1);
    let next = at(start + len);
    let x = at(gap);
    let y = at(gap + 1);

    let (Some(prev_c), Some(head_c), Some(tail_c), Some(next_c), Some(x_c), Some(y_c)) = (
        coord_of(spots, prev),
        coord_of(spots, head),
        coord_of(spots, tail),
        coord_of(spots, next),
        coord_of(spots, x),
        coord_of(spots, y),
    ) else {
        return 0;
    };

    let old = u64::from(edge_cost(prev_c, head_c))
        .saturating_add(u64::from(edge_cost(tail_c, next_c)))
        .saturating_add(u64::from(edge_cost(x_c, y_c)));
    let new = u64::from(edge_cost(prev_c, next_c))
        .saturating_add(u64::from(edge_cost(x_c, head_c)))
        .saturating_add(u64::from(edge_cost(tail_c, y_c)));
    i64::try_from(new).unwrap_or(i64::MAX) - i64::try_from(old).unwrap_or(i64::MAX)
}

/// Apply the or-opt move described by [`or_opt_delta`]: cut the `len` spots
/// starting at `start` and re-insert them after `gap`.
///
/// Like [`crate::three_opt::apply_three_opt_move`], the result is rotated
/// so the element that started at index `0` stays at index `0` when it
/// survives the move — the edge set is rotation-invariant, the day/agent
/// assignment is not.
///
/// # Panics
/// Never in release; debug-asserts the tour length bookkeeping.
pub fn apply_or_opt(tour: &mut [u32], start: usize, len: usize, gap: usize) {
    let n = tour.len();
    debug_assert!(len > 0 && len < n);
    debug_assert!(gap_is_legal(n, start, len, gap));
    let mut segment = Vec::with_capacity(len);
    for t in 0..len {
        segment.push(tour[(start + t) % n]);
    }
    let anchor = tour[0];
    let mut out: Vec<u32> = Vec::with_capacity(n);
    for (idx, &id) in tour.iter().enumerate() {
        let rel = (idx + n - (start % n)) % n;
        if rel < len {
            continue;
        }
        out.push(id);
        if idx == gap {
            out.extend(segment.iter().copied());
        }
    }
    debug_assert_eq!(out.len(), n);
    if let Some(pos) = out.iter().position(|&id| id == anchor) {
        out.rotate_left(pos);
    }
    tour.copy_from_slice(&out);
}

/// First-improvement or-opt pass over the whole tour: for every
/// `(start, len, gap)` in ascending order, apply the move when its delta is
/// strictly negative.
///
/// Returns the number of moves applied (0 = locally optimal w.r.t. or-opt).
/// Later candidates see the tour as earlier candidates left it, which is
/// what keeps the pass deterministic without a best-move buffer.
#[must_use]
pub fn apply_or_opt_pass(tour: &mut [u32], spots: &[Spot]) -> u32 {
    let n = tour.len();
    let max_len = OR_OPT_MAX_LEN.min(n.saturating_sub(1));
    let mut applied: u32 = 0;
    for start in 0..n {
        for len in 1..=max_len {
            for gap in 0..n {
                if !gap_is_legal(n, start, len, gap) {
                    continue;
                }
                if or_opt_delta(tour, spots, start, len, gap) < 0 {
                    apply_or_opt(tour, start, len, gap);
                    applied = applied.saturating_add(1);
                }
            }
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::two_opt::tour_cost;
    use crate::types::{HexCoord, Series};

    fn spot(id: u32, q: i32, r: i32) -> Spot {
        Spot { id, coord: HexCoord::new(q, r), series: Series::new("s"), stock: 1 }
    }

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

    #[test]
    fn gap_is_legal_excludes_only_touching_gaps() {
        // Segment [2, 4): touching gaps are 1 (before), 2, 3 (inside).
        assert!(!gap_is_legal(8, 2, 2, 1));
        assert!(!gap_is_legal(8, 2, 2, 2));
        assert!(!gap_is_legal(8, 2, 2, 3));
        assert!(gap_is_legal(8, 2, 2, 0));
        assert!(gap_is_legal(8, 2, 2, 4));
        assert!(gap_is_legal(8, 2, 2, 7));
        // Wrapping segment [7, 2) covers 7, 0: touching 6, 7, 0.
        assert!(!gap_is_legal(8, 7, 2, 6));
        assert!(!gap_is_legal(8, 7, 2, 7));
        assert!(!gap_is_legal(8, 7, 2, 0));
        assert!(gap_is_legal(8, 7, 2, 1));
        // A segment may not cover the whole cycle.
        assert!(!gap_is_legal(4, 0, 4, 1));
        assert!(!gap_is_legal(4, 0, 0, 1));
    }

    /// Unit check for the delta formula: for every legal `(start, len,
    /// gap)` — one case per segment length 1, 2 and 3 — applying the move
    /// must change `tour_cost` by exactly `or_opt_delta`.
    #[test]
    fn or_opt_delta_matches_actual_cost_change() {
        let spots = varied_spots();
        let tours: Vec<Vec<u32>> =
            vec![(0..8).collect(), vec![0, 3, 1, 6, 2, 7, 4, 5], vec![7, 6, 5, 4, 3, 2, 1, 0]];
        for tour in tours {
            let n = tour.len();
            for start in 0..n {
                for len in 1..=OR_OPT_MAX_LEN {
                    if len >= n {
                        continue;
                    }
                    for gap in 0..n {
                        if !gap_is_legal(n, start, len, gap) {
                            continue;
                        }
                        let delta = or_opt_delta(&tour, &spots, start, len, gap);
                        let mut moved = tour.clone();
                        apply_or_opt(&mut moved, start, len, gap);
                        // Still a permutation of the tour.
                        let (mut got, mut want) = (moved.clone(), tour.clone());
                        got.sort_unstable();
                        want.sort_unstable();
                        assert_eq!(got, want, "or-opt must preserve the spot set");
                        let actual = i64::from(tour_cost(&moved, &spots))
                            - i64::from(tour_cost(&tour, &spots));
                        assert_eq!(
                            actual, delta,
                            "or-opt (start={start}, len={len}, gap={gap}) on {tour:?}"
                        );
                    }
                }
            }
        }
    }

    /// The single-spot case is the textbook or-opt: pulling a spot out of a
    /// crossing position must strictly lower the tour cost.
    #[test]
    fn or_opt_relocates_a_single_spot_to_remove_a_crossing() {
        let spots = vec![spot(0, 0, 0), spot(1, 6, 0), spot(2, 1, 1), spot(3, 5, 1), spot(4, 3, 4)];
        // Tour runs 0 -> 1 -> 2 -> 3 -> 4: spot 2 and 3 are visited in the
        // middle of the far side, so relocating one of them shortens the
        // run (exact best spot found by the scan below).
        let tour = vec![0, 1, 2, 3, 4];
        let mut improved = tour.clone();
        let applied = apply_or_opt_pass(&mut improved, &spots);
        assert!(applied >= 1, "expected at least one improving relocation");
        assert!(
            tour_cost(&improved, &spots) < tour_cost(&tour, &spots),
            "or-opt pass must strictly lower tour cost"
        );
    }

    /// The pass never increases cost and is reproducible.
    #[test]
    fn or_opt_pass_is_monotone_and_deterministic() {
        let spots = varied_spots();
        for tour in [
            (0..8).collect::<Vec<u32>>(),
            vec![4, 0, 7, 2, 5, 1, 6, 3],
            vec![0, 1, 2, 3, 4, 5, 6, 7],
        ] {
            let before = tour_cost(&tour, &spots);
            let mut a = tour.clone();
            let mut b = tour.clone();
            let applied_a = apply_or_opt_pass(&mut a, &spots);
            let applied_b = apply_or_opt_pass(&mut b, &spots);
            assert_eq!(a, b, "same input must yield the same tour");
            assert_eq!(applied_a, applied_b, "same input must yield the same move count");
            let after = tour_cost(&a, &spots);
            assert!(after <= before, "or-opt pass must never raise cost: {before} -> {after}");
        }
    }

    /// The Issue #191 bound: the pass only ever relocates segments of
    /// length `1..=3`. Each of those three lengths is exercised by
    /// `or_opt_delta_matches_actual_cost_change`; here we pin the bound
    /// itself and check the pass leaves a valid permutation behind.
    #[test]
    fn pass_is_capped_at_three_spots_and_preserves_the_tour() {
        assert_eq!(OR_OPT_MAX_LEN, 3);
        let spots = varied_spots();
        let mut tour: Vec<u32> = vec![4, 0, 7, 2, 5, 1, 6, 3];
        let _ = apply_or_opt_pass(&mut tour, &spots);
        let mut sorted = tour.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..8).collect::<Vec<u32>>());
    }
}
