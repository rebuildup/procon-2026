# Evaluation report

- commit: `551f847eecff2bd67a4e8cf483202f3014c7ed34`
- baseline: `coverage:budget=official:park=far` (win / draw / lose is measured against it)
- candidates: 3
- cases: 15
- runs: 45 (45 scored)

Ranking order is the official lexicographic one — **variety → daily variety sum → total balls → response time (lower better)**. It is never collapsed into a weighted score. Response time is measured; wall-clock elapsed is recorded separately and never ranked.

## Candidates

| candidate | wins | draws | losses | timeouts | invalid | panics | variety (min–max) | daily sum (min–max) | total balls (min–max) | response µs (min–max) |
|---|---:|---:|---:|---:|---:|---:|---|---|---|---|
| `coverage:budget=official:park=far` | 0 | 15 | 0 | 0 | 0 | 0 | 1–20 | 1–198 | 4–358 | 281–495657 |
| `search:trials=16:seed=42:budget=official:park=far` | 5 | 0 | 10 | 0 | 0 | 0 | 1–20 | 1–196 | 4–376 | 4438–5994340 |
| `search:trials=32:seed=42:budget=official:park=far` | 6 | 0 | 9 | 0 | 0 | 0 | 1–20 | 1–196 | 4–377 | 10742–12902640 |

Comparison counts cover only cases where **both** the candidate and the baseline produced a score; the rest land in `not compared`.

## Per-case ranking

### `fixture:openapi-sample`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 355 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 4 | 4438 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 4 | 14766 | success |

### `fixture:small-8x8`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 7 | 281 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 7 | 6395 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 7 | 10742 | success |

### `fixture:medium-16x16`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 4 | 11 | 36922 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 4 | 11 | 96609 | success |
| 3 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 1822 | success |

### `fixture:large-32x32`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 4 | 20 | 1124604 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 4 | 20 | 2126988 | success |
| 3 | `coverage:budget=official:park=far` | 4 | 4 | 14 | 67113 | success |

### `fixture:road-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 629 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 4 | 12531 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 4 | 23292 | success |

### `fixture:road-heavy-v2`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=32:seed=42:budget=official:park=far` | 3 | 9 | 27 | 465539 | success |
| 2 | `coverage:budget=official:park=far` | 3 | 9 | 25 | 11481 | success |
| 3 | `search:trials=16:seed=42:budget=official:park=far` | 3 | 9 | 25 | 201762 | success |

### `fixture:mountain-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 9 | 3130 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 9 | 75347 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 9 | 159392 | success |

### `fixture:series-many`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 4 | 10 | 10441 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 4 | 10 | 31225 | success |
| 3 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 730 | success |

### `fixture:series-few`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official:park=far` | 1 | 1 | 11 | 35296 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 1 | 1 | 11 | 38342 | success |
| 3 | `coverage:budget=official:park=far` | 1 | 1 | 9 | 1280 | success |

### `fixture:701`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 20 | 198 | 358 | 495657 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 20 | 196 | 377 | 12902640 | success |
| 3 | `search:trials=16:seed=42:budget=official:park=far` | 20 | 196 | 376 | 5994340 | success |

### `generated:v2:open:seed=31:days=3`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 6 | 18 | 40 | 19520 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 6 | 18 | 40 | 390494 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 6 | 18 | 40 | 740124 | success |

### `generated:v2:road-heavy:seed=19`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 4 | 12 | 106 | 42341 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 12 | 106 | 900699 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 12 | 106 | 1852441 | success |

### `generated:v2:mountain-heavy:seed=23:size=small`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 3 | 3 | 15 | 1127 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 3 | 3 | 15 | 19033 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 3 | 3 | 15 | 53902 | success |

### `generated:v2:many-series:seed=19`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official:park=far` | 12 | 36 | 114 | 475557 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 12 | 36 | 114 | 935839 | success |
| 3 | `coverage:budget=official:park=far` | 12 | 36 | 113 | 41419 | success |

### `generated:v2:pond-obstacle:seed=11:fuel=tight`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 4 | 8 | 18 | 10519 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 8 | 18 | 214941 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 8 | 18 | 341281 | success |
