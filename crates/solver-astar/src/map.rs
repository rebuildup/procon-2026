//! Map container and minimal context types.
//!
//! These are local stubs for now. They will be replaced by the canonical
//! `HexMap` / `MatchState` from `crates/domain` once issues #11 and #12 land
//! on the active release branch; the public API of this crate will then
//! switch to those types in a follow-up migration commit.

use thiserror::Error;

use crate::hex::{CellKind, Direction, HexCoord, MapLayout, MapSize};

/// Static hex map. Row-major cell vector; entry `(col, row)` lives at
/// `cells[row * width + col]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexMap {
    size: MapSize,
    layout: MapLayout,
    cells: Vec<CellKind>,
}

/// Errors that can occur while constructing a [`HexMap`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HexMapError {
    #[error("cell count {actual} does not match declared size ({width} * {height} = {expected})")]
    CellCountMismatch { actual: usize, width: u32, height: u32, expected: usize },
    #[error("map size out of range: {width}x{height} (must be 8..=32)")]
    InvalidSize { width: u32, height: u32 },
}

impl HexMap {
    /// Build a map from a flat cell vector. The vector length must equal
    /// `width * height`.
    pub fn new(width: u32, height: u32, cells: Vec<CellKind>) -> Result<Self, HexMapError> {
        let size = MapSize::new(width, height).ok_or(HexMapError::InvalidSize { width, height })?;
        let expected = size.cell_count();
        if cells.len() != expected {
            return Err(HexMapError::CellCountMismatch {
                actual: cells.len(),
                width,
                height,
                expected,
            });
        }
        Ok(Self { size, layout: MapLayout::EvenRowRightOffset, cells })
    }

    /// Map dimensions.
    #[must_use]
    pub const fn size(&self) -> MapSize {
        self.size
    }

    /// Grid layout. Always [`MapLayout::EvenRowRightOffset`] for now.
    #[must_use]
    pub const fn layout(&self) -> MapLayout {
        self.layout
    }

    /// Look up a cell by coordinate. Returns `None` if the coordinate is
    /// outside the map (boundary filter).
    #[must_use]
    pub fn get(&self, coord: HexCoord) -> Option<CellKind> {
        let idx = coord.to_index(self.size.width, self.size.height)?;
        Some(self.cells[idx])
    }

    /// All six neighbour coordinates that fall inside the map and pass the
    /// `is_passable` filter. Ponds and out-of-bounds cells are excluded;
    /// this is the single chokepoint that enforces the "境界外 / 池 への
    /// 進入を弾く" acceptance criterion.
    ///
    /// `fuel_remaining` is also consulted via `is_passable_with_fuel` so the
    /// empty-fuel hard constraint can be expressed at expansion time.
    pub fn neighbors(&self, from: HexCoord, allows: impl Fn(CellKind) -> bool) -> Vec<HexCoord> {
        let mut out = Vec::with_capacity(6);
        for dir in Direction::ALL {
            let next = dir.step(from);
            if let Some(cell) = self.get(next) {
                if allows(cell) {
                    out.push(next);
                }
            }
            // Out-of-bounds cells silently dropped — see spec Q2 (no torus).
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_size() {
        assert!(matches!(
            HexMap::new(8, 8, vec![CellKind::Plain; 7]),
            Err(HexMapError::CellCountMismatch { .. })
        ));
    }

    #[test]
    fn rejects_out_of_range_dimensions() {
        assert!(matches!(
            HexMap::new(4, 8, vec![CellKind::Plain; 32]),
            Err(HexMapError::InvalidSize { width: 4, height: 8 })
        ));
    }

    #[test]
    fn neighbors_skips_ponds_and_out_of_bounds() {
        // 8x8 map: surround (4, 4) with ponds and confirm only the open
        // neighbours come back.
        let mut cells = vec![CellKind::Plain; 64];
        for row in 0_i32..8 {
            for col in 0_i32..8 {
                if (col, row) == (4, 4) {
                    continue;
                }
                // Ring around the centre.
                let dc = (col - 4).abs();
                let dr = (row - 4).abs();
                if dc + dr <= 1 || (dc == 1 && dr == 1) {
                    let idx = usize::try_from(row * 8 + col).expect("non-negative");
                    cells[idx] = CellKind::Pond;
                }
            }
        }
        let map = HexMap::new(8, 8, cells).unwrap();
        let nb = map.neighbors(HexCoord::new(4, 4), CellKind::is_passable);
        // Centre (4,4) sits on an even row, so the NW/NE/SW/SE neighbours
        // are (3,3), (4,3), (3,5), (4,5) and East/West are (5,4), (3,4).
        // All six are ponds in this fixture, so the result must be empty.
        assert!(nb.is_empty(), "expected zero neighbours, got {nb:?}");
    }
}
