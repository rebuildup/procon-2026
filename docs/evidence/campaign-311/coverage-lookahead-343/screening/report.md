# Evaluation report

- commit: `081aa070364489011328dbade65e94bb2d27c0e7`
- baseline: `coverage:budget=official:park=far` (win / draw / lose is measured against it)
- candidates: 4
- cases: 10
- runs: 40 (40 scored)

Ranking order is the official lexicographic one — **variety → daily variety sum → total balls → response time (lower better)**. It is never collapsed into a weighted score. Response time is measured; wall-clock elapsed is recorded separately and never ranked.

## Candidates

| candidate | wins | draws | losses | timeouts | invalid | panics | variety (min–max) | daily sum (min–max) | total balls (min–max) | response µs (min–max) |
|---|---:|---:|---:|---:|---:|---:|---|---|---|---|
| `coverage:budget=official:park=far` | 0 | 10 | 0 | 0 | 0 | 0 | 1–20 | 1–198 | 4–358 | 324–427519 |
| `search:trials=8:seed=42:budget=official:park=far` | 4 | 0 | 6 | 0 | 0 | 0 | 1–20 | 1–191 | 4–352 | 2168–3074045 |
| `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 6 | 0 | 4 | 0 | 0 | 0 | 1–20 | 1–198 | 4–367 | 1300–3473636 |
| `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 6 | 0 | 4 | 0 | 0 | 0 | 1–20 | 1–198 | 4–370 | 2224–5877668 |

Comparison counts cover only cases where **both** the candidate and the baseline produced a score; the rest land in `not compared`.

## Per-case ranking

### `fixture:openapi-sample`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 324 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 1823 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 4 | 2448 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 2936 | success |

### `fixture:small-8x8`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 7 | 325 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 7 | 1300 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 7 | 2168 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 7 | 2224 | success |

### `fixture:medium-16x16`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 11 | 8545 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 11 | 16895 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 11 | 17145 | success |
| 4 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 1608 | success |

### `fixture:large-32x32`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 19 | 191461 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 19 | 386033 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 19 | 429453 | success |
| 4 | `coverage:budget=official:park=far` | 4 | 4 | 14 | 53875 | success |

### `fixture:road-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 984 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 3770 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 4698 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 4 | 9593 | success |

### `fixture:road-heavy-v2`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 3 | 9 | 28 | 98682 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 3 | 9 | 28 | 179157 | success |
| 3 | `coverage:budget=official:park=far` | 3 | 9 | 25 | 12932 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 3 | 9 | 25 | 115297 | success |

### `fixture:mountain-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 9 | 6663 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 9 | 21001 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 9 | 36633 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 9 | 46756 | success |

### `fixture:series-many`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 10 | 2949 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 10 | 6352 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 10 | 10005 | success |
| 4 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 793 | success |

### `fixture:series-few`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 1 | 1 | 10 | 7221 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 1 | 1 | 10 | 12350 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 1 | 1 | 10 | 13710 | success |
| 4 | `coverage:budget=official:park=far` | 1 | 1 | 9 | 1599 | success |

### `fixture:701`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 20 | 198 | 370 | 5877668 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 20 | 198 | 367 | 3473636 | success |
| 3 | `coverage:budget=official:park=far` | 20 | 198 | 358 | 427519 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 20 | 191 | 352 | 3074045 | success |

