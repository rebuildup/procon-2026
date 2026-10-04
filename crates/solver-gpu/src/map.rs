//! Minimal hex map types for the shortest-path solver skeleton.
//!
//! These types intentionally live here rather than in `crates/domain` so
//! the solver-gpu crate does not have to depend on domain while #18 is still
//! in flight. Once the canonical hex map lands in `domain`, the solver-gpu
//! crate will switch over via a dedicated migration ticket (see #34 follow-up).
//!
//! The map is rectangular: `rows × cols` cells, both bounded by
//! [`MAX_MAP_DIM`] = 32, giving [`MAX_CELLS`] = 1024. Hex coordinates use
//! axial (q, r); offset (col, row) mapping is `even-r` (see
//! [`axial_to_offset`]).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum rows / columns per map dimension. Set by the contest spec
/// (マップ最大 32×32 = 1024 セル, see `docs/contest/strategy.md §5.1`).
pub const MAX_MAP_DIM: u8 = 32;

/// Maximum number of cells per map. `MAX_MAP_DIM * MAX_MAP_DIM`.
pub const MAX_CELLS: usize = (MAX_MAP_DIM as usize) * (MAX_MAP_DIM as usize);

/// Axial hex coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexCoord {
    pub q: i32,
    pub r: i32,
}

impl HexCoord {
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// Six axial neighbors, clockwise from east (+q).
    pub const NEIGHBORS: [HexCoord; 6] = [
        HexCoord { q: 1, r: 0 },
        HexCoord { q: 1, r: -1 },
        HexCoord { q: 0, r: -1 },
        HexCoord { q: -1, r: 0 },
        HexCoord { q: -1, r: 1 },
        HexCoord { q: 0, r: 1 },
    ];

    #[must_use]
    pub fn neighbors(self) -> [HexCoord; 6] {
        let n = &Self::NEIGHBORS;
        [
            HexCoord::new(self.q + n[0].q, self.r + n[0].r),
            HexCoord::new(self.q + n[1].q, self.r + n[1].r),
            HexCoord::new(self.q + n[2].q, self.r + n[2].r),
            HexCoord::new(self.q + n[3].q, self.r + n[3].r),
            HexCoord::new(self.q + n[4].q, self.r + n[4].r),
            HexCoord::new(self.q + n[5].q, self.r + n[5].r),
        ]
    }
}

/// Map dimensions in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapSize {
    pub rows: u8,
    pub cols: u8,
}

impl MapSize {
    pub const MAX: MapSize = MapSize { rows: MAX_MAP_DIM, cols: MAX_MAP_DIM };

    pub const fn new(rows: u8, cols: u8) -> Result<Self, MapError> {
        if rows == 0 || cols == 0 {
            return Err(MapError::ZeroDimension);
        }
        if rows > MAX_MAP_DIM || cols > MAX_MAP_DIM {
            return Err(MapError::TooLarge { rows, cols });
        }
        Ok(Self { rows, cols })
    }

    pub const fn cells(self) -> usize {
        self.rows as usize * self.cols as usize
    }
}

/// Per-cell cost. `cost == 0` means impassable. The skeleton encodes costs
/// as `u8` to keep the CPU buffer compact; this caps per-step cost at 255,
/// which is more than enough for the contest weights (1, 2, 4 mentioned in
/// `docs/contest/strategy.md §4`). Pass costs > 0 only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cell {
    pub cost: u8,
}

impl Cell {
    pub const IMPASSABLE: Cell = Cell { cost: 0 };
    pub const fn new(cost: u8) -> Self {
        Self { cost }
    }
    pub const fn is_blocked(self) -> bool {
        self.cost == 0
    }
}

/// Error returned when a map fails structural validation.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MapError {
    #[error("map dimensions must be non-zero")]
    ZeroDimension,
    #[error("map dimensions exceed {max} (got {rows}x{cols})", max = MAX_MAP_DIM)]
    TooLarge { rows: u8, cols: u8 },
    #[error("cost grid length {got} does not match map size {expected}")]
    SizeMismatch { got: usize, expected: usize },
    #[error("hex coord {q},{r} is outside the map")]
    OutOfBounds { q: i32, r: i32 },
}

/// Hex grid map. Cells are stored in row-major order (`r * cols + c`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Map {
    pub size: MapSize,
    cells: Vec<Cell>,
}

