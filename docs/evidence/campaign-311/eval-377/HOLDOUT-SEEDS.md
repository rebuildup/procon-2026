# ホールドアウト seed 運用規則（Issue #377）

## 目的

開発中のパラメータ調整に使った seed で作った盤面で候補を採点すると、
その seed への過学習が「競争力」の見かけを作る。そこで seed 系列を
「開発に使ってよい系列」と「採点専用の系列」に分け、採点専用系列を
このリポジトリに固定する。

## seed 系列の台帳

| 系列 (seed) | 使われ方 | 状態 |
|---|---|---|
| `0..239` | `gen_fuzz.py`（不規則盤 240 面、堅牢性ラン） | 開発・消費済み |
| `1..8`, `101..108` | `--generated` 軸・`scripts/longrun_eval.py` の探索/確認 seed | 開発・消費済み |
| `1, 2, 3, 4, 5, 7, 17, 29, 42` | バンドル fixture・候補の探索 seed | 開発・消費済み |
| `1001..1024` | `gen_corpus.py tune`（調整 24 面＋ 701） | **調整専用**。採点に使わない |
| `5001..5024` | `gen_corpus.py holdout`（旧 holdout-24） | **消費済み**。fleet 開発の hard D 判断に使われたため、完全な未使用セットではない |
| `9001..9012` | `gen_corpus.py hard`（hard-12） | **部分消費済み**（高温連鎖判断） |
| `61001..61024` | `eval-377/gen_holdout2.py hold2`（新 holdout-24） | **採点専用** |
| `62001..62012` | `eval-377/gen_holdout2.py hard2`（最悪ケース 12 面） | **採点専用** |

## 採点専用 seed の扱い（運用規則）

1. `corpus-holdout2/` と `corpus-hard-extreme/` の盤面では、
   candidate のパラメータ・温度・重み・kinds 選択規則を**変更しない**。
   生成後の初回実行が調整に相当する。変更が必要なら seed 系列を新しく
   掘る（次の空きは `63000` 台。`SEED_BASE` に追加して台帳に書く）。
2. 採点専用盤面の run は `docs/evidence/campaign-311/eval-377/runs/`
   に置き、どの candidate がいつ触ったかを追跡できるようにする。
3. 候補選抜（#317）の判断は、調整 24＋701 と旧 holdout の結果だけでは
   採否しない。必ず新 holdout（`hold2`）の結果を併記する。
4. `gen_holdout2.py` は決定論的（seed 固定）。盤面 JSON はこの
   ディレクトリにバイト一致で commit 済みで、再生成すると同一になる。

## 検証

- `hold2` / `hard2` の case_id は `fixture:hold2-*` / `fixture:hard2-*`。
- 再生成: `python3 -I docs/evidence/campaign-311/eval-377/gen_holdout2.py <dir> hold2|hard2`
