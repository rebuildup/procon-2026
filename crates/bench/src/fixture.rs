//! Known-map fixture loader for the bench harness (Issue #59).
//!
//! A *fixture* is a JSON file matching the official Procon 37
//! [`protocol::match_config::MatchSetting`] shape, stored under
//! [`FIXTURES_DIR`]. Each fixture carries a small metadata header
//! (`name`, `description`, `category`) so the bench runner can present
//! the map set in solver reports without re-deriving labels.
//!
//! Acceptance criteria (Issue #59) call for a small / medium / large set
//! plus road-heavy, mountain-heavy, series-many, and series-few shapes.
//! All fixtures live in [`FIXTURES_DIR`] / [`bundled_fixtures`] and are
//! validated by [`MatchFixture::validate`].

use std::path::{Path, PathBuf};

use protocol::match_config::{CellType, MatchSetting, Spot};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hard-coded bundled fixture directory, resolved relative to the crate's
/// manifest.
pub const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/maps");

/// Result alias for fixture operations.
pub type FixtureResult<T> = Result<T, FixtureError>;

/// Failure modes for [`MatchFixture`] loading and validation.
#[derive(Debug, Error)]
pub enum FixtureError {
    /// Underlying I/O failure while reading a fixture file.
    #[error("io error reading {path}: {source}")]
    Io {
        /// Path that was being read.
        path: PathBuf,
        /// Original I/O error.
        #[source]
        source: std::io::Error,
    },
    /// JSON decode failure.
    #[error("decode error in {path}: {source}")]
    Decode {
        /// Path that was being decoded.
        path: PathBuf,
        /// Original serde error.
        #[source]
        source: serde_json::Error,
    },
    /// Structural invariant violation.
    #[error("invalid fixture {name}: {reason}")]
    Invalid {
        /// Fixture name (`name` field).
        name: String,
        /// Why the fixture is invalid.
        reason: String,
    },
}

/// Categorical bucket a fixture belongs to. The bench runner uses the
/// category to pick which fixtures to run per solver scenario
/// (warmup / integration / property).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureCategory {
    /// Official Procon 37 sample taken from `docs/pdf/.../試合設定用JSONファイル`.
    OfficialSample,
    /// 8x8 — small, exercises every code path with minimal cost.
    Small,
    /// 16x16 — medium, the typical contest map.
    Medium,
    /// 32x32 — large, stress-tests memory and search.
    Large,
    /// Many road tiles — solvers that special-case road cost will dominate.
    RoadHeavy,
    /// Many mountain tiles — solvers that special-case terrain will dominate.
    MountainHeavy,
    /// Many distinct brands — variety-first solvers will dominate.
    SeriesMany,
    /// Few brands (typically one) — total-noodles solvers will dominate.
    SeriesFew,
}

/// Top-level known-map fixture.
///
/// Wraps the official [`MatchSetting`] with a small metadata header used by
/// the bench runner. The metadata fields are part of the JSON format so they
/// round-trip cleanly through `serde_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchFixture {
    /// Short human-readable name (matches the file stem).
    pub name: String,
    /// One-line description used in solver reports.
    #[serde(default)]
    pub description: String,
    /// Which acceptance-criteria bucket this fixture belongs to.
    pub category: FixtureCategory,
    /// The official Procon 37 match setting.
    #[serde(flatten)]
    pub setting: MatchSetting,
}

