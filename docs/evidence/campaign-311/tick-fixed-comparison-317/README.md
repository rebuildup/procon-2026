# tick 修正後 候補順位表 — 同一13盤・4候補

source: `0eb3a9f6e91ada406f6d1cff4d65e3c60c9fe7b9`（refuel tick 修正 `8728ed5` を含む）。`procon-eval` の canonical MatchSim で、13 盤（bundled 9 fixture + `fixture:701` + generated 3）× 4 候補 = 52 run。全 run success、timeout / invalid / panic / illegal action / fuel short は 0。

公式4段scoreは `(variety, daily_variety_sum, total_balls, -response_time)` の辞書順。win/draw/lose は baseline `multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official` 比（ms 単位の response time で同点判定）。数値の一次データは `runs.jsonl` / `aggregate.json`。

## 順位表（同一caseセット・52 run）

| 順 | candidate | vs baseline | 総当たり W/D/L | 平均順位 / 1位回数 | V mean | D mean | B mean | response mean / max | 最悪ケース (V/D/B) | 安定性 |
|---|---|---|---|---|---:|---:|---:|---|---|---|
| 1 | `search:trials=16:budget=official` | 12勝0分1敗 | 30勝0分9敗 | 1.69 / 6 | 4.92 | 21.92 | 53.9 | 173 ms / 1.74 s | series-few (1/1/11) | timeout 0 |
| 2 | `search:trials=64:budget=official` | 12勝0分1敗 | 24勝0分15敗 | 2.15 / 3 | 4.92 | 22.08 | 55.2 | 686 ms / 6.89 s | series-few (1/1/11) | timeout 0 |
| 3 | `coverage:budget=official` | 12勝1分0敗 | 21勝1分17敗 | 2.31 / 4 | 4.92 | 21.69 | 52.2 | 11 ms / 108 ms | series-few (1/1/9) | timeout 0 |
| 4 | `multiloop:kmeans=5:seed=42`（baseline） | — | 2勝1分36敗 | 3.77 / 1 | 4.77 | 17.38 | 32.0 | 6.4 ms / 65 ms | road-heavy (1/1/1) | timeout 0 |

総当たりは3対戦相手×13盤=39比較の集計。平均順位・1位回数は `aggregate.json` の `per_case` と同じ公式辞書順。

総当たり（辞書順、13 盤ずつ）:

| 対戦 | 結果 |
|---|---|
| coverage vs kmeans5 | 12勝1分0敗 |
| search:16 vs kmeans5 | 12勝0分1敗（openapi-sample は前3軸同点・response time 軸で敗） |
| search:64 vs kmeans5 | 12勝0分1敗（同上） |
| search:16 vs coverage | 8勝5敗 |
| search:64 vs coverage | 9勝4敗 |
| search:16 vs search:64 | 10勝3敗 |

search 2 候補は 3 軸目の玉数で coverage を押し負けることが多く、coverage は応答時間で 1〜2 桁速い。第4軸 `-response_time` が効くのは前3軸同点のときのみ。

## 701（実戦盤・10 日・P,S,P,S,P）

| candidate | variety | daily_variety_sum | total_balls | response 合計 | 日別種類 |
|---|---:|---:|---:|---|---|
| `search:trials=64:budget=official` | 20 | **197** | **395** | 6890 ms | 20,20,20,20,20,20,19,20,18,20 |
| `search:trials=16:budget=official` | 20 | 195 | 386 | 1737 ms | 20,20,19,20,20,20,17,20,19,20 |
| `coverage:budget=official` | 20 | 194 | 377 | 108 ms | 20,20,18,20,20,20,19,20,17,20 |
| `multiloop:kmeans=5:seed=42` | 20 | 156 | 194 | 65 ms | 20,20,10,20,17,17,10,17,8,17 |

## 制約（この表が言えないこと）

- **winner 認定ではない**。同一 corpus は campaign 中に触ってきた盤を含み、Issue #317 の求める未使用 seed / 未使用実戦盤の holdout 評価は未実施。
- response time は計測環境の実測値。**計測 profile（debug/release）は本 artifact に記録されていない**。release 本番性能として読まない。絶対値ではなく候補間の相対順序の根拠にとどめる。
- canonical MatchSim の時計は要素単位で、公式サーバーの tick 受理スコアと同一視しない。公式バイナリでの受理検証は `../official-cli-317-coverage/` `../official-cli-317-search-seven/`、tick 修正の受理検証は `../official-tick-317/` を参照。
- 実外部 arena での順位確認ではない。

## 再現

```bash
cargo build --release -p bench --bin procon-eval --offline --locked
PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) ./target/release/procon-eval \
  --cases all \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --generated 'open:seed=17:days=3,road-heavy:seed=17:fuel=tight,many-series:seed=29' \
  --candidates 'multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official,coverage:budget=official,search:trials=16:budget=official,search:trials=64:budget=official' \
  --baseline 'multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official' \
  --output-dir target/eval-317-tick-fixed
```

既存 output を上書きしない（`--output-dir` は新規ディレクトリ）。本番用時間計測は同じコマンドを release build で行い `environment.json` 相当のメタとセットで残す。起動・切り替え手順は `LIVE-RUNBOOK.md`。

未完: holdout、release 計測、実外部 arena、最終 winner 認定。#317 の全受け入れ条件完了を意味しない。
