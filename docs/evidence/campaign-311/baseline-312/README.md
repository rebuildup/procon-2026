# #312 初動 baseline

公式予算 (`budget=official`) で14盤×4候補を同条件比較したbaseline証跡。固定9盤、arena701、由来未確定のimport盤、generated3盤。全112件scored、invalid/timeout/panicなし。

- 採点batch (`ps/` `pssps/` の `runs.jsonl`): source `6bb166b99b227a5a3d20558637c3d3c067586db6`、debug profile。回答時間をrelease本番性能と解釈しない。
- 日別取得種類集合・停止原因の診断 (`*/diagnostics.jsonl`) とrelease batch (`release-ps/`): source `bcfa0be46816178ba4d1d0b97f4b6a662b81e4bd`。`6bb166b` からの差分は診断ツール (`campaign_diagnose.rs`) と `scripts/campaign/baseline.py` のみで、solver/engineは不変。診断の公式4段scoreは全112件で `runs.jsonl` と一致（0件不一致）を確認済み。

701 / P,S: grid2x2=20/119/173、kmeans5=20/158/207、kmeans8=20/148/200、loop128=18/139/189。kmeans5は701で優位だが、全14盤ではgridに5勝3分6敗で全盤優位ではない。

701 / P,S,S,P,S: grid2x2=17/90/119、kmeans5=20/144/169。車種が異なるbatch間の数字を候補改善として比較しない。

追加importは `fixture:arena-20261008T143339Z`（10×10、4日、3台、fuel20）。match696ではない。match696の14/70/320は既存Issueの相手実績のみで、現条件で再現できていない。

## ファイル構成

| path | 内容 |
|---|---|
| `ps/` | kinds `P,S` のdebug batch。`runs.jsonl` 56件 / `aggregate.json` / `environment.json` |
| `ps/diagnostics.jsonl` | 同compositionの日別取得種類集合＋停止原因（4候補×14盤） |
| `pssps/` | kinds `P,S,S,P,S` のdebug batch と diagnostics |
| `release-ps/` | kinds `P,S` のrelease batch。本番相当のresponse_time |
| `import-live-match.json` | 追加import盤 `fixture:arena-20261008T143339Z` |

各 `environment.json` / `diagnostics-environment.json` にsource SHA・コマンド・profile・response-time方針を記録。`diagnostics.jsonl` は `kind:"day"`（1日1行）と `kind:"case_summary"`（case末尾1行）のJSON Lines。解析にsolver内部の近似スコアは使わない。

## 同一条件の確認

- 4候補は同一corpus・同一kinds・同一source SHAで比較。kind比率が違うbatchは `ps` / `pssps` に分離し、各runの `agent_kinds` に実際の編成を記録。release batchも `release-ps/` に分離し、`environment.json` の `response_time_policy` に計測方針を明示。
- `variety` / `daily_variety_sum` / `total_balls` は `runs.jsonl`、失点原因は `diagnostics.jsonl` の `stop_reasons` と `evidence` でcase別に確認できる。

## 第1・第2軸の取りこぼし事例（原因別、P,S）

1. **`fixture:701` / grid2x2** — 第1軸は達成（20/20）だが第2軸が119でkmeans5(158)に劣る。day0で全20系列取得済みのためday1-9は全日 `repeat_only`。日別取得数はgrid `[20,18,12,20,9,8,6,12,3,11]` 対 kmeans5 `[20,20,10,20,17,17,11,17,8,18]`。**カバレッジ後の第2軸差は1日ごとの再取得系列数（再巡回密度）**であり、新規系列探索ではない。
2. **`fixture:701` / loop128** — 系列3,17を最後まで未取得（`series_unreached`）。agent2(Patrol)はday1-2に燃料0（`fuel_exhausted`）、day2-3は移動0（`vehicle_stalled`）、day1-9は既出系列のみ（`series_repeats`）。補給ペア漏れでPatrolが枯死する既知パターン（#252系）。
3. **`fixture:road-heavy` / grid2x2** — 系列2を未取得（1/2）。day0でagent2が移動0・待機32（`vehicle_stalled`）。割当車両が出発しないカバレッジ欠落。
4. **`fixture:series-many` / grid2x2** — 系列1-3を未取得（3/6）。agent0が移動0・待機24（`vehicle_stalled`）。loop128はさらに1/6。
5. **`fixture:arena-20261008T143339Z` / grid2x2** — 系列3を未取得（2/3）。agent2はday0の燃料残り1のままday1-3が移動0（`vehicle_stalled`）。day1-3は `repeat_only`。
6. **`fixture:road-heavy-v2` / grid2x2** — 第1軸達成（3/3）だが第2軸が5でkmeans5(7)に劣る。day1-2は `repeat_only` かつ再取得は1系列/日のみ。

