# `search` の `park=far` 比較（2026-10-09）

- Issue / PR: #340 / #341
- 評価した source commit: `551f847eecff2bd67a4e8cf483202f3014c7ed34`
- 公式 candidate: `coverage:budget=official:park=far`
- 試した探索候補: `search:trials=4|8|16|32:seed=42:budget=official` と `search:trials=8|16|32:seed=42:budget=official:park=far`
- 全候補で `--kinds 'P,S,P,S,P'`、同じケース集合、`budget=official` を使用。外部 arena は使っていない。

## 結果

現行候補を置き換える新 candidate は選べなかった。`park=far` を search の初期計画と各 trial に引き継ぐと、search の既知回帰は縮まったが、最良 coverage 候補の日別種類網羅を維持できず、未使用seedにも一般化しなかった。探索はここで止めた。

### Screening

全9 fixture + fixture:701 + 4 generated cases の14盤を、8 candidate 同時に評価した（112 run）。各 search は1日の計画あたり最大32 trial、評価器は4並列、全体の上限は900秒。14盤の score tuple の各軸合計と平均実行時間は記述統計であり、公式順位を合算して定義するものではない。W/D/L はケースごとの辞書順比較だが、`--jobs 4` では第4軸の応答時間が不安定なため参考値とする。

| candidate | W/D/L vs coverage far | variety 合計 | daily variety 合計 | balls 合計 | 平均 response | 平均 elapsed |
|---|---:|---:|---:|---:|---:|---:|
| `coverage:budget=official:park=far` | — | 60 | 276 | 797 | 59.6 ms | 67.5 ms |
| `search:trials=4:seed=42:budget=official` | 7/0/7 | 60 | 271 | 818 | 212.2 ms | 217.8 ms |
| `search:trials=8:seed=42:budget=official` | 7/0/7 | 60 | 272 | 835 | 507.6 ms | 517.5 ms |
| `search:trials=16:seed=42:budget=official` | 8/0/6 | 60 | 273 | 857 | 976.6 ms | 983.3 ms |
| `search:trials=32:seed=42:budget=official` | 9/0/5 | 60 | 270 | 860 | 1,673.4 ms | 1,679.2 ms |
| `search:trials=8:seed=42:budget=official:park=far` | 8/0/6 | 60 | 269 | 811 | 458.7 ms | 465.8 ms |
| `search:trials=16:seed=42:budget=official:park=far` | 8/0/6 | 60 | 274 | 847 | 956.6 ms | 963.2 ms |
| `search:trials=32:seed=42:budget=official:park=far` | 9/0/5 | 60 | 274 | 858 | 1,597.9 ms | 1,607.0 ms |

`search` with `trials=32, park=far` increased corpus balls by 61 over the baseline, but daily variety fell by 2. On fixture:701 it scored `20/196/377` versus `20/198/358`; the lost day was day 8 (18 instead of 20 series). The `park=far` option therefore reduced the prior `park=none` regression (192/200 on fixture:701), but did not remove it.

Screening finished in 22.824 seconds. All 112 records were scored `success`; invalid actions, fuel shortages, day-budget exceedances, timeouts, and panics were zero.

### Sequential confirmation

The baseline, `search trials=16 park=far`, and `search trials=32 park=far` were rerun with `--jobs 1` on the 9 fixtures, fixture:701, and 5 previously unused generated cases (45 runs). This is the ranking set for response time.

| candidate | W/D/L vs baseline | variety total | daily variety total | balls total | mean response | mean elapsed |
|---|---:|---:|---:|---:|---:|---:|
| `coverage:budget=official:park=far` | — | 73 | 305 | 740 | 46.5 ms | 50.6 ms |
| `search:trials=16:seed=42:budget=official:park=far` | 5/0/10 | 73 | 303 | 770 | 633.5 ms | 637.3 ms |
| `search:trials=32:seed=42:budget=official:park=far` | 6/0/9 | 73 | 303 | 773 | 1,319.5 ms | 1,324.2 ms |

On the five unseen generated cases, both search candidates scored 1 win and 4 losses. Fixture:701 remained `20/196/376` at 16 trials and `20/196/377` at 32 trials, versus the baseline `20/198/358`. The daily-series deficit was on day 8 for both. There were 45 successful scored runs and zero invalid actions, fuel shortages, day-budget exceedances, timeouts, or panics.

Confirmation finished in 30.205 seconds. Search increased balls but lowered an earlier score axis, lost more cases than it won in sequential comparison, and took substantially longer. Keep `coverage:budget=official:park=far` as the current recommendation; do not promote the search variants from this evidence.

## Reproduction

At source commit `551f847eecff2bd67a4e8cf483202f3014c7ed34`, build the evaluator with:

```bash
cargo build --release --offline --locked -p bench --bin procon-eval
```

Screening command (same 112 records):

```bash
timeout --kill-after=5s 900s env PROCON_SOURCE_COMMIT_SHA=551f847eecff2bd67a4e8cf483202f3014c7ed34 \
  ./target/release/procon-eval \
    --cases all \
    --case-files crates/bench/tests/fixtures/arena-701.json \
    --generated 'open:seed=17:days=3,open:seed=29:days=3,many-series:seed=3:days=3,open:seed=1:days=3:fuel=tight' \
    --kinds 'P,S,P,S,P' \
    --candidates 'coverage:budget=official:park=far,search:trials=4:seed=42:budget=official,search:trials=8:seed=42:budget=official,search:trials=16:seed=42:budget=official,search:trials=32:seed=42:budget=official,search:trials=8:seed=42:budget=official:park=far,search:trials=16:seed=42:budget=official:park=far,search:trials=32:seed=42:budget=official:park=far' \
    --baseline 'coverage:budget=official:park=far' \
    --jobs 4 \
    --output-dir target/eval-search-park-issue-340-screen \
    --markdown-out target/eval-search-park-issue-340-screen/report.md \
    --quiet
```

Sequential confirmation command (same 45 records):

```bash
timeout --kill-after=5s 300s env PROCON_SOURCE_COMMIT_SHA=551f847eecff2bd67a4e8cf483202f3014c7ed34 \
  ./target/release/procon-eval \
    --cases all \
    --case-files crates/bench/tests/fixtures/arena-701.json \
    --generated 'open:seed=31:days=3,road-heavy:seed=19,mountain-heavy:seed=23:size=small,many-series:seed=19,pond-obstacle:seed=11:fuel=tight' \
    --kinds 'P,S,P,S,P' \
    --candidates 'coverage:budget=official:park=far,search:trials=16:seed=42:budget=official:park=far,search:trials=32:seed=42:budget=official:park=far' \
    --baseline 'coverage:budget=official:park=far' \
    --jobs 1 \
    --output-dir target/eval-search-park-issue-340-confirmation \
    --markdown-out target/eval-search-park-issue-340-confirmation/report.md \
    --quiet
```

### Evidence layout and resume

`screening/` and `confirmation/` contain the raw `runs.jsonl`, aggregate JSON, generated report, evaluated source SHA, and wall-time log. If a future code or scoring hypothesis changes this result, start a new evaluation from its own commit SHA and use a disjoint explicit generated seed set; keep candidate conditions identical and re-run the canonical evaluator. Do not merge the current Draft PR as a winning solver candidate based on these results.
