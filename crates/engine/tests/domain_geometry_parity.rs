//! Cross-crate geometry parity: `domain::action::neighbor` must agree with
//! `engine::model::Direction::step` for every coordinate and direction.
//!
//! `engine` is what actually drives a match, `domain` is the crate future
//! callers are pointed at — so the two must describe the same even-r graph.
//! `domain` once implemented the mirrored (odd-r) offsets while its own docs
//! claimed "even-row right-offset", and nothing caught it because the two
//! implementations were never compared.

use domain::{Direction as DomainDirection, HexCoord as DomainHexCoord, MapLayout, neighbor};
use engine::model::{Direction, HexCoord};

#[test]
fn domain_neighbor_matches_the_engine_step_table() {
    for row in 0..8 {
        for col in 0..8 {
            let engine_from = HexCoord::new(col, row);
            let domain_from = DomainHexCoord::new(
                u8::try_from(col).expect("in range"),
                u8::try_from(row).expect("in range"),
            );
            for dir in Direction::ALL {
                let engine_to = dir.step(engine_from);
                let domain_dir = match dir {
                    Direction::UpperLeft => DomainDirection::UpperLeft,
                    Direction::UpperRight => DomainDirection::UpperRight,
                    Direction::Right => DomainDirection::Right,
                    Direction::LowerRight => DomainDirection::LowerRight,
                    Direction::LowerLeft => DomainDirection::LowerLeft,
                    Direction::Left => DomainDirection::Left,
                };
                // `domain` stores `u8` and reports an off-map negative
                // neighbour as `None`; the engine computes the coordinate and
                // lets the caller bounds-check. Same graph, different error.
                let expected = (engine_to.col >= 0 && engine_to.row >= 0)
                    .then(|| HexCoord::new(engine_to.col, engine_to.row));
                let domain_to = neighbor(domain_from, domain_dir, MapLayout::default())
                    .map(|c| HexCoord::new(i32::from(c.col), i32::from(c.row)));
                assert_eq!(
                    domain_to, expected,
                    "({col},{row}) {dir:?}: domain and engine disagree"
                );
            }
        }
    }
}
