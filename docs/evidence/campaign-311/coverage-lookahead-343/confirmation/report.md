# Evaluation report

- commit: `081aa070364489011328dbade65e94bb2d27c0e7`
- baseline: `coverage:budget=official:park=far` (win / draw / lose is measured against it)
- candidates: 4
- cases: 15
- runs: 60 (60 scored)

Ranking order is the official lexicographic one — **variety → daily variety sum → total balls → response time (lower better)**. It is never collapsed into a weighted score. Response time is measured; wall-clock elapsed is recorded separately and never ranked.

## Candidates

| candidate | wins | draws | losses | timeouts | invalid | panics | variety (min–max) | daily sum (min–max) | total balls (min–max) | response µs (min–max) |
|---|---:|---:|---:|---:|---:|---:|---|---|---|---|
| `coverage:budget=official:park=far` | 0 | 15 | 0 | 0 | 0 | 0 | 1–20 | 1–198 | 4–358 | 383–399429 |
| `search:trials=8:seed=42:budget=official:park=far` | 5 | 0 | 10 | 0 | 0 | 0 | 1–20 | 1–191 | 4–352 | 2263–4217381 |
| `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 7 | 1 | 7 | 0 | 0 | 0 | 1–20 | 1–198 | 4–367 | 952–4050552 |
| `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 7 | 0 | 8 | 0 | 0 | 0 | 1–20 | 1–198 | 4–370 | 1853–5752618 |

Comparison counts cover only cases where **both** the candidate and the baseline produced a score; the rest land in `not compared`.

## Per-case ranking

### `fixture:openapi-sample`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 489 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 1255 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 4 | 2263 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 2374 | success |

### `fixture:small-8x8`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 7 | 383 | success |
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 7 | 952 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 7 | 1853 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 7 | 2398 | success |

### `fixture:medium-16x16`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 11 | 7406 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 11 | 14919 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 11 | 14937 | success |
| 4 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 2615 | success |

### `fixture:large-32x32`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 19 | 189047 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 19 | 354060 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 19 | 362048 | success |
| 4 | `coverage:budget=official:park=far` | 4 | 4 | 14 | 58737 | success |

### `fixture:road-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 625 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 2568 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 4 | 4898 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 4 | 5048 | success |

### `fixture:road-heavy-v2`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 3 | 9 | 28 | 69104 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 3 | 9 | 28 | 145120 | success |
| 3 | `coverage:budget=official:park=far` | 3 | 9 | 25 | 10343 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 3 | 9 | 25 | 104836 | success |

### `fixture:mountain-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 9 | 3756 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 9 | 17439 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 2 | 2 | 9 | 38148 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 9 | 59776 | success |

### `fixture:series-many`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 10 | 2506 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 10 | 5689 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 4 | 10 | 5368 | success |
| 4 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 764 | success |

### `fixture:series-few`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 1 | 1 | 10 | 6456 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 1 | 1 | 10 | 9734 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 1 | 1 | 10 | 11400 | success |
| 4 | `coverage:budget=official:park=far` | 1 | 1 | 9 | 1059 | success |

### `fixture:701`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 20 | 198 | 370 | 5752618 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 20 | 198 | 367 | 4050552 | success |
| 3 | `coverage:budget=official:park=far` | 20 | 198 | 358 | 399429 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 20 | 191 | 352 | 4217381 | success |

### `generated:v2:open:seed=31:days=3`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 6 | 18 | 40 | 19189 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 6 | 18 | 40 | 114562 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 6 | 18 | 40 | 174452 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 6 | 18 | 40 | 229852 | success |

### `generated:v2:road-heavy:seed=19`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 4 | 12 | 106 | 50082 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 12 | 106 | 264034 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 12 | 106 | 521952 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 12 | 105 | 390093 | success |

### `generated:v2:mountain-heavy:seed=23:size=small`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 3 | 3 | 15 | 1256 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 3 | 3 | 15 | 4625 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 3 | 3 | 15 | 9744 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 3 | 3 | 15 | 9397 | success |

### `generated:v2:many-series:seed=19`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 12 | 36 | 114 | 162314 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 12 | 36 | 114 | 258247 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 12 | 36 | 114 | 343518 | success |
| 4 | `coverage:budget=official:park=far` | 12 | 36 | 113 | 32547 | success |

### `generated:v2:pond-obstacle:seed=11:fuel=tight`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 4 | 8 | 18 | 13146 | success |
| 2 | `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 4 | 8 | 18 | 65767 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 8 | 18 | 88848 | success |
| 4 | `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 4 | 8 | 18 | 120836 | success |