impl MatchFixture {
    /// Construct a fixture from explicit metadata + setting.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        category: FixtureCategory,
        setting: MatchSetting,
    ) -> Self {
        Self { name: name.into(), description: description.into(), category, setting }
    }

    /// Map width.
    #[must_use]
    pub fn width(&self) -> i32 {
        self.setting.map.width
    }

    /// Map height.
    #[must_use]
    pub fn height(&self) -> i32 {
        self.setting.map.height
    }

    /// Total cell count (`width * height`).
    #[must_use]
    pub fn cell_count(&self) -> i64 {
        i64::from(self.setting.map.width) * i64::from(self.setting.map.height)
    }

    /// Number of distinct series (`brand`s) actually referenced by spots.
    #[must_use]
    pub fn series_count(&self) -> usize {
        let mut brands = std::collections::BTreeSet::new();
        for s in &self.setting.spots {
            brands.insert(s.brand);
        }
        brands.len()
    }

    /// Count cells of a given terrain type.
    #[must_use]
    pub fn cell_type_count(&self, want: CellType) -> usize {
        self.setting.map.cells.iter().flat_map(|row| row.iter()).filter(|c| **c == want).count()
    }

    /// Validate structural invariants of this fixture.
    ///
    /// # Errors
    /// Returns [`FixtureError::Invalid`] with a descriptive message when a
    /// required invariant is violated.
    pub fn validate(&self) -> FixtureResult<()> {
        self.validate_dimensions()?;
        self.validate_counts()?;
        self.validate_cells_shape()?;
        self.validate_positions()?;
        self.validate_category()?;
        Ok(())
    }

    fn validate_dimensions(&self) -> FixtureResult<()> {
        const MIN_DIM: i32 = 8;
        const MAX_DIM: i32 = 32;
        let w = self.width();
        let h = self.height();
        if !(MIN_DIM..=MAX_DIM).contains(&w) {
            return Err(FixtureError::Invalid {
                name: self.name.clone(),
                reason: format!("width {w} not in [{MIN_DIM}, {MAX_DIM}]"),
            });
        }
        if !(MIN_DIM..=MAX_DIM).contains(&h) {
            return Err(FixtureError::Invalid {
                name: self.name.clone(),
                reason: format!("height {h} not in [{MIN_DIM}, {MAX_DIM}]"),
            });
        }
        Ok(())
    }

    fn validate_counts(&self) -> FixtureResult<()> {
        const MIN_AGENTS: usize = 3;
        const MAX_AGENTS: usize = 8;
        const MIN_SPOTS: usize = 3;
        const MAX_SPOTS: usize = 32;

        if self.setting.agents.len() < MIN_AGENTS || self.setting.agents.len() > MAX_AGENTS {
            return Err(FixtureError::Invalid {
                name: self.name.clone(),
                reason: format!(
                    "agents count {} not in [{MIN_AGENTS}, {MAX_AGENTS}]",
                    self.setting.agents.len()
                ),
            });
        }
        if self.setting.spots.len() < MIN_SPOTS || self.setting.spots.len() > MAX_SPOTS {
            return Err(FixtureError::Invalid {
                name: self.name.clone(),
                reason: format!(
                    "spots count {} not in [{MIN_SPOTS}, {MAX_SPOTS}]",
                    self.setting.spots.len()
                ),
            });
        }
        Ok(())
    }

    fn validate_cells_shape(&self) -> FixtureResult<()> {
        // After `validate_dimensions`, width/height fit in u32.
        let w = u32::try_from(self.width()).expect("width fits in u32") as usize;
        let h = u32::try_from(self.height()).expect("height fits in u32") as usize;
        if self.setting.map.cells.len() != h {
            return Err(FixtureError::Invalid {
                name: self.name.clone(),
                reason: format!("cells row count {} != height {h}", self.setting.map.cells.len()),
            });
        }
        for (y, row) in self.setting.map.cells.iter().enumerate() {
            if row.len() != w {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("cells row {y} length {} != width {w}", row.len()),
                });
            }
        }
        Ok(())
    }

    fn validate_positions(&self) -> FixtureResult<()> {
        let cells = self.cell_count();
        let in_bounds = |p: i32| -> bool { p >= 0 && i64::from(p) < cells };

        let mut seen = std::collections::HashSet::new();
        for (i, pos) in self.setting.agents.iter().enumerate() {
            if !in_bounds(*pos) {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("agent #{i} pos {pos} out of bounds (cells={cells})"),
                });
            }
            if !seen.insert(*pos) {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("duplicate agent start pos {pos} at #{i}"),
                });
            }
        }

        let mut spot_pos = std::collections::HashSet::new();
        for (i, s) in self.setting.spots.iter().enumerate() {
            if !in_bounds(s.pos) {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("spot #{i} pos {} out of bounds (cells={cells})", s.pos),
                });
            }
            if s.stocks < 1 {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("spot #{i} stocks {} < 1", s.stocks),
                });
            }
            if !spot_pos.insert(s.pos) {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("duplicate spot pos {} at #{i}", s.pos),
                });
            }
        }

        // No agent may start on a spot (start cells must be clear).
        let spot_set: std::collections::HashSet<i32> =
            self.setting.spots.iter().map(|s| s.pos).collect();
        for (i, p) in self.setting.agents.iter().enumerate() {
            if spot_set.contains(p) {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("agent #{i} starts on spot {p}"),
                });
            }
        }

        // Every agent must start on passable terrain. The engine's
        // `Terrain::traversable` treats `Pond` as impassable, so a fixture
        // that plants agents on water cannot be evaluated at all — the match
        // setting is rejected and the case is dead. Catch it at load time
        // instead of letting a whole corpus column fail at evaluation time
        // (this is how the `road-heavy` fixture regressed: every one of its
        // five agents sat on a pond).
        // `validate_cells_shape` has already checked width/height are sane
        // and positive by the time we get here.
        let Some(w) = usize::try_from(self.setting.map.width).ok().filter(|v| *v > 0) else {
            return Ok(());
        };
        for (i, p) in self.setting.agents.iter().enumerate() {
            let Ok(idx) = usize::try_from(*p) else { continue };
            let cell =
                self.setting.map.cells.get(idx / w).and_then(|row| row.get(idx % w)).copied();
            if cell == Some(CellType::TYPE_2) {
                return Err(FixtureError::Invalid {
                    name: self.name.clone(),
                    reason: format!("agent #{i} starts on impassable terrain Pond ({p})"),
                });
            }
        }
        Ok(())
    }

    fn validate_category(&self) -> FixtureResult<()> {
        const MIN_BRANDS: usize = 3;
        match (self.category, self.series_count()) {
            (FixtureCategory::SeriesMany, n) if n < MIN_BRANDS => Err(FixtureError::Invalid {
                name: self.name.clone(),
                reason: format!("series_many fixture has only {n} brands (< {MIN_BRANDS})"),
            }),
            (FixtureCategory::SeriesFew, n) if n > 1 => Err(FixtureError::Invalid {
                name: self.name.clone(),
                reason: format!("series_few fixture has {n} brands (> 1)"),
            }),
            _ => Ok(()),
        }
    }

    /// Convenience: linear cell index `(y, x) -> y * width + x`.
    #[must_use]
    pub fn linear_index(&self, y: i32, x: i32) -> i32 {
        y * self.width() + x
    }
}

