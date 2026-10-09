# Evaluation report

- commit: `551f847eecff2bd67a4e8cf483202f3014c7ed34`
- baseline: `coverage:budget=official:park=far` (win / draw / lose is measured against it)
- candidates: 8
- cases: 14
- runs: 112 (112 scored)

Ranking order is the official lexicographic one — **variety → daily variety sum → total balls → response time (lower better)**. It is never collapsed into a weighted score. Response time is measured; wall-clock elapsed is recorded separately and never ranked.

## Candidates

| candidate | wins | draws | losses | timeouts | invalid | panics | variety (min–max) | daily sum (min–max) | total balls (min–max) | response µs (min–max) |
|---|---:|---:|---:|---:|---:|---:|---|---|---|---|
| `coverage:budget=official:park=far` | 0 | 14 | 0 | 0 | 0 | 0 | 1–20 | 1–198 | 4–358 | 317–529620 |
| `search:trials=4:seed=42:budget=official` | 7 | 0 | 7 | 0 | 0 | 0 | 1–20 | 1–193 | 4–359 | 2247–1578855 |
| `search:trials=8:seed=42:budget=official` | 7 | 0 | 7 | 0 | 0 | 0 | 1–20 | 1–194 | 4–376 | 7657–4501920 |
| `search:trials=16:seed=42:budget=official` | 8 | 0 | 6 | 0 | 0 | 0 | 1–20 | 1–195 | 4–386 | 9072–9193113 |
| `search:trials=32:seed=42:budget=official` | 9 | 0 | 5 | 0 | 0 | 0 | 1–20 | 1–192 | 4–379 | 21959–14553586 |
| `search:trials=8:seed=42:budget=official:park=far` | 8 | 0 | 6 | 0 | 0 | 0 | 1–20 | 1–191 | 4–352 | 4265–4318051 |
| `search:trials=16:seed=42:budget=official:park=far` | 8 | 0 | 6 | 0 | 0 | 0 | 1–20 | 1–196 | 4–376 | 10724–8992216 |
| `search:trials=32:seed=42:budget=official:park=far` | 9 | 0 | 5 | 0 | 0 | 0 | 1–20 | 1–196 | 4–377 | 20097–13371298 |

Comparison counts cover only cases where **both** the candidate and the baseline produced a score; the rest land in `not compared`.

## Per-case ranking

### `fixture:openapi-sample`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 317 | success |
| 2 | `search:trials=4:seed=42:budget=official` | 2 | 2 | 4 | 2247 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 4 | 4776 | success |
| 4 | `search:trials=16:seed=42:budget=official` | 2 | 2 | 4 | 9072 | success |
| 5 | `search:trials=8:seed=42:budget=official` | 2 | 2 | 4 | 13759 | success |
| 6 | `search:trials=32:seed=42:budget=official` | 2 | 2 | 4 | 21959 | success |
| 7 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 4 | 22941 | success |
| 8 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 4 | 41138 | success |

### `fixture:small-8x8`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 7 | 1605 | success |
| 2 | `search:trials=4:seed=42:budget=official` | 2 | 2 | 7 | 4081 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 7 | 4265 | success |
| 4 | `search:trials=8:seed=42:budget=official` | 2 | 2 | 7 | 7657 | success |
| 5 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 7 | 10724 | success |
| 6 | `search:trials=16:seed=42:budget=official` | 2 | 2 | 7 | 17693 | success |
| 7 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 7 | 20097 | success |
| 8 | `search:trials=32:seed=42:budget=official` | 2 | 2 | 7 | 25020 | success |

### `fixture:medium-16x16`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 11 | 18927 | success |
| 2 | `search:trials=4:seed=42:budget=official` | 4 | 4 | 11 | 20711 | success |
| 3 | `search:trials=8:seed=42:budget=official` | 4 | 4 | 11 | 55892 | success |
| 4 | `search:trials=16:seed=42:budget=official` | 4 | 4 | 11 | 67602 | success |
| 5 | `search:trials=32:seed=42:budget=official` | 4 | 4 | 11 | 106885 | success |
| 6 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 4 | 11 | 116643 | success |
| 7 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 4 | 11 | 327972 | success |
| 8 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 5063 | success |

