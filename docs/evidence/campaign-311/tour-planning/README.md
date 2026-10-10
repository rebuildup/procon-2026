# tour-planning evidence (Issue #333)

32×32 大盤ツアー計画の評価 evidence。Issue #333 / PR #334。

## Files

| File | What |
|---|---|
| `evaluation.json` | Current measurement at `0ee22c4` (generator fix included) |
| `evaluation-pre-generator-fix.json` | Pre-fix snapshot at `8f4cc89` — kept for history, do not edit |

## Why two snapshots

`SeriesPattern` (Uniform / Central / Peripheral) was a **no-op** before
`0ee22c4` ("fix(bench): implement series pattern placement in 32x32
generator"). The pre-fix suite therefore produced **identical** results for
cases 0-2, which is not a real measurement of the distribution axis. The
same 9-case × 3-candidate suite was re-run at `0ee22c4`; `evaluation.json`
is that run. The pre-fix numbers are preserved verbatim in
`evaluation-pre-generator-fix.json` so the history is not erased.

## Reproduce

```bash
PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) \
  cargo test -p bench --release --test large32_evaluation --offline --locked -- --nocapture
```

The test prints a machine-readable JSON block between
`=== BEGIN evaluation.json ===` / `=== END evaluation.json ===` markers.

## Invalid-case policy

`valid` = `RunRecord.scored` (no illegal actions, no fuel shorts, no
day-budget overruns, no malformed plan). Invalid runs are recorded with
their observed numbers but **excluded from valid-only tallies**; all-case
tallies include them and are labelled as such. Each case carries both a
per-candidate `valid` flag and a case-level `valid_all_candidates` flag.

## Reading the judgment

Official order is lexicographic: `variety`, `daily_variety_sum`,
`total_balls`, `-response_time`. Tiers 1-3 are deterministic axis metrics;
`response_time_us` is a wall-clock **measurement** and is only consulted to
break ties among axis-equal candidates — it is never folded into the axis
win counts.