/// Loader entry point for known-map fixtures.
#[derive(Debug, Default, Clone, Copy)]
pub struct MatchFixtureLoader;

impl MatchFixtureLoader {
    /// Create a new loader.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Names of every fixture bundled in `crates/bench/fixtures/maps/`.
    ///
    /// The list is the canonical set the bench runner iterates over.
    #[must_use]
    pub fn bundled_fixtures() -> &'static [&'static str] {
        &[
            "openapi-sample",
            "small-8x8",
            "medium-16x16",
            "large-32x32",
            "road-heavy",
            "mountain-heavy",
            "series-many",
            "series-few",
        ]
    }

    /// Load a bundled fixture by short name (e.g. `"small-8x8"`).
    ///
    /// # Errors
    /// - [`FixtureError::Invalid`] when the short name is not in the bundled set.
    /// - I/O or decode errors propagate from [`Self::load_path`].
    pub fn load_bundled(self, name: &str) -> FixtureResult<MatchFixture> {
        if !Self::bundled_fixtures().contains(&name) {
            return Err(FixtureError::Invalid {
                name: name.to_owned(),
                reason: "not a bundled fixture".to_owned(),
            });
        }
        let mut path = PathBuf::from(FIXTURES_DIR);
        path.push(format!("{name}.json"));
        self.load_path(&path)
    }

    /// Load and validate every bundled fixture, in declaration order.
    ///
    /// # Errors
    /// Returns the first failing fixture's error.
    pub fn load_all_bundled(self) -> FixtureResult<Vec<MatchFixture>> {
        Self::bundled_fixtures().iter().map(|n| self.load_bundled(n)).collect()
    }

    /// Load and validate a fixture from a filesystem path.
    ///
    /// # Errors
    /// - [`FixtureError::Io`] if the file cannot be read.
    /// - [`FixtureError::Decode`] if the JSON is malformed.
    /// - [`FixtureError::Invalid`] from structural validation.
    #[allow(
        clippy::unused_self,
        reason = "API contract: keeps `loader.load_path(...)` callable; loader may gain state."
    )]
    pub fn load_path(self, path: &Path) -> FixtureResult<MatchFixture> {
        let raw = std::fs::read(path)
            .map_err(|source| FixtureError::Io { path: path.to_path_buf(), source })?;
        let fixture: MatchFixture = serde_json::from_slice(&raw)
            .map_err(|source| FixtureError::Decode { path: path.to_path_buf(), source })?;
        fixture.validate()?;
        Ok(fixture)
    }

    /// Parse and validate a fixture from an in-memory JSON slice.
    ///
    /// # Errors
    /// - [`FixtureError::Decode`] if the JSON is malformed.
    /// - [`FixtureError::Invalid`] from structural validation.
    #[allow(
        clippy::unused_self,
        reason = "API contract: keeps `loader.parse(...)` callable; loader may gain state."
    )]
    pub fn parse(self, raw: &[u8]) -> FixtureResult<MatchFixture> {
        let fixture: MatchFixture = serde_json::from_slice(raw)
            .map_err(|source| FixtureError::Decode { path: PathBuf::from("<memory>"), source })?;
        fixture.validate()?;
        Ok(fixture)
    }
}

/// Re-export of [`Spot`] for downstream solver code that wants a stable path.
pub use protocol::match_config::Spot as FixtureSpot;

#[allow(dead_code)]
fn _spot_alias_used(s: &Spot) -> i32 {
    s.brand
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fixtures_are_unique() {
        let names = MatchFixtureLoader::bundled_fixtures();
        let mut sorted: Vec<&str> = names.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "bundled fixture names must be unique");
    }

    #[test]
    fn loader_loads_every_bundled_fixture() {
        let loader = MatchFixtureLoader::new();
        let fixtures = loader.load_all_bundled().expect("all bundled fixtures must load");
        assert_eq!(fixtures.len(), MatchFixtureLoader::bundled_fixtures().len());
        for fx in &fixtures {
            // All fixtures share the contest-allowed bounds.
            assert!((8..=32).contains(&fx.width()));
            assert!((8..=32).contains(&fx.height()));
            assert!((3..=8).contains(&fx.setting.agents.len()));
            assert!((3..=32).contains(&fx.setting.spots.len()));
            assert_eq!(
                fx.setting.map.cells.len(),
                u32::try_from(fx.height()).expect("height fits in u32") as usize,
                "cells row count must equal height"
            );
        }
    }

    #[test]
    fn reject_undersized_map() {
        // width below 8 violates the contest constraint.
        let raw = serde_json::json!({
            "name": "too_small",
            "category": "small",
            "startsAt": 0,
            "daySeconds": [60],
            "daySteps": [10],
            "map": {
                "height": 8,
                "width": 7,
                "cells": vec![vec![0; 7]; 8]
            },
            "spots": [
                {"brand": 0, "pos": 0, "stocks": 1},
                {"brand": 0, "pos": 1, "stocks": 1},
                {"brand": 0, "pos": 2, "stocks": 1}
            ],
            "agents": [3, 4, 5],
            "fuelLimits": 10,
            "players": 1,
            "busyThreshold": 1,
            "jammedThreshold": 2
        });
        let loader = MatchFixtureLoader::new();
        let err = loader.parse(raw.to_string().as_bytes()).unwrap_err();
        assert!(matches!(err, FixtureError::Invalid { .. }));
    }

    #[test]
    fn reject_agent_on_spot() {
        let raw = serde_json::json!({
            "name": "overlap",
            "category": "small",
            "startsAt": 0,
            "daySeconds": [60],
            "daySteps": [10],
            "map": {
                "height": 8,
                "width": 8,
                "cells": vec![vec![0; 8]; 8]
            },
            "spots": [
                {"brand": 0, "pos": 5, "stocks": 1},
                {"brand": 0, "pos": 6, "stocks": 1},
                {"brand": 0, "pos": 7, "stocks": 1}
            ],
            "agents": [5, 9, 10],
            "fuelLimits": 10,
            "players": 1,
            "busyThreshold": 1,
            "jammedThreshold": 2
        });
        let loader = MatchFixtureLoader::new();
        let err = loader.parse(raw.to_string().as_bytes()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("starts on spot"), "unexpected error: {msg}");
    }

    /// An agent planted on water cannot move, so the case is dead on arrival.
    /// This is the exact regression the `road-heavy` fixture shipped with —
    /// all five agents sat on ponds and every evaluation of that fixture
    /// failed as `invalid`. Catch it at load time instead.
    #[test]
    fn reject_agent_on_impassable_terrain() {
        let mut cells = vec![vec![0_i32; 8]; 8];
        cells[2][3] = 2; // Pond
        let raw = serde_json::json!({
            "name": "ponded",
            "category": "small",
            "startsAt": 0,
            "daySeconds": [60],
            "daySteps": [10],
            "map": { "height": 8, "width": 8, "cells": cells },
            "spots": [
                {"brand": 0, "pos": 0, "stocks": 1},
                {"brand": 1, "pos": 1, "stocks": 1},
                {"brand": 2, "pos": 2, "stocks": 1}
            ],
            "agents": [19, 9, 10], // 19 = row 2, col 3 -> pond
            "fuelLimits": 10,
            "players": 1,
            "busyThreshold": 1,
            "jammedThreshold": 2
        });
        let loader = MatchFixtureLoader::new();
        let err = loader.parse(raw.to_string().as_bytes()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("impassable terrain Pond"), "unexpected error: {msg}");
    }

    /// Passing terrain: same fixture but with the agent on plain ground.
    #[test]
    fn accept_agent_on_passable_terrain() {
        let mut cells = vec![vec![0_i32; 8]; 8];
        cells[2][3] = 2; // Pond, but no agent stands on it
        cells[4][4] = 1; // Mountain is traversable
        let raw = serde_json::json!({
            "name": "ok-terrain",
            "category": "small",
            "startsAt": 0,
            "daySeconds": [60],
            "daySteps": [10],
            "map": { "height": 8, "width": 8, "cells": cells },
            "spots": [
                {"brand": 0, "pos": 0, "stocks": 1},
                {"brand": 1, "pos": 1, "stocks": 1},
                {"brand": 2, "pos": 2, "stocks": 1}
            ],
            "agents": [9, 10, 36], // plain, plain, mountain
            "fuelLimits": 10,
            "players": 1,
            "busyThreshold": 1,
            "jammedThreshold": 2
        });
        let loader = MatchFixtureLoader::new();
        loader.parse(raw.to_string().as_bytes()).expect("passable terrain is accepted");
    }

    #[test]
    fn reject_series_many_with_few_brands() {
        let raw = serde_json::json!({
            "name": "wrong_series",
            "category": "series_many",
            "startsAt": 0,
            "daySeconds": [60],
            "daySteps": [10],
            "map": {
                "height": 8,
                "width": 8,
                "cells": vec![vec![0; 8]; 8]
            },
            "spots": [
                {"brand": 0, "pos": 5, "stocks": 1},
                {"brand": 0, "pos": 6, "stocks": 1},
                {"brand": 0, "pos": 7, "stocks": 1}
            ],
            "agents": [1, 2, 3],
            "fuelLimits": 10,
            "players": 1,
            "busyThreshold": 1,
            "jammedThreshold": 2
        });
        let loader = MatchFixtureLoader::new();
        let err = loader.parse(raw.to_string().as_bytes()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("series_many fixture has only"), "unexpected error: {msg}");
    }
}
