# Known-map fixtures (Issue #59)

This directory bundles the **fixed corpus** for the canonical offline
evaluation — see [`docs/evaluation.md`](../../../docs/evaluation.md). Every
fixture here becomes one case (`case_id = fixture:<file stem>`) that any
candidate can be run against:

```bash
just eval                                    # all fixtures, default candidates
just eval-list                               # what candidates exist
cargo run -q -p bench --bin procon-eval -- \
  --cases small-8x8,medium-16x16 --candidates 'loop:iterations=32,wait' --baseline wait
```

Each fixture matches the official Procon 37 `MatchSetting` shape
(`crates/protocol/src/match_config.rs`) wrapped in a small metadata header
(`name`, `description`, `category`) so the runner can pick fixtures by
characteristic without re-deriving labels.

> **Complementary, not replaced:** this directory stays the fixed corpus.
> Seed-fixed *generated* cases (Issue #185) are opt-in via
> `procon-eval --generated` and never change what `--cases all` means here —
> including the procedure for promoting a high-value generated case into a
> new fixture in this directory. See
> [`docs/evaluation.md`](../../../docs/evaluation.md#generated-corpusseed-固定の再生成-case).

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
- Have no agent starting on impassable terrain (`Pond`) — an agent planted
  on water can never move, which kills the case outright. This is enforced by
  `MatchFixture::validate_positions` at load time; see the regression notes
  below.
- Pass the `series_many` / `series_few` category-specific assertions.

All of the above run whenever a fixture is loaded into a case, so a broken
fixture fails at *load* rather than silently producing `invalid` records for
an entire corpus column.

## Known content caveats

**`maps/road-heavy.json` — terrain content does not match its category.**
The file is tagged `road_heavy` and the catalog below says it is "dominated
by a road network", but the actual cells are `Plain 198 / Pond 54 / Road 4`.
It therefore does **not** exercise road-cost-aware pathing the way it claims,
and the matching README row ("road-aware pathing (solver-astar)") is not
being served.

This is a *corpus content* defect, not a loader or evaluation-path defect: the
fixture validates and evaluates fine. Re-authoring the terrain would change
what every solver is tested on, so it is deliberately left for a fixture
ticket rather than silently rewritten here. Treat `road-heavy` as a
general-purpose map until it is fixed.

Previously all five of its agents also sat on `Pond` cells, which made the
case unusable (`invalid` for every candidate). The loader now rejects agents
on impassable terrain, so that class of defect cannot reach an evaluation run
again.

## Sources

- `example_match.json` — minimal bench-runner fixture from the upstream
  bench crate; documented in `crates/bench/src/lib.rs`.
- `openapi-sample.json` — derived from the official Procon 37 OpenAPI
  schema at `.reference/procon37/openapi.json`, mirrored in Rust at
  `crates/protocol/src/match_config.rs`.
- All other map fixtures — hand-authored for the v0.2.0 release. They are
  designed to exercise distinct solver characteristics, not to mimic any
  particular real contest map.