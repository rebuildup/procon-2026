//! Smoke tests for the bundled match fixtures (Issue #59).
//!
//! These are integration tests so that loading-from-disk code paths are
//! exercised end-to-end (the unit tests in `fixture.rs` parse from
//! in-memory JSON only). Each fixture is checked for:
//!
//!   1. JSON parse against the `MatchFixture` schema.
//!   2. Structural invariants (size / agents / spots bounds).
//!   3. Self-consistency between `cells`, `agents`, and `spots`
//!      (every position is in-bounds and unique).
//!   4. Category-specific expectations (e.g. `series_many` has >= 3 brands).
//!
//! These tests are the regression gate: if anyone edits a fixture by hand
//! and breaks an invariant, the bench CI run fails immediately.

use std::collections::HashSet;

use bench::MatchFixture;
use bench::fixture::{FIXTURES_DIR, FixtureCategory, MatchFixtureLoader};

fn every_bundled_fixture() -> Vec<MatchFixture> {
    let loader = MatchFixtureLoader::new();
    loader.load_all_bundled().expect("all bundled fixtures must load")
}

#[test]
fn every_bundled_fixture_parses_and_validates() {
    let fixtures = every_bundled_fixture();
    assert_eq!(
        fixtures.len(),
        MatchFixtureLoader::bundled_fixtures().len(),
        "loader returned the wrong number of fixtures"
    );
}

#[test]
fn every_bundled_fixture_resides_under_fixture_dir() {
    let dir = std::path::Path::new(FIXTURES_DIR);
    assert!(dir.is_dir(), "{FIXTURES_DIR} should be a directory");
    for name in MatchFixtureLoader::bundled_fixtures() {
        let p = dir.join(format!("{name}.json"));
        assert!(p.exists(), "fixture file missing: {}", p.display());
    }
}

#[test]
fn every_bundled_fixture_meets_official_size_bounds() {
    for fx in every_bundled_fixture() {
        assert!((8..=32).contains(&fx.width()), "{} width out of bounds: {}", fx.name, fx.width());
        assert!(
            (8..=32).contains(&fx.height()),
            "{} height out of bounds: {}",
            fx.name,
            fx.height()
        );
        assert!(
            (3..=8).contains(&fx.setting.agents.len()),
            "{} agent count out of bounds: {}",
            fx.name,
            fx.setting.agents.len()
        );
        assert!(
            (3..=32).contains(&fx.setting.spots.len()),
            "{} spot count out of bounds: {}",
            fx.name,
            fx.setting.spots.len()
        );
    }
}

#[test]
fn every_bundled_fixture_cells_match_dimensions() {
    for fx in every_bundled_fixture() {
        let h = u32::try_from(fx.height()).expect("height fits in u32") as usize;
        let w = u32::try_from(fx.width()).expect("width fits in u32") as usize;
        assert_eq!(
            fx.setting.map.cells.len(),
            h,
            "{}: cells row count {} != height {h}",
            fx.name,
            fx.setting.map.cells.len()
        );
        for (y, row) in fx.setting.map.cells.iter().enumerate() {
            assert_eq!(row.len(), w, "{}: cells row {y} len {} != width {w}", fx.name, row.len());
        }
    }
}

#[test]
fn every_bundled_fixture_positions_are_in_bounds_and_unique() {
    let cell_count = |fx: &MatchFixture| -> i64 { fx.cell_count() };
    for fx in every_bundled_fixture() {
        let cells = cell_count(&fx);
        let mut agent_pos = HashSet::new();
        for (i, p) in fx.setting.agents.iter().enumerate() {
            assert!(
                *p >= 0 && i64::from(*p) < cells,
                "{}: agent #{i} pos {p} out of bounds",
                fx.name
            );
            assert!(agent_pos.insert(*p), "{}: duplicate agent pos {p} at #{i}", fx.name);
        }
        let mut spot_pos = HashSet::new();
        for (i, s) in fx.setting.spots.iter().enumerate() {
            assert!(
                s.pos >= 0 && i64::from(s.pos) < cells,
                "{}: spot #{i} pos {} out of bounds",
                fx.name,
                s.pos
            );
            assert!(s.stocks >= 1, "{}: spot #{i} stocks {} < 1", fx.name, s.stocks);
            assert!(spot_pos.insert(s.pos), "{}: duplicate spot pos {} at #{i}", fx.name, s.pos);
        }
        // No agent may start on a spot.
        for (i, p) in fx.setting.agents.iter().enumerate() {
            assert!(!spot_pos.contains(p), "{}: agent #{i} starts on spot {p}", fx.name);
        }
    }
}

#[test]
fn series_many_fixtures_have_at_least_three_brands() {
    for fx in every_bundled_fixture() {
        if matches!(fx.category, FixtureCategory::SeriesMany) {
            assert!(
                fx.series_count() >= 3,
                "{} (series_many) has only {} brands",
                fx.name,
                fx.series_count()
            );
        }
    }
}

#[test]
fn series_few_fixtures_have_at_most_one_brand() {
    for fx in every_bundled_fixture() {
        if matches!(fx.category, FixtureCategory::SeriesFew) {
            assert!(
                fx.series_count() <= 1,
                "{} (series_few) has {} brands",
                fx.name,
                fx.series_count()
            );
        }
    }
}

#[test]
fn road_heavy_fixtures_have_above_average_road_share() {
    for fx in every_bundled_fixture() {
        if matches!(fx.category, FixtureCategory::RoadHeavy) {
            // TYPE_1 = Road under the official table (Issue #226:
            // 0=平地, 1=道路, 2=山地, 3=池). (Before #187 this assertion read
            // `TYPE_2` — the *Pond* code under the pre-#226 table — so the
            // pre-#187 `road-heavy` — Road 4/256 — passed a "road share"
            // gate it never satisfied. Issue #187 fixed the cell code and
            // moved the same 15% floor into `validate_category`.)
            let road_cells = fx.cell_type_count(protocol::match_config::CellType::TYPE_1);
            let total_cells = u64::try_from(fx.cell_count()).expect("cell count is non-negative");
            assert!(
                u64::try_from(road_cells).expect("road cell count fits in u64") * 100
                    >= total_cells * 15,
                "{}: road-heavy share below 15% threshold",
                fx.name
            );
        }
    }
}

#[test]
fn mountain_heavy_fixtures_have_above_average_mountain_share() {
    for fx in every_bundled_fixture() {
        if matches!(fx.category, FixtureCategory::MountainHeavy) {
            let mountain_cells = fx.cell_type_count(protocol::match_config::CellType::TYPE_2);
            let total_cells = u64::try_from(fx.cell_count()).expect("cell count is non-negative");
            assert!(
                u64::try_from(mountain_cells).expect("mountain cell count fits in u64") * 100
                    >= total_cells * 50,
                "{}: mountain-heavy share below 50% threshold",
                fx.name
            );
        }
    }
}

#[test]
fn fixture_loader_rejects_unknown_bundled_name() {
    let loader = MatchFixtureLoader::new();
    let err = loader.load_bundled("definitely-not-a-real-fixture").unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("not a bundled fixture"), "unexpected error: {msg}");
}

#[test]
fn fixture_round_trips_through_json() {
    let loader = MatchFixtureLoader::new();
    let fx = loader.load_bundled("small-8x8").expect("fixture must load");
    let json = serde_json::to_string(&fx).expect("serialize");
    let back: MatchFixture = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(fx, back);
}

/// Issue #226 equivalence proof (machine-checked): every bundled fixture's
/// **terrain histogram** still equals the pre-#226 world.
///
/// The constants below were computed from `origin/release-0-2-0`'s fixture
/// files read through the *old* (permuted) table — i.e. the terrain these
/// maps were authored to contain. #226 re-coded every authored cell
/// (`0→0, 1→2, 2→3, 3→1`, `openapi-sample` included: its 8 code-`1` ridge
/// cells became code-`2` mountains), so raw codes changed while terrain did
/// not. A mismatch here means the permutation was applied inconsistently —
/// evidence would silently describe a different board.
///
/// Order per fixture: `[Plain, Mountain, Pond, Road]` (wire codes
/// `0, 2, 3, 1` under the official table).
#[test]
fn terrain_histograms_match_the_old_world_constants() {
    const EXPECTED: &[(&str, [u64; 4])] = &[
        ("small-8x8", [56, 8, 0, 0]),
        ("medium-16x16", [174, 22, 60, 0]),
        ("large-32x32", [853, 38, 124, 9]),
        ("road-heavy", [198, 0, 54, 4]),
        ("road-heavy-v2", [140, 15, 21, 80]),
        ("mountain-heavy", [52, 142, 60, 2]),
        ("series-many", [96, 4, 44, 0]),
        ("series-few", [86, 8, 44, 6]),
        ("openapi-sample", [56, 8, 0, 0]),
    ];

    let fixtures = every_bundled_fixture();
    assert_eq!(fixtures.len(), EXPECTED.len(), "every bundled fixture must be covered");
    for (name, want) in EXPECTED {
        let fx = fixtures
            .iter()
            .find(|f| f.name == *name)
            .unwrap_or_else(|| panic!("bundled fixture {name} missing"));
        let plain = fx.cell_type_count(protocol::match_config::CellType::TYPE_0);
        let road = fx.cell_type_count(protocol::match_config::CellType::TYPE_1);
        let mountain = fx.cell_type_count(protocol::match_config::CellType::TYPE_2);
        let pond = fx.cell_type_count(protocol::match_config::CellType::TYPE_3);
        let got = [
            plain as u64, mountain as u64, pond as u64, road as u64,
        ];
        assert_eq!(
            got, *want,
            "{name}: terrain histogram drifted from the pre-#226 world \
             (expected [Plain, Mountain, Pond, Road] = {want:?})"
        );
    }
}
