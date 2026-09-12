# Known-map fixtures (Issue #59)

This directory bundles JSON fixtures for the `bench` crate's solver-evaluation
runner. Each fixture matches the official Procon 37 `MatchSetting` shape
(`crates/protocol/src/match_config.rs`) wrapped in a small metadata header
(`name`, `description`, `category`) so the runner can pick fixtures by
characteristic without re-deriving labels.

## Layout

```
crates/bench/fixtures/
├── README.md                  # this file
├── example_match.json         # bench-runner smoke fixture (minimal)
└── maps/                      # known-map fixtures loaded by MatchFixtureLoader
    ├── openapi-sample.json    # official Procon 37 sample
    ├── small-8x8.json
    ├── medium-16x16.json
    ├── large-32x32.json
    ├── road-heavy.json
    ├── mountain-heavy.json
    ├── series-many.json
    └── series-few.json
```

## Fixture catalog

| File | Category | Size | Agents | Spots | Brands | Intended test |
|------|----------|------|--------|-------|--------|---------------|
| `maps/openapi-sample.json` | `official_sample` | 8×8 | 4 | 4 | 2 | warmup (loader round-trip) |
| `maps/small-8x8.json` | `small` | 8×8 | 3 | 5 | 2 | warmup / unit |
| `maps/medium-16x16.json` | `medium` | 16×16 | 4 | 12 | 4 | integration |
| `maps/large-32x32.json` | `large` | 32×32 | 8 | 16 | 4 | integration / soak |
| `maps/road-heavy.json` | `road_heavy` | 16×16 | 5 | 10 | 2 | road-aware pathing (solver-astar) |
| `maps/mountain-heavy.json` | `mountain_heavy` | 16×16 | 6 | 8 | 2 | terrain-aware routing (solver-loop, solver-mcts) |
| `maps/series-many.json` | `series_many` | 12×12 | 4 | 12 | 6 | variety-first (solver-series-clustered) |
| `maps/series-few.json` | `series_few` | 12×12 | 5 | 8 | 1 | total-noodles (solver-greedy) |

## Per-fixture descriptions

### `maps/openapi-sample.json`
`official_sample` — A minimal 8×8 map with a vertical mountain ridge and 4
agents at the four corners. Mirrors the warmup default in
`crates/cli/src/fixtures.rs`. Used to confirm the loader round-trips a
file end-to-end before any custom solver runs.

### `maps/small-8x8.json`
`small` — 8×8 warmup map: a single mountain ridge down the middle column
splits plain terrain into two halves, with 3 agents on the west side and
5 spots across 2 brands (kake / bukkake) on the east.

### `maps/medium-16x16.json`
`medium` — 16×16 reference map: 4 agents at the corners, 12 spots covering
4 brands (kake / bukkake / kamatama / zaru), a central mountain cross, and
a road loop on the perimeter. The default integration test fixture.

### `maps/large-32x32.json`
`large` — 32×32 reference map: 8 agents at corners and mid-edges, 16 spots
across 4 brands (kake / bukkake / kamatama / zaru), a road border, a diagonal
mountain ridge, and a pond cluster on the NW. Designed to stress solver
memory and search.

### `maps/road-heavy.json`
`road_heavy` — 16×16 map dominated by a road network: central road cross
plus four diagonal roads radiating to the corners, with 5 agents and 10
spots across 2 brands. Solvers that special-case road cost (solver-astar)
should dominate.

### `maps/mountain-heavy.json`
`mountain_heavy` — 16×16 map dominated by mountains with two plain
corridors and a road perimeter. 6 agents and 8 spots across 2 brands.
Solvers that special-case terrain (solver-loop, solver-mcts) should
dominate.

### `maps/series-many.json`
`series_many` — 12×12 map with 12 spots spread across 6 brands and 4
agents. Designed for variety-first solvers (solver-series-clustered) to
dominate the leaderboard.

### `maps/series-few.json`
`series_few` — 12×12 map with 8 spots on a single brand (kake) and 5
agents, divided by a mountain wall. Designed for total-noodles solvers
(solver-greedy) to dominate the leaderboard.

## Adding a new fixture

1. Pick a name and add it to `MatchFixtureLoader::bundled_fixtures()` in
   `crates/bench/src/fixture.rs`.
2. Pick a category (`FixtureCategory`) — keep it consistent with the
   fixture's actual characteristics.
3. Drop the file under `crates/bench/fixtures/maps/<name>.json`.
4. Run `cargo test -p bench` — the smoke tests in
   `crates/bench/tests/fixtures_smoke.rs` will pick it up automatically.

The fixture must:

- Have `width`, `height ∈ [8, 32]` (the contest's hard bounds).
- Have `3..=8` agents and `3..=32` spots.
- Have `cells: [height][width]` — every row the right length.
- Have agent positions and spot positions inside `[0, width*height)`.
- Have no agent starting on a spot (start cells must be clear).
- Pass the `series_many` / `series_few` category-specific assertions.

## Sources

- `example_match.json` — minimal bench-runner fixture from the upstream
  bench crate; documented in `crates/bench/src/lib.rs`.
- `openapi-sample.json` — derived from the official Procon 37 OpenAPI
  schema at `.reference/procon37/openapi.json`, mirrored in Rust at
  `crates/protocol/src/match_config.rs`.
- All other map fixtures — hand-authored for the v0.2.0 release. They are
  designed to exercise distinct solver characteristics, not to mimic any
  particular real contest map.