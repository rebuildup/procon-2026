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
    ├── road-heavy.json        # legacy case (pre-#187 content, relabeled `medium`)
    ├── road-heavy-v2.json     # genuine road network + traffic-threshold case (#187)
    ├── mountain-heavy.json
    ├── series-many.json
    └── series-few.json
```

## Terrain cell codes (Issue #226)

Every `cells` value here follows the **official** table —
`試合設定JSONについて.md` §「cells の地形タイプ」, bundled inside
`docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip` (inner path
`試合設定用JSONファイル/試合設定JSONについて.md`):

| value | terrain |
|------:|---------|
| `0` | 平地 (Plain) |
| `1` | 道路 (Road) |
| `2` | 山地 (Mountain) |
| `3` | 池 (Pond, impassable) |

Until Issue #226 the engine decoded a **permuted** table (`1`=Mountain,
`2`=Pond, `3`=Road), and everything authored against that table therefore
carried permuted codes. #226 corrected `engine::from_wire` and applied the
meaning-preserving permutation `0→0, 1→2, 2→3, 3→1` to authored content:

- **permuted** — the authored maps `small-8x8`, `medium-16x16`,
  `large-32x32`, `road-heavy`, `road-heavy-v2`, `mountain-heavy`,
  `series-many`, `series-few`, plus `bench::generator`'s cell painting and
  the gates that count codes (road floor `TYPE_1`, pond checks `TYPE_3`);
- **kept as-is** — `maps/openapi-sample.json`: no permutation is applied to
  it, so its codes are read exactly like a raw wire payload under the
  corrected table (its code-`1` column is therefore a *road*, not a
  mountain — see the description below).

Terrain *counts* in this README are terrain, not codes: they are unchanged
by the permutation.

## Fixture catalog

| File | Category | Size | Agents | Spots | Brands | Intended test |
|------|----------|------|--------|-------|--------|---------------|
| `maps/openapi-sample.json` | `official_sample` | 8×8 | 4 | 4 | 2 | warmup (loader round-trip) |
| `maps/small-8x8.json` | `small` | 8×8 | 3 | 5 | 2 | warmup / unit |
| `maps/medium-16x16.json` | `medium` | 16×16 | 4 | 12 | 4 | integration |
| `maps/large-32x32.json` | `large` | 32×32 | 8 | 16 | 4 | integration / soak |
| `maps/road-heavy.json` | `medium` (legacy) | 16×16 | 5 | 10 | 2 | historical `fixture:road-heavy` evidence — pre-#187 content, **not** a road case |
| `maps/road-heavy-v2.json` | `road_heavy` | 16×16 | 5 | 10 | 3 | road-aware vs road-agnostic routing + traffic-threshold state flips (3-day) |
| `maps/mountain-heavy.json` | `mountain_heavy` | 16×16 | 6 | 8 | 2 | terrain-aware routing (solver-loop, solver-mcts) |
| `maps/series-many.json` | `series_many` | 12×12 | 4 | 12 | 6 | variety-first (solver-series-clustered) |
| `maps/series-few.json` | `series_few` | 12×12 | 5 | 8 | 1 | total-noodles (solver-greedy) |

## Per-fixture descriptions

### `maps/openapi-sample.json`
`official_sample` — A minimal 8×8 map whose cell codes were **not**
permuted by #226 (see the terrain-code section above): under the official
table the vertical line of `1`s down the middle column is a **road**, and
4 agents sit at the four corners (all on plain). Mirrors the warmup default
in `crates/cli/src/fixtures.rs`. Used to confirm the loader round-trips a
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
`medium` (legacy) — the **pre-#187** content of case `fixture:road-heavy`:
a pond-heavy 16×16 field (Plain 198 / Pond 54 / **Road 4**) with 5 agents
and 10 spots across 2 brands. #187 left the body untouched, and #226 only
identity-permuted its cell *codes* — the terrain mix is unchanged (still
Road 4 / Pond 54), so every terrain-based description of the historical
evidence stays true. It never matched its old `road_heavy` tag,
so #187 relabeled the category to `medium` (it is 16×16, which *is*
honest). Treat it as a general-purpose
map; for road evidence use `road-heavy-v2`.

### `maps/road-heavy-v2.json`
`road_heavy` — the genuine road corpus case added by #187. 16×16 with
**80/256 road cells (31%)**: two cross-town avenues (rows 4 and 10), a
perimeter loop (cols 0 and 15), and two southern spurs (cols 4 and 11).
5 agents (all on plain terrain), 10 spots across 3 brands, all on plain
cells; a north lake (12 ponds), a south lake (9 ponds) and two mountain
clusters (15 cells) provide barriers without disconnecting the map.

The routing choice is structural, not incidental. Agent 0 starts at
`(1,3)` and the showcase spot sits at `(13,3)`:

| Route | Cells | move-steps (Smooth) | move-steps (Busy) | move-steps (Jammed) | fuel |
|---|---|---:|---:|---:|---:|
| road corridor via row 4 | 14 (13 road) | **15** | 28 | 54 | 27 |
| plain detour along row 3 | 12 (all plain) | 24 | **24** | **24** | 12 |

Costs come from `domain::cell` step tables (Plain 2 / Road 1-2-4) and
`engine::rules::fuel_cost` (Plain 1, Road 2) — the same tables the
planners and the simulator use. While the corridor is `Smooth` a
road-aware planner strictly prefers it; once it flips to `Busy` the
advantage inverts and the plain detour wins; a road-*agnostic* planner
(cell-count only) picks the detour in every state — so the two strategies
diverge observably on this topology. The corridor always costs more fuel,
which is the counter-pressure road-aware routing has to budget for.

It is the corpus's first **multi-day** case: `daySteps [40, 16, 16]`
(3 days), `daySeconds [60, 60, 60]`, `busyThreshold 1`,
`jammedThreshold 2`, `fuelLimits 120`. Multi-day is load-bearing: road
traffic is classified from *two days* of counts (`classify_traffic` /
`phase_traffic_update`), so a single-day case can never exhibit a state
transition. Here day-1 traffic flips corridor cells to `Busy` for day 2,
day-2 traffic jams the over-used cells for day 3, and both flips are
experienced by routing mid-match. See the caveat section for why the day
lengths are non-uniform.

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
  below. Spots must also sit on walkable cells (put them on plain) — see the
  caveat about legacy pond spots below.
- Pass the `series_many` / `series_few` category-specific assertions, and
  for `road_heavy` the density floor in `validate_category`: road cells
  (`CellType::TYPE_1`, the official road code) ≥ 15% of the map
  (Issue #187 floor, re-coded by #226).
- For traffic-threshold evidence, declare ≥ 2 days (`daySteps` /
  `daySeconds` of equal length) — see the multi-day caveat below.

All of the above run whenever a fixture is loaded into a case, so a broken
fixture fails at *load* rather than silently producing `invalid` records for
an entire corpus column.

## Known content caveats

**Resolved by #187 — the `road-heavy` category mismatch.**
The historical `maps/road-heavy.json` shipped as `road_heavy` while its
actual cells were `Plain 198 / Pond 54 / Road 4` (1.6% roads): it did not
exercise road-cost-aware pathing at all, and a "road share" smoke assertion
that would have caught it was reading `CellType::TYPE_2` — which, under the
pre-#226 (wrong) table, was the *Pond* code — instead of the road code
(`TYPE_3` before #226, `TYPE_1` under the official table). #187 fixed both
and rebuilt the case as a genuine road
corpus entry, using **identity Option A** so old and new evidence can never
be read as each other:

1. **Legacy content kept, category relabeled.** `maps/road-heavy.json` keeps
   its map body (still Road 4 / Pond 54 — #226 re-coded the cell values
   without changing that mix) and keeps the case id
   `fixture:road-heavy`; only its `category` changed to `medium` (16×16 —
   honest) and its `description` was corrected. The loader now enforces a
   **road-density floor** for `road_heavy` fixtures in `validate_category`
   (road cells ≥ **15%** of the map — ≥ 39 of 256 on the 16×16 maps, vs the
   defective 4), and the smoke test counts `TYPE_1` (the official road
   code), so a road-light fixture
   can never claim the category again.
2. **New case, new id.** `maps/road-heavy-v2.json` (case id
   `fixture:road-heavy-v2`) carries the real 31%-road network, the
   Smooth/Busy/Jammed routing inversion, and the multi-day threshold
   scenario described above.

**Historical evidence warning.** Any evaluation record whose `case_id` is
`fixture:road-heavy` refers to the **pre-#187 content** (the pond-heavy
map above, formerly tagged `road_heavy`). It must not be re-read as, or
aggregated with, the new `fixture:road-heavy-v2` case — different ids,
different terrain, different category. Nothing was silently replaced under
an unchanged id. (Separately, #226 identity-permuted the cell *codes* of
every authored fixture here: terrain-based statements about these ids stay
valid, but a byte-level hash of a map body no longer matches a pre-#226
copy of the file.)

**Multi-day is required for traffic-threshold evidence.** All other bundled
fixtures are single-day. `classify_traffic` combines *today's* and
*yesterday's* counts (Q&A Q13), so on a single-day case
`busyThreshold`/`jammedThreshold` can never produce an observable state
transition inside the match. Any future fixture whose purpose includes
traffic behaviour must declare ≥ 2 `daySteps` entries (with a
`daySeconds` entry per day — `engine::from_wire` rejects length
mismatches).

**Why `road-heavy-v2` uses non-uniform days (`[40, 16, 16]`).**
`fuelLimits` is officially bounded to `daySteps[0] × 1..3`
(`docs/contest/api.md`, `docs/contest/openapi.md`; the official sample is
`daySteps [64,64,64]`, `fuelLimits 192` = 3×64) — the **cap** scales with
day 1 only, while fuel **burn** accumulates match-long. Measured on this
map (with `fuelLimits` temporarily raised so nothing drowns): the fuel-blind
`loop` baseline burns **100** fuel across a 3-day match, because it keeps
walking every step of every day. Three uniform days at the Q14 maximum
(`[24,24,24]` → cap 72) therefore invalidate `loop` (12 fuel-short
actions), whereas `[40,16,16]` → cap **120 ≥ 100** keeps every baseline
candidate inside the official envelope *and* inside the match. Note this
when authoring fixtures or (#185) generated cases: size `fuelLimits` from
*total* steps, not from day 1 alone — or shorten the match.

**Legacy spots on ponds (pre-existing, left as-is).** 4 of the legacy
`road-heavy` spots (`pos` 72 / 132 / 139 / 184) sit on `Pond` cells and are
unreachable, so only the other 6 spots can ever contribute to its score.
The loader rejects *agents* on impassable terrain (all five used to sit on
ponds, which made the case `invalid` for every candidate) but deliberately
does not reject spots on ponds: doing so would break the frozen legacy
content this section just promised to keep loadable. New fixtures should
place spots on plain cells — `road-heavy-v2` does, and its test
(`road_heavy_v2_positions_avoid_impassable_terrain`) pins that.

## Sources

- `example_match.json` — minimal bench-runner fixture from the upstream
  bench crate; documented in `crates/bench/src/lib.rs`.
- `openapi-sample.json` — derived from the official Procon 37 OpenAPI
  schema at `.reference/procon37/openapi.json`, mirrored in Rust at
  `crates/protocol/src/match_config.rs`.
- All other map fixtures — hand-authored for the v0.2.0 release. They are
  designed to exercise distinct solver characteristics, not to mimic any
  particular real contest map.