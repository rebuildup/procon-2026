# eval-377 — 競争力評価基盤の刷新（Issue #377 / PR #378）

KPI を「anytime との内部勝率」から「競争相手 B との公式辞書順比較」へ
刷新するための土台。このディレクトリは **fleet `5ddb153` の baseline
証跡**であり、#374 / #375 / #376 の各候補はこの形式で計測して比較する。

## 構成

| ファイル | 内容 |
|---|---|
| [`HOLDOUT-SEEDS.md`](HOLDOUT-SEEDS.md) | seed 系列の台帳と「採点専用 holdout を調整に使わない」運用規則 |
| [`DAY-PATH-RECORD.md`](DAY-PATH-RECORD.md) | 「どの日・どの経路が・なぜ」の記録形式（候補作者が添付する） |
| `gen_corpus.py` | 調整 24 面（seed 1001..1024）の再生成。fleet-371 から無改変コピー |
| `gen_holdout2.py` | 新 holdout（`hold2` 61001..61024）と最悪ケース（`hard2` 62001..62012）の生成 |
| `corpus/` | 調整 24 面（再生成・fleet-371 の `corpus/` とバイト一致） |
| `corpus-holdout2/` | **新 holdout-24**。seed 系列としては一度も開発に使われていない |
| `corpus-hard-extreme/` | **最悪ケース 12 面**。全日が歩数下限 `W+H`、燃料 1.0 倍（hard-12 より厳しい） |
| `kinds_auto_check.py` | `--kinds-auto` 選択規則のオフライン再現（盤面 dir を引数に取れるよう改変） |
| `runs/<バッチ名>/` | `runs/<config>/runs.jsonl` ＋ `batch.json` ＋ `compare.json` |
| `scripts/campaign/eval_compare.py` | 比較 entry point（次節） |

## 比較 entry point

2+ 候補 config（candidate id ＋ kinds パターン）を 1 コマンドで同じ盤面
セットに流し、公式辞書順 V→D→B→応答で判定する。

```bash
cargo build --release --locked -p bench
python3 scripts/campaign/eval_compare.py \
  --name holdout2-<your-candidate> \
  --case-files docs/evidence/campaign-311/eval-377/corpus-holdout2 \
  --config 'fleet:budget=official:iters=20000:chains=4|kinds=S,P,P,P,P,P,P,P' \
  --config '<your-candidate>|kinds=S,P,P,P,P,P,P,P' \
  --reference 'fleet:budget=official:iters=20000:chains=4' \
  --jobs 4 \
  --out docs/evidence/campaign-311/eval-377/runs/holdout2-<your-candidate>
```

- 出力: `runs.jsonl`（procon-eval そのもの）と `compare.json`
  （盤面ごとの V/D/B/応答・日別玉数 `daily_ball_counts`・日別受理
  `rejected_days`／`day_budget_exceeded_days`・不正カウンタ・辞書順勝敗）。
- 中断再開可能（`runs.jsonl` が完成している config はスキップ）。
- 各 config の実行コマンド・SHA・盤面リストは `batch.json` に記録。
- `:iters=` / `:trials=` の config は完全決定論。`:budget_ms=` は
  CPU 負荷でぶれるので、応答時間を軸 4 で論じるには `--jobs 1` で
  専用バッチを切ること（下記 sequential 節）。
- 難条件軸: `--case-files` を `corpus-hard-extreme`（燃料 1.0 倍＋全日
  歩数下限）に変える。fuel-tight 単独軸は `procon-eval --generated
  '...:fuel=tight'` が既存の軸。

## 盤面セット

| セット | 盤面数 | seed | 性格 |
|---|---:|---|---|
| `corpus/` ＋ 701 | 25 | 1001..1024 | 調整に使った 24 面＋実戦盤面 701。**調整専用**（採点根拠に単独で使わない） |
| `fleet-371/corpus-holdout` | 24 | 5001..5024 | 旧 holdout。fleet 開発の hard D 判断に使われたため**消費済み**。継続性確認用 |
| `corpus-holdout2/` | 24 | **61001..61024** | **本 Issue の新 holdout**。どの候補開発でも未使用。候補採否はここで判定する |
| `fleet-371/corpus-hard` | 12 | 9001..9012 | hard-12。全日下限付近（1.15 倍まで）、燃料 1.0 倍。部分消費済み |
| `corpus-hard-extreme/` | 12 | **62001..62012** | **最悪ケース**。全日 `W+H`（下限ちょうど）・燃料 1.0 倍。未使用 |

