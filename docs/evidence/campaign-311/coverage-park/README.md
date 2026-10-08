# coverage park=far — daily-variety repositioning evidence

Issue #331. Parent candidate: `coverage:budget=official` (#313).
All numbers from `procon-eval` release profile, `--kinds P,S,P,S,P`, SHA in `source-sha.txt`.

## Result summary (fixture:701, 10 days)

| candidate | variety | daily_variety_sum | total_balls | daily series acquired |
|---|---:|---:|---:|---|
| `coverage:budget=official:park=none` | 20 | 195 | 379 | 20,20,18,20,20,20,19,20,18,20 |
| `coverage:budget=official:park=far` | 20 | **198** | 358 | 20,20,19,20,20,20,19,20,20,20 |

- **Axis 2 (+3)**: day 2 18→19, day 8 18→20, day 6 unchanged 19.
- Axis 1 unchanged at 20/20.
- Axis 3 (−21): repositioning trades ball harvest for daily coverage. Lexicographic order makes this a net win.
- 0 timeouts / invalids / panics / illegal actions / fuel shorts.

## Full corpus (14 cases: 9 fixtures + 701 + 4 generated)

| candidate | W/D/L vs park=none | V mean | D mean | B mean |
|---|---|---:|---:|---:|
| park=far | 2W / 12D / 0L | 4.29 | **19.71** | 56.9 |
| park=none (baseline) | — | 4.29 | 19.50 | 58.4 |

No case regresses on axis 1 or 2. Axis 3 drops only on 701 (−21) and many-series (0 in this run).

## What `park=far` changes

Day-scoped claim set + spreading guard on the coverage target key:

1. `day_claimed` persists across steps (not reset per step like `reserved`), so two patrols never re-target the same series after the first has claimed it.
2. Spreading activates only when `today_steps >= 70 && tomorrow_steps >= 70`.
   - Medium-length days before a short day keep the original greedy so the next start position is preserved.
   - Very short days (< 70) keep the original greedy (already near their coverage ceiling; scattering patrols there loses more than it gains).
3. `park=none` is byte-identical to the original `coverage:budget=official`.

## Reproduction

```bash
cargo build --release -p bench --bin procon-eval --offline
PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) ./target/release/procon-eval \
  --cases all \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --generated 'open:seed=17:days=3,open:seed=29:days=3,many-series:seed=3:days=3,open:seed=1:days=3:fuel=tight' \
  --kinds 'P,S,P,S,P' \
  --candidates 'coverage:budget=official:park=none,coverage:budget=official:park=far' \
  --baseline 'coverage:budget=official:park=none' \
  --output-dir target/eval-331
```

## Negative results kept for context

| variant | 701 D | 701 B | outcome |
|---|---:|---:|---|
| park=risk (risk preference in coverage key) | 178 | 287 | risk preference leaked into coverage phase |
| park=risk (harvest-only, threshold budget/4) | 194 | 334 | day 6 collapsed to 14 (repositioning stranded patrols) |
| park=far (farthest-first harvest, no fuel guard) | 194 | 240 | fuel exhaustion: patrol fuel hit 0 for 3 consecutive days |
| park=far (day_claimed spreading, no threshold) | 192 | 349 | short days scatter; days 6/8 lost 2 each |
| park=far (day_claimed, threshold >= 60) | 195 | 362 | day 2 +1 but day 6 −1 (day-5 spreading hurt day-6 start) |
| **park=far (day_claimed, >= 70 && tomorrow >= 70)** | **198** | **358** | **adopted** |
| owner-aware claims (repel only other agents) | 196 | 351 | day 2 perfect but day 6/8 worse (commitment vs sweep) |
| coverage-phase-only spreading | 193 | 335 | harvest repositioning is essential for D gain |
| approach-target (move toward unreachable unacquired) | 196 | 343 | day-end transit ruins next-day start positions |
| distance-limited spread (cost <= 15) | 151 | 230 | repositioning requires far targets; nearby spread is noise |
| cross-day miss memory | 194 | 363 | boost disrupts long-day coverage (day 8 lost 4) |
| fuel-aware supply pairing | 192 | 360 | nearest-supply keeps escort proximity better |
| threshold sweep 65/70/75 | 198 | 358 | plateau; 80 breaks day 8 (7/20) |

## Live arena evidence

4 consecutive completed matches with `coverage:budget=official:park=far` @ `6ff04c1`:
match 1752 (4d), 1754 (8d), 1756 (4d), 1758 (4d). Match 1760 (7-agent) rejected server-side.
