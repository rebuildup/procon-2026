//! Hex grid coordinate primitives.
//!
//! Layout reference: 募集要項「移動と燃料」 + Q&A Q1 fixes an **even-row
//! right-offset** grid (sometimes called "even-r" in offset-coordinate
//! conventions). For a cell at `(col, row)`:
//!
//! - if `row` is even, the cell is rendered half a cell to the right of an
//!   odd-row cell at the same `col`,
//! - if `row` is odd, the cell sits directly under `(col, row - 1)`.
//!
//! **Known divergence:** [`Direction::step`] branches the *other* way round —
//! its even-row neighbour set is the engine's odd-row set, i.e. this crate
//! searches a column-mirrored twin of the contest graph. `crates/bench`
//! documents the mirror as load-bearing and un-mirrors every coordinate on
//! the way in and every direction on the way out
//! (`adapters/astar_walk::{mirror_coord, to_engine_dir}`), so engine-facing
//! paths are correct today. Flipping the parity here and dropping the bench
//! mirror must happen atomically, or paths come out double-mirrored — until
//! then treat these docs and tests as describing *this crate's* graph, not
//! the engine's.
//!
//! Direction order is clockwise from the upper-left neighbour
//! (`NorthWest, NorthEast, East, SouthEast, SouthWest, West`). That is the
//! order [`HexCoord::neighbors`] enumerates, which keeps greedy tie-breaking
//! deterministic across runs.

/// Hex grid coordinate. Offset coordinates, not axial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HexCoord {
    /// Zero-based column.
    pub col: i32,
    /// Zero-based row.
    pub row: i32,
}

impl HexCoord {
    /// Construct a coordinate from `(col, row)`.
    #[must_use]
    pub const fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }

    /// Convert to a linear index into a row-major `Vec` (`row * width + col`).
    ///
    /// Returns `None` if the resulting index would overflow `usize` or the
    /// coordinate falls outside `width * height` cells.
    #[must_use]
    pub fn to_index(self, width: u32, height: u32) -> Option<usize> {
        let col = u32::try_from(self.col).ok()?;
        let row = u32::try_from(self.row).ok()?;
        if col >= width || row >= height {
            return None;
        }
        Some((row as usize) * (width as usize) + (col as usize))
    }
}

/// Map dimensions. Matches issue #11's `MapSize` validate (8 <= W, H <= 32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapSize {
    pub width: u32,
    pub height: u32,
}

impl MapSize {
    /// Create a `MapSize`, returning `None` if either dimension is outside the
    /// contest-allowed range `[8, 32]`.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Option<Self> {
        if (8..=32).contains(&width) && (8..=32).contains(&height) {
            Some(Self { width, height })
        } else {
            None
        }
    }

    /// Total number of cells in the map.
    #[must_use]
    pub fn cell_count(self) -> usize {
        (self.width as usize) * (self.height as usize)
    }
}

/// One of the six neighbour directions, ordered clockwise from upper-left.
///
/// The `Wait` action is **not** a `Direction`; it is modelled separately at
/// the action layer because it never consumes fuel / steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    /// `(col + 1, row)`.
    East,
    /// One row up, to the right.
    NorthEast,
    /// One row up, to the left.
    NorthWest,
    /// `(col - 1, row)`.
    West,
    /// One row down, to the left.
    SouthWest,
    /// One row down, to the right.
    SouthEast,
}

impl Direction {
    /// All six directions in clockwise order starting from upper-left.
    ///
    /// The clockwise-from-upper-left ordering matches the spec wording and
    /// makes neighbour expansion deterministic.
    pub const ALL: [Self; 6] = [
        Self::NorthWest,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::SouthWest,
        Self::West,
    ];

    /// Compute the neighbour coordinate for `from` along `self`.
    ///
    /// Note the parity is **column-mirrored** relative to the engine's even-r
    /// table (`engine::model::Direction::step` swaps the two halves), so:
    /// - on even rows, the two upper neighbours sit at `(col, row - 1)` and
    ///   `(col - 1, row - 1)` — the engine's *odd*-row offsets,
    /// - on odd rows, they sit at `(col, row - 1)` and `(col + 1, row - 1)`.
    ///
    /// Symmetrically for the two lower neighbours. `crates/bench` mirrors
    /// coordinates to compensate; see the module docs.
    #[must_use]
    pub fn step(self, from: HexCoord) -> HexCoord {
        let (col, row) = (from.col, from.row);
        let even = row.rem_euclid(2) == 0;
        match self {
            Self::East => HexCoord::new(col + 1, row),
            Self::West => HexCoord::new(col - 1, row),
            Self::NorthEast => {
                if even {
                    HexCoord::new(col, row - 1)
                } else {
                    HexCoord::new(col + 1, row - 1)
                }
            }
            Self::NorthWest => {
                if even {
                    HexCoord::new(col - 1, row - 1)
                } else {
                    HexCoord::new(col, row - 1)
                }
            }
            Self::SouthEast => {
                if even {
                    HexCoord::new(col, row + 1)
                } else {
                    HexCoord::new(col + 1, row + 1)
                }
            }
            Self::SouthWest => {
                if even {
                    HexCoord::new(col - 1, row + 1)
                } else {
                    HexCoord::new(col, row + 1)
                }
            }
        }
    }
}

/// Grid layout. Currently only the even-row right-offset variant is exposed,
/// because Q&A Q1 fixes the layout for the contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapLayout {
    EvenRowRightOffset,
}

/// Cell kind. Issue #12 introduces the canonical `Cell` enum; this crate
/// keeps a local minimum-needed mirror until #12 lands on the active release
/// branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellKind {
    /// Passable. Step cost 1, fuel cost 1.
    Plain,
    /// Passable, slow. Step cost 2, fuel cost 2.
    Mountain,
    /// Impassable — [`HexMap::neighbors`] never returns ponds.
    Pond,
    /// Road cell. Step cost depends on traffic; fuel cost is always 2.
    Road {
        /// Current traffic state. Issue #12 exposes this as a separate
        /// `TrafficState` enum; here it lives inline because the solver only
        /// needs the three traffic values, not the wider domain vocabulary.
        traffic: TrafficState,
    },
}

impl CellKind {
    /// Whether a cell can be entered at all.
    #[must_use]
    pub fn is_passable(self) -> bool {
        !matches!(self, Self::Pond)
    }
}

/// Road traffic state.
///
/// Issue #12 names the same three states (`Smooth`, `Busy`, `Jammed`). Kept
/// inline here so the solver has no dependency on the in-flight domain
/// types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrafficState {
    /// Step cost 1.
    Smooth,
    /// Step cost 2.
    Busy,
    /// Step cost 4.
    Jammed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_row_neighbours_use_the_mirrored_parity() {
        // This crate's own graph: the even-row set is the engine's odd-row
        // set (column-mirrored). The bench adapter un-mirrors on the way out —
        // see the module docs before "fixing" this.
        let from = HexCoord::new(4, 4);
        assert_eq!(Direction::East.step(from), HexCoord::new(5, 4));
        assert_eq!(Direction::West.step(from), HexCoord::new(3, 4));
        // Row 4 is even.
        assert_eq!(Direction::NorthEast.step(from), HexCoord::new(4, 3));
        assert_eq!(Direction::NorthWest.step(from), HexCoord::new(3, 3));
        assert_eq!(Direction::SouthEast.step(from), HexCoord::new(4, 5));
        assert_eq!(Direction::SouthWest.step(from), HexCoord::new(3, 5));
    }

    #[test]
    fn odd_row_neighbours_use_the_mirrored_parity() {
        // Mirror half of the test above — same divergence, odd rows.
        let from = HexCoord::new(4, 5);
        assert_eq!(Direction::East.step(from), HexCoord::new(5, 5));
        assert_eq!(Direction::West.step(from), HexCoord::new(3, 5));
        // Row 5 is odd.
        assert_eq!(Direction::NorthEast.step(from), HexCoord::new(5, 4));
        assert_eq!(Direction::NorthWest.step(from), HexCoord::new(4, 4));
        assert_eq!(Direction::SouthEast.step(from), HexCoord::new(5, 6));
        assert_eq!(Direction::SouthWest.step(from), HexCoord::new(4, 6));
    }

    #[test]
    fn to_index_round_trip() {
        let size = MapSize::new(16, 16).unwrap();
        for row in 0_i32..16 {
            for col in 0_i32..16 {
                let coord = HexCoord::new(col, row);
                let idx = coord.to_index(size.width, size.height).unwrap();
                let expected = usize::try_from(row * 16 + col).expect("non-negative");
                assert_eq!(idx, expected);
            }
        }
    }

    #[test]
    fn to_index_rejects_out_of_bounds() {
        let size = MapSize::new(16, 16).unwrap();
        assert!(HexCoord::new(-1, 0).to_index(size.width, size.height).is_none());
        assert!(HexCoord::new(0, -1).to_index(size.width, size.height).is_none());
        assert!(HexCoord::new(16, 0).to_index(size.width, size.height).is_none());
        assert!(HexCoord::new(0, 16).to_index(size.width, size.height).is_none());
    }
}