## baseline 計測（fleet `5ddb153`）

- 計測バイナリ: `b718e2f`（`5ddb153` に Issue #377 の診断軸
  `daily_ball_counts` / `rejected_days` を足しただけの差分で、
  `crates/` の candidate 挙動は `5ddb153` と同一。記録の
  `source_commit_sha` は走らせた時点の `8392451`）。
- config（candidate id は canonical param 形式で固定）:

| ラベル | candidate id | kinds |
|---|---|---|
| `fleet-live2s` | `fleet:budget=official:budget_ms=2000:adapt=on` | `S,P,P,P,P,P,P,P` |
| `fleet-chains4` | `fleet:budget=official:iters=20000:chains=4` | `S,P,P,P,P,P,P,P` |
| `fleet-PS-chains4` | `fleet:budget=official:iters=20000:chains=4` | `P,S` |
| `anytime-PS` | `anytime:trials=64:seed=42:budget=official` | `P,S` |

- 同一 kinds 比較は `fleet-chains4` vs `fleet-PS-chains4`（kinds を
  実験変数にしない原則の両側）と、`anytime-PS` との比較で確認する。
- 実行: `python3 scripts/campaign/eval_compare.py --name <set>-fleet-baseline
  --case-files <setのdir> --config …上記4件… --reference
  'anytime:trials=64:seed=42:budget=official' --jobs 4 --out runs/<set>-fleet-baseline`

## 結果

### holdout2（seed 61001..61024, 24 面）— baseline 計測

`runs/holdout2-fleet-baseline/compare.json`（reference: `anytime-PS`）。

| ラベル | vs anytime-PS（辞書順） | V 計 | D 計 | B 計 | invalid |
|---|---|---:|---:|---:|---:|
| `fleet-live2s` | W24 D0 L0 | 480 | 3662 | 9363 | 0 |
| `fleet-chains4` | W24 D0 L0 | 480 | 3661 | 9268 | 0 |
| `fleet-PS-chains4` | W23 D0 L1 | 480 | 3650 | 6707 | 0 |
| `anytime-PS`（reference） | — | 480 | 3620 | 5778 | 0 |

- **kinds を実験変数にしない両側**: `fleet-chains4`（S,P×8）vs
  `fleet-PS-chains4`（P,S）→ S,P×8 が D +11 / B +2561 / 1敗未満
  （P,S のみ1敗）。以後の候補比較は基本 `S,P,P,P,P,P,P,P` で行う。
- `fleet-chains4` vs `fleet-live2s`: D −1 / B −107。`budget_ms` 行は
  負荷感受（非決定論）なので、この差を採否根拠には使わない。
- 不正カウンタは全 config 全面 0（受理失敗・fuel_short・illegal なし）。
- ラベルは kinds 込み（`config_label` が `id|kinds` 全体を正規化）。
  同一 candidate id に異なる kinds を付けると衝突検出で異常終了する。

## 既知の限界

- `budget_ms` の行は CPU 負荷（本計測中も並行 agent が重い探索を回して
  いた）で数球ぶれる。決定論 config（`iters=`）の V/D/B は負荷の影響を
  受けない。
- 応答時間は並列実行（`--jobs 4`）で測ったため軸 4 の比較には使わない
  こと（sequential 再計測は下記「残っている計測」）。
- 合成盤面であり、他チームの交通量を含まない（fleet-371 と同じ限界）。

## 残っている計測（queued）

- sequential 再計測（`--jobs 1`）: 応答時間を軸 4 の証拠にするための
  `fleet-live2s` 専用バッチ。
- `corpus-hard-extreme`（燃料 1.0 倍＋全日下限 12 面）での baseline。
- #374 / #375 / #376 の各候補を同一ラベル形式で holdout2 に流す比較。
