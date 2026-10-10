# Track B: convoy series-cluster label partitioning (Issue #432)

Branch `432` (base `2b5eb453` = release-0-5-1 with convoy merged). Two
param-gated mechanisms, both **default off**:

| param | effect | commit |
| --- | --- | --- |
| `bail=on` | when a patrol-day label search fails (cap/deadline), the pool holds only `fleet`'s plan — answer it immediately instead of burning the rest of the window on a one-node beam | `eb2f5de` |
| `cluster=on` (`clusters=K`, `0`=auto) | partition spots into geographic clusters; each patrol's label-search mask only carries its own cluster's spots | `0f07a4c` |

## Problem

s5020 (32×32, 28 spots, 27 series, 8 agents, 1 supply): the patrol-day
label search state is `(cell, collected-spot mask)` with Pareto `(tick,
fuel)` fronts. The mask spans all 28 spots, so the label space explodes
past the 1.5M cap on every day (`pool==1` for the whole match); raising
the cap to 4M and the budget to 59 s does not complete it. `decide()`
still plays the full beam over that single node, so a 20 s day budget is
spent to return `fleet`'s plan.

## Mechanism 1 — `bail=on`

With `pool.len() == 1` the beam provably cannot change the answer: the
only root is `fleet`'s plan and `best` is always `0`
(`(1..pool.len())` is empty). `bail=on` answers `pool[0]` as soon as the
root `day_plans` returns with a single entry — measured on such boards
as response 20 s → ≈5 s (the fleet's quarter-window slice) at equal
score, because the ~2/3 of the window the beam would have spent on
replays is simply not spent.

Deliberately **not** done: rebuilding the embedded fleet with the full
window on the failure path. That would push the response back to the
whole window; the win here is the early answer.

## Mechanism 2 — `cluster=on`

Deterministic, coordinates only, no randomness:

1. Split the spot set into `k` groups by repeated median splits: always
   split the largest group (tie: lowest group index) on its wider axis
   (column span vs row span, ties toward columns), members ordered by
   coordinate then spot index, lower half keeps the group id.
2. `k = clusters` param, `0` = auto = `min(patrols, 4)`; clamped to
   `1..=patrols` so every cluster has at least one patrol.
3. Patrol `p` is responsible for cluster `p % k`.
4. In patrol `p`'s label search, a spot of cluster `p % k` gets a
   **compact local bit** (its rank inside the cluster); other spots get
   **no bit** — passing through them is free and picking them still
   scores balls in the official replay, but they contribute nothing to
   the search's mask state. The mask dimension drops from
   `2^(all spots)` to `2^(cluster spots)`.

Coverage stays correct: every spot belongs to some cluster and every
cluster has a patrol, so the union of the patrol masks can still reach
every series. A series with spots in two clusters appears in both
clusters' masks — both patrols may pick it, and `tally` charges the
stock per visitor as before. `tally` translates patrol-local masks back
to global spots through per-patrol spot→bit tables (`Board::cluster`).

With `cluster=off` the bit table is the identity (global spot index),
which reproduces the pre-change search exactly; `default_params_keep_
the_pre_432_day_plan` pins the day-0 answer on the 701 fixture by plan
hash.

## Expected label reduction

- s5020: 28 spots → `k = min(8 patrols, 4) = 4` clusters of ~7 spots →
  mask space 2^28 → ~2^7 per patrol (~2^21× smaller state space). The
  label cap should no longer bind, so `pool` can grow beyond 1 and the
  touch/escort combinations become searchable.
- 701 (24×24, 24 spots, 5 agents): `k = 4` clusters of 6 spots → 2^24 →
  2^6.

## Tests (crates/bench/tests/convoy_candidate.rs)

- param validation: `bail=on/off/bad`, `cluster=on/off/bad`,
  `clusters=K` range and `clusters` without `cluster=on` rejected.
- default identity: `cluster=off + bail=off` day-0 plan on 701 equals
  the pre-#432 plan (FNV hash 5676638542188020946 over the action list).
- `bail=on` with a 1-label cap answers the embedded fleet's plan
  (equality) in < 10 s; `bail=off` still answers a well-shaped day.
- `cluster=on` reproducibility (two identical runs, iters mode).
- `cluster=on` full 701 match: replay clean, well-shaped, official
  normalization fixed point, daily variety sum >= 150.
- auto `clusters` covers every spot (well-shaped day on 701).

## Smoke (701 fixture, single runs, budget_ms=20000, seed=42, jobs=1)

Two runs of `procon-eval` on `crates/bench/tests/fixtures/arena-701.json`
(kinds `S,P,P,P,P`), one pair each: baseline vs `cluster=on` (both
illegal/fuel-short free, `day_budget_exceeded` empty).

| candidate | V (series) | daily sum | B (balls) |
| --- | --- | --- | --- |
| `convoy:…budget_ms=20000:seed=42` | 20 | 200 | 573 / 575 |
| `convoy:…seed=42:cluster=on` | 20 | 200 | 575 / 572 |

Both saturate 701 (all 20 series every day): cluster=on neither gains
nor loses on this 24×24 board — the label search was not the binding
constraint here. Daily responses hover at the 20 s budget on the first
days for both (the beam, not the label search, spends it).

**Pool size on 701**: `procon-eval` workers never install a tracing
subscriber (only `crates/cli` does), so RUST_LOG=info produced no
"convoy day plan" lines. Direct evidence instead: with default labels,
701 day-0 `convoy:…:depth=1:width=2:forecast=persist` answers exactly
`fleet`'s plan (plan equality probe), i.e. the pool holds only fleet's
entry — baseline convoy is pool==1 on 701 too, same failure shape as
s5020, just not score-visible because fleet already saturates the board.
`cluster=on` is the mechanism that should lift s5020's 32×32 pool beyond
1; that check belongs to the measurement team (machine-load rules kept
this worker to the single 701 smoke).