impl Map {
    /// Build a map from a row-major cost grid. Each inner row must have
    /// exactly `cols` cells.
    pub fn from_costs(rows: &[Vec<u8>], cols: u8) -> Result<Self, MapError> {
        let r = u8::try_from(rows.len()).map_err(|_| MapError::TooLarge { rows: u8::MAX, cols })?;
        let size = MapSize::new(r, cols)?;
        let mut cells = Vec::with_capacity(size.cells());
        for row in rows {
            if row.len() != usize::from(size.cols) {
                return Err(MapError::SizeMismatch {
                    got: row.len(),
                    expected: usize::from(size.cols),
                });
            }
            cells.extend(row.iter().copied().map(Cell::new));
        }
        Ok(Self { size, cells })
    }

    /// Total number of cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Convert (col, row) to flat index, returning `None` when out of bounds.
    #[must_use]
    pub fn offset_index(&self, col: u8, row: u8) -> Option<usize> {
        if col >= self.size.cols || row >= self.size.rows {
            return None;
        }
        Some(usize::from(row) * usize::from(self.size.cols) + usize::from(col))
    }

    /// Convert axial [`HexCoord`] to a flat index, returning `None` when
    /// the coord is outside the rectangular map bounds.
    #[must_use]
    pub fn hex_index(&self, coord: HexCoord) -> Option<usize> {
        let (col, row) = axial_to_offset(coord, self.size)?;
        let col_u = u8::try_from(col).ok()?;
        let row_u = u8::try_from(row).ok()?;
        self.offset_index(col_u, row_u)
    }

    /// Look up a cell by axial coord.
    #[must_use]
    pub fn get(&self, coord: HexCoord) -> Option<&Cell> {
        self.hex_index(coord).map(|i| &self.cells[i])
    }

    /// Iterate all cells in row-major order.
    pub fn iter(&self) -> std::slice::Iter<'_, Cell> {
        self.cells.iter()
    }
}

/// `IntoIterator` impl lets callers use `for cell in &map` ergonomically.
impl<'a> IntoIterator for &'a Map {
    type Item = &'a Cell;
    type IntoIter = std::slice::Iter<'a, Cell>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Convert axial `HexCoord` to (col, row) using even-r offset coordinates.
/// Returns `None` when the resulting offset is outside the rectangular map.
#[must_use]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
pub fn axial_to_offset(coord: HexCoord, size: MapSize) -> Option<(i32, i32)> {
    let q = coord.q;
    let r = coord.r;
    if r < 0 || r >= i32::from(size.rows) {
        return None;
    }
    let parity = (r as u32) & 1;
    let col = q.wrapping_add((parity as i32).wrapping_mul(r / 2));
    if col < 0 || col >= i32::from(size.cols) {
        return None;
    }
    Some((col, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_size_validates_bounds() {
        assert!(MapSize::new(0, 5).is_err());
        assert!(MapSize::new(5, 0).is_err());
        assert!(MapSize::new(33, 5).is_err());
        assert!(MapSize::new(5, 33).is_err());
        assert!(MapSize::new(32, 32).is_ok());
    }

    #[test]
    fn hex_neighbors_six_distinct() {
        let c = HexCoord::new(0, 0);
        let n = c.neighbors();
        assert_eq!(n.len(), 6);
        for i in 0..6 {
            for j in (i + 1)..6 {
                assert_ne!(n[i], n[j]);
            }
        }
    }

    #[test]
    fn from_costs_rejects_size_mismatch() {
        let rows = vec![vec![1u8, 2, 3], vec![4, 5]];
        let err = Map::from_costs(&rows, 3).unwrap_err();
        assert!(matches!(err, MapError::SizeMismatch { .. }));
    }

    #[test]
    fn axial_offset_round_trip_within_bounds() {
        let size = MapSize::new(4, 5).unwrap();
        for r in 0..i32::from(size.rows) {
            for c in 0..i32::from(size.cols) {
                let axial = offset_to_axial(c, r, size);
                let back = axial_to_offset(axial, size).unwrap();
                assert_eq!(back, (c, r));
            }
        }
    }

    #[test]
    fn map_into_iter_matches_iter() {
        let m = Map::from_costs(&[vec![1u8, 2], vec![3u8, 4]], 2).unwrap();
        let from_iter: Vec<_> = m.iter().copied().collect();
        let into_iter: Vec<_> = (&m).into_iter().copied().collect();
        assert_eq!(from_iter, into_iter);
    }

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
    fn offset_to_axial(col: i32, row: i32, size: MapSize) -> HexCoord {
        let parity = (row as u32) & 1;
        let q = col.wrapping_sub((parity as i32).wrapping_mul(row / 2));
        let _ = size;
        HexCoord::new(q, row)
    }
}