5車種構成 (`pssps`) では同一盤でも結果が変わる（`fixture:701` / grid2x2 = 17/90/119、series 7,8,16未取得、day8は5台中3台が移動0）。**kind比率が違うbatch間のスコア差を候補差として読まない。**

## 停止原因の分類

`case_summary.stop_reasons`（`complete` は他と排他）:

| reason | 意味 | 確認先 |
|---|---|---|
| `complete` | 盤面の全系列を取得 | `missing_series` が空 |
| `series_unreached` | 未取得系列が残る（未取得系列） | `missing_series` |
| `fuel_exhausted` | 燃料切れ動作、または燃料0で終えたagent（燃料不足） | `evidence.fuel_exhausted_agent_days` |
| `vehicle_stalled` | 未取得系列がある日に移動0のagent（車両停止） | `evidence.stalled_agent_days` |
| `series_repeats` | 既出系列しか取れない日がある（重複） | `evidence.repeat_only_days` |
| `plan_normalized` | 提出前normalizeが計画を書き換えた（ステップ切詰め） | `evidence.normalized_action_changes_total` |

分布: `ps` = complete 43 / series_unreached 13 / vehicle_stalled 12 / series_repeats 5 / fuel_exhausted 2。`pssps` = complete 36 / series_unreached 20 / series_repeats 8 / vehicle_stalled 5。

日別行は `acquired_series`（当日取得集合）/ `repeat_series`（当日のうち既出）/ `missing_series`（当日未取得）/ `unreached_series`（当日終了時点の未取得累計）とper-agentの移動・待機・燃料を返す。

## 出発セル地形課金と提出前normalizeの整合

全112 replay（`ps`+`pssps`）の全dayで `normalized_action_changes` = 0。`budget=official` の計画は提出前normalizeに対してidempotentで、#255/#302/#304/PR #303系統の出発セル地形課金（当該branchの `f9087b2` / `3256a55` / `ba94196`）と矛盾しない。このticketでは重複修正を行わない。

## 再現コマンド

```bash
# debug batch（既存outputは上書きせず、必ず新規dirへ）
python3 scripts/campaign/baseline.py --kinds P,S --output-dir target/baseline-ps
python3 scripts/campaign/baseline.py --kinds P,S,S,P,S --output-dir target/baseline-pssps

# release batch（本番相当のresponse_time）
python3 scripts/campaign/baseline.py --profile release --kinds P,S --output-dir target/baseline-release

# 日別取得種類集合＋停止原因のみ
python3 scripts/campaign/baseline.py --skip-batch --diagnose --kinds P,S --output-dir target/diagnose-ps

# 単一caseの診断（CASE_SOURCE は fixture:<name> / generated:<sel> / CaseSpec JSON path）
cargo run --offline --locked -p bench --example campaign_diagnose -- \
  fixture:701 'multiloop:grid=2x2:walk=astar:assign=all:budget=official' 'P,S'
```

`baseline.py` は `runs.jsonl` / `diagnostics.jsonl` の既存を検出したら停止する。既存のevidence dirにbatchを再投入しないこと。

## 完了 / 未完

完了（このPRまで）:

- 固定fixture＋generated＋import盤の同条件baselineが再実行できる（コマンド・環境・source SHA記録済み）。
- kind比率・評価クロックが違うbatchの分離と明示。
- case別の `variety` / `daily_variety_sum` / `total_balls` と失点原因（`stop_reasons` ＋日別取得種類集合）。
- 第1・第2軸が満たされない理由を原因別に説明できる事例6件（上記）。
- 公式出発セル地形課金と提出前normalizeの整合確認（idempotent、重複修正なし）。
- 全数字にsource SHA・コマンド・evidence保存先を付記。
- release profile batch 1件（`release-ps/`）で本番相当response_timeを計測（701 grid=245ms、kmeans5=158ms、kmeans8=192ms、loop128=202ms）。

未完（他ticketの出力。ここでは実装しない）:

- holdout評価・新候補比較・live parity — #313〜#317、#331 の成果待ち（依存）。
- match696相当盤での `14/70/320` 再現（相手実績のみで現条件未達）。
