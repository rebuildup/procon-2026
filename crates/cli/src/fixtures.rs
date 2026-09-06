//! Local fixtures used exclusively by the warmup command.
//!
//! These are intentionally tiny stand-ins for the real contest solver crates
//! (greedy / A* / loop / MCTS), which have not yet been merged into the
//! `cli` workspace. The shapes mirror the eventual protocol so that once
//! the real solvers land, swapping `MiniSolver` for the production trait is
//! a mechanical change.
//!
//! Constraint: no dependency on the real solver crates. This module compiles
//! purely against `domain` / `engine` / `serde_json`.

use std::time::Duration;

use domain::id::{MatchId, PlayerId};
use serde::{Deserialize, Serialize};

/// Tile types in the warmup grid. Matches the eventual contest model closely
/// enough to exercise each solver's branch structure without requiring the
/// full map parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tile {
    Plain,
    Road,
    Mountain,
    Blocked,
}

impl Tile {
    /// Step cost for traversing this tile (the contest rule of thumb is
    /// `1` / `1.5` / `2` for plain / road / mountain). This is the same
    /// shorthand used by `docs/contest/strategy.md` so warmup numbers stay
    /// representative.
    #[must_use]
    pub const fn cost(self) -> u32 {
        match self {
            Tile::Plain | Tile::Road => 1,
            Tile::Mountain => 2,
            Tile::Blocked => u32::MAX,
        }
    }
}

/// Bounded grid (`rows x cols`) plus a list of `agents` and `spots`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmupSetting {
    pub match_id: MatchId,
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<Tile>, // row-major, length = width * height
    pub agents: Vec<PlayerId>,
    pub spots: Vec<(usize, usize)>,
    pub day_steps: u32,
}

impl WarmupSetting {
    /// Convenience: tile lookup with bounds check.
    ///
    /// # Panics
    /// Panics if `(row, col)` is out of bounds — warmup is internal and
    /// always uses coordinates it just produced, so this is acceptable.
    #[must_use]
    pub fn tile(&self, row: usize, col: usize) -> Tile {
        assert!(
            row < self.height && col < self.width,
            "tile({row},{col}) out of bounds for {}x{}",
            self.width,
            self.height
        );
        self.tiles[row * self.width + col]
    }

    /// Linear index into the row-major tile buffer.
    #[must_use]
    #[allow(dead_code)] // exercised by tests in this module
    pub const fn index(&self, row: usize, col: usize) -> usize {
        row * self.width + col
    }
}

/// One concrete action emitted by a solver. Kept generic until the real
/// action schema lands (Issue #14 / #22-#44).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub agent: PlayerId,
    pub step: u32,
    pub dr: i8,
    pub dc: i8,
}

/// Lightweight solver result. Real solvers will replace this with their own
/// richer output, but the timing surface (count + length) is what warmup
/// measures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveOutcome {
    pub actions: Vec<Action>,
    pub explored_nodes: usize,
}

/// Trait implemented by every mini solver. Mirrors the future
/// `engine::Solver` trait so the warmup plumbing stays reusable.
pub trait MiniSolver {
    fn name(&self) -> &'static str;

    /// Decide the next action for `agent_id` from `from` toward `to`. Pure:
    /// identical inputs must yield identical outputs.
    fn decide(
        &mut self,
        setting: &WarmupSetting,
        agent_id: &PlayerId,
        from: (usize, usize),
        to: (usize, usize),
    ) -> SolveOutcome;

    /// Optional pre-computation pass that the solver may use to amortise
    /// repeated work across calls (e.g. A* tables, MCTS caches). The
    /// warmup command times this separately as `cold_init` when the solver
    /// has never been warmed, and skips it when it has.
    fn prepare(&mut self, _setting: &WarmupSetting) {}
}

/// Default fixture: an 8x8 grid, two plain regions separated by a mountain
/// ridge, 3 agents and 4 spots. Tiny enough that cold + warm + smoke all
/// finish in a few milliseconds each, so warmup stays cheap in CI.
#[must_use]
pub fn tiny_setting() -> WarmupSetting {
    let width = 8;
    let height = 8;
    let mut tiles = vec![Tile::Plain; width * height];
    // Carve a north-south mountain ridge down column 4.
    for row in 0..height {
        tiles[row * width + 4] = Tile::Mountain;
    }
    // Place four spots deterministically: two on each side of the ridge.
    let spots = vec![(1, 1), (1, 6), (6, 1), (6, 6)];
    WarmupSetting {
        match_id: MatchId::new("warmup-tiny").expect("non-empty id"),
        width,
        height,
        tiles,
        agents: vec![
            PlayerId::new("agent-a").expect("non-empty"),
            PlayerId::new("agent-b").expect("non-empty"),
            PlayerId::new("agent-c").expect("non-empty"),
        ],
        spots,
        day_steps: 30,
    }
}

/// Number of timing samples per pass. Picked low enough to keep CI warm
/// under a second but high enough that p50/p90/max are meaningful for
/// `n >= 5`.
pub const SAMPLES_PER_PASS: usize = 10;

/// Number of warmup iterations before the warm pass is recorded. After this
/// many iterations the solver's internal caches (if any) should be hot.
pub const WARMUP_ITERATIONS: usize = 3;

/// Convenience: convert a `Duration` into microseconds as `u64`.
#[must_use]
pub fn micros(d: Duration) -> u64 {
    let nanos = d.as_nanos();
    u64::try_from(nanos / 1_000).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_setting_is_well_formed() {
        let s = tiny_setting();
        assert_eq!(s.width * s.height, s.tiles.len());
        assert_eq!(s.agents.len(), 3);
        assert_eq!(s.spots.len(), 4);
        // The ridge is `Mountain`, costing 2 steps. We also confirm a
        // `Blocked` sentinel can never accidentally appear in the default
        // fixture (the warmup must remain solvable).
        assert_eq!(s.tile(3, 4), Tile::Mountain);
        assert_eq!(s.tile(3, 4).cost(), 2);
        for t in &s.tiles {
            assert_ne!(*t, Tile::Blocked);
        }
    }

    #[test]
    fn index_round_trips() {
        let s = tiny_setting();
        for row in 0..s.height {
            for col in 0..s.width {
                let idx = s.index(row, col);
                assert_eq!(idx / s.width, row);
                assert_eq!(idx % s.width, col);
            }
        }
    }
}