### `fixture:large-32x32`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official` | 4 | 4 | 20 | 1398679 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 4 | 20 | 1718125 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 4 | 20 | 3001110 | success |
| 4 | `search:trials=32:seed=42:budget=official` | 4 | 4 | 20 | 3339164 | success |
| 5 | `search:trials=4:seed=42:budget=official` | 4 | 4 | 19 | 672382 | success |
| 6 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 19 | 806459 | success |
| 7 | `search:trials=8:seed=42:budget=official` | 4 | 4 | 19 | 1081429 | success |
| 8 | `coverage:budget=official:park=far` | 4 | 4 | 14 | 142116 | success |

### `fixture:road-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 4 | 6588 | success |
| 2 | `coverage:budget=official:park=far` | 2 | 2 | 4 | 7721 | success |
| 3 | `search:trials=4:seed=42:budget=official` | 2 | 2 | 4 | 16279 | success |
| 3 | `search:trials=8:seed=42:budget=official` | 2 | 2 | 4 | 16040 | success |
| 5 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 4 | 20662 | success |
| 6 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 4 | 31496 | success |
| 7 | `search:trials=16:seed=42:budget=official` | 2 | 2 | 4 | 32194 | success |
| 8 | `search:trials=32:seed=42:budget=official` | 2 | 2 | 4 | 50844 | success |

### `fixture:road-heavy-v2`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=32:seed=42:budget=official` | 3 | 9 | 27 | 566620 | success |
| 1 | `search:trials=32:seed=42:budget=official:park=far` | 3 | 9 | 27 | 566432 | success |
| 3 | `coverage:budget=official:park=far` | 3 | 9 | 25 | 16073 | success |
| 4 | `search:trials=4:seed=42:budget=official` | 3 | 9 | 25 | 76665 | success |
| 5 | `search:trials=8:seed=42:budget=official:park=far` | 3 | 9 | 25 | 101660 | success |
| 6 | `search:trials=16:seed=42:budget=official` | 3 | 9 | 25 | 187501 | success |
| 7 | `search:trials=8:seed=42:budget=official` | 3 | 9 | 25 | 217092 | success |
| 8 | `search:trials=16:seed=42:budget=official:park=far` | 3 | 9 | 25 | 270235 | success |

### `fixture:mountain-heavy`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 2 | 2 | 9 | 6635 | success |
| 2 | `search:trials=4:seed=42:budget=official` | 2 | 2 | 9 | 24708 | success |
| 3 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 2 | 9 | 46757 | success |
| 4 | `search:trials=8:seed=42:budget=official` | 2 | 2 | 9 | 54075 | success |
| 5 | `search:trials=16:seed=42:budget=official` | 2 | 2 | 9 | 103828 | success |
| 6 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 2 | 9 | 106661 | success |
| 7 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 2 | 9 | 204090 | success |
| 8 | `search:trials=32:seed=42:budget=official` | 2 | 2 | 9 | 210436 | success |

### `fixture:series-many`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official` | 4 | 4 | 10 | 3750 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 4 | 4 | 10 | 9088 | success |
| 3 | `search:trials=8:seed=42:budget=official` | 4 | 4 | 10 | 11215 | success |
| 4 | `search:trials=16:seed=42:budget=official` | 4 | 4 | 10 | 17080 | success |
| 4 | `search:trials=16:seed=42:budget=official:park=far` | 4 | 4 | 10 | 17396 | success |
| 6 | `search:trials=32:seed=42:budget=official` | 4 | 4 | 10 | 35236 | success |
| 7 | `search:trials=32:seed=42:budget=official:park=far` | 4 | 4 | 10 | 43179 | success |
| 8 | `coverage:budget=official:park=far` | 4 | 4 | 9 | 993 | success |

### `fixture:series-few`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official` | 1 | 1 | 11 | 29917 | success |
| 2 | `search:trials=16:seed=42:budget=official:park=far` | 1 | 1 | 11 | 33681 | success |
| 3 | `search:trials=32:seed=42:budget=official` | 1 | 1 | 11 | 57180 | success |
| 4 | `search:trials=32:seed=42:budget=official:park=far` | 1 | 1 | 11 | 59784 | success |
| 5 | `search:trials=4:seed=42:budget=official` | 1 | 1 | 10 | 7050 | success |
| 6 | `search:trials=8:seed=42:budget=official` | 1 | 1 | 10 | 17506 | success |
| 6 | `search:trials=8:seed=42:budget=official:park=far` | 1 | 1 | 10 | 17624 | success |
| 8 | `coverage:budget=official:park=far` | 1 | 1 | 9 | 1636 | success |

### `fixture:701`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `coverage:budget=official:park=far` | 20 | 198 | 358 | 529620 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 20 | 196 | 377 | 13371298 | success |
| 3 | `search:trials=16:seed=42:budget=official:park=far` | 20 | 196 | 376 | 8992216 | success |
| 4 | `search:trials=16:seed=42:budget=official` | 20 | 195 | 386 | 9193113 | success |
| 5 | `search:trials=8:seed=42:budget=official` | 20 | 194 | 376 | 4501920 | success |
| 6 | `search:trials=4:seed=42:budget=official` | 20 | 193 | 359 | 1578855 | success |
| 7 | `search:trials=32:seed=42:budget=official` | 20 | 192 | 379 | 14553586 | success |
| 8 | `search:trials=8:seed=42:budget=official:park=far` | 20 | 191 | 352 | 4318051 | success |

### `generated:v2:open:seed=17:days=3`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=32:seed=42:budget=official` | 3 | 9 | 138 | 929381 | success |
| 2 | `search:trials=32:seed=42:budget=official:park=far` | 3 | 9 | 138 | 1760967 | success |
| 3 | `search:trials=16:seed=42:budget=official:park=far` | 3 | 9 | 133 | 476396 | success |
| 4 | `search:trials=16:seed=42:budget=official` | 3 | 9 | 133 | 991219 | success |
| 5 | `search:trials=4:seed=42:budget=official` | 3 | 9 | 127 | 136080 | success |
| 6 | `search:trials=8:seed=42:budget=official:park=far` | 3 | 9 | 127 | 220810 | success |
| 7 | `search:trials=8:seed=42:budget=official` | 3 | 9 | 127 | 268893 | success |
| 8 | `coverage:budget=official:park=far` | 3 | 9 | 124 | 28188 | success |

### `generated:v2:open:seed=29:days=3`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 6 | 54 | 310185 | success |
| 2 | `search:trials=16:seed=42:budget=official` | 2 | 6 | 54 | 428777 | success |
| 3 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 6 | 54 | 577178 | success |
| 4 | `search:trials=32:seed=42:budget=official` | 2 | 6 | 54 | 866329 | success |
| 5 | `coverage:budget=official:park=far` | 2 | 6 | 53 | 34294 | success |
| 6 | `search:trials=4:seed=42:budget=official` | 2 | 6 | 53 | 65815 | success |
| 7 | `search:trials=8:seed=42:budget=official` | 2 | 6 | 53 | 144686 | success |
| 8 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 6 | 53 | 245553 | success |

### `generated:v2:many-series:seed=3`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=32:seed=42:budget=official:park=far` | 9 | 27 | 110 | 1049926 | success |
| 2 | `search:trials=32:seed=42:budget=official` | 9 | 27 | 110 | 1531279 | success |
| 3 | `search:trials=16:seed=42:budget=official` | 9 | 27 | 107 | 640360 | success |
| 4 | `search:trials=16:seed=42:budget=official:park=far` | 9 | 27 | 107 | 652693 | success |
| 5 | `search:trials=4:seed=42:budget=official` | 9 | 27 | 104 | 203338 | success |
| 6 | `search:trials=8:seed=42:budget=official:park=far` | 9 | 27 | 104 | 331730 | success |
| 7 | `search:trials=8:seed=42:budget=official` | 9 | 27 | 104 | 384586 | success |
| 8 | `coverage:budget=official:park=far` | 9 | 27 | 101 | 29672 | success |

### `generated:v2:open:seed=1:days=3:fuel=tight`

| rank | candidate | variety | daily sum | total balls | response µs | status |
|---:|---|---:|---:|---:|---:|---|
| 1 | `search:trials=4:seed=42:budget=official` | 2 | 6 | 76 | 159082 | success |
| 2 | `search:trials=8:seed=42:budget=official:park=far` | 2 | 6 | 76 | 289835 | success |
| 3 | `search:trials=8:seed=42:budget=official` | 2 | 6 | 76 | 331202 | success |
| 4 | `search:trials=16:seed=42:budget=official` | 2 | 6 | 76 | 555408 | success |
| 5 | `search:trials=16:seed=42:budget=official:park=far` | 2 | 6 | 76 | 643772 | success |
| 6 | `search:trials=32:seed=42:budget=official` | 2 | 6 | 76 | 1134344 | success |
| 7 | `search:trials=32:seed=42:budget=official:park=far` | 2 | 6 | 76 | 1315357 | success |
| 8 | `coverage:budget=official:park=far` | 2 | 6 | 71 | 29852 | success |
