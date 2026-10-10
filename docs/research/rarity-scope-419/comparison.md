# rarity_scope=initial|both のペアワイズ測定結果（#419）

`docs/research/rarity-scope-419/README.md` の計画に基づく測定結果。3候補を hard12（12盤面）と holdout24（24盤面）で canonical evaluator により同一source・同一順でペアワイズ比較した。

## 結論（先出し）

- **A vs C（`rarity_scope=initial` vs `rarity=0`）: スコア完全一致**。36盤面すべてで V/D/B が等しく、最終計画（`agent_ball_counts` + `daily_varieties`）もバイト同等。勝敗は回答時間の揺れのみ。
- **A vs B（`rarity_scope=both`）: 辞書順で負け**。hard12 で ΔB+7 / holdout24 で ΔB+8（B が 8〜7 ボール多い）、ΔD も −3/−2。スコア差が出た 28 盤面のうち B が勝つのは 13、A が勝つのは 15。
- **判定: negative**。scope 分離（14f88ac）の不変性主張は実証されたが、`initial` は A と測定上まったく同じであり、`both` は親（#417/#418, d87c66d）と同様に辞書順でベースラインに劣る。採用理由にならない。

## 測定条件

- source: `/Users/rebui/work/procon-candidate-419` branch `419` @ `14f88ac828615bc52bf079f11e929c6d339ae18d`（`PROCON_SOURCE_COMMIT_SHA` を設定。全レコードの `source_commit_sha` も同じ値）
- binary: `cargo build --release --locked -p bench --bin procon-eval`（2026-10-10 19:22 JST ビルド、22s）
- corpus:
  - hard12: `docs/evidence/campaign-311/fleet-371/corpus-hard/` 12ファイル（`hard-16-a3-d9-short-s9003.json` … `hard-32-a8-d7-short-s9012.json`、seed s9001..s9012）
  - holdout24: `docs/evidence/campaign-311/fleet-371/corpus-holdout/` 24ファイル（`hold-16-a3-d7-mixed-s5007.json` … `hold-32-a8-d7-short-s5024.json`、seed s5001..s5024）
- 候補（3つとも `fleet:budget=official:iters=20000:chains=4:seed=42` 共通）:
  - **A-rarity0**: `rarity=0`
  - **B-both**: `rarity=2:rarity_scope=both`（親動作）
  - **C-initial**: `rarity=2:rarity_scope=initial`（seed-only）
- 車種: `--kinds 'S,P,P,P,P,P,P,P'`、`--jobs 1`、実行は hard12 → holdout24 の順で直列。
- 日時: 2026-10-10 19:24〜19:43 JST（hard12 2m32s、holdout24 8m10s）。実行中の負荷: load1 約19〜36（他セッションと共用、バッチは直列なので候補間の負荷条件は同一）。

### 再現コマンド

```sh
cd /Users/rebui/work/procon-candidate-419
export PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD)
HARD=$(ls /Users/rebui/work/procon-2026/docs/evidence/campaign-311/fleet-371/corpus-hard/*.json | sort | tr '\n' ',' | sed 's/,$//')
HOLD=$(ls /Users/rebui/work/procon-2026/docs/evidence/campaign-311/fleet-371/corpus-holdout/*.json | sort | tr '\n' ',' | sed 's/,$//')
CANDS='fleet:budget=official:iters=20000:chains=4:seed=42:rarity=0,fleet:budget=official:iters=20000:chains=4:seed=42:rarity=2:rarity_scope=both,fleet:budget=official:iters=20000:chains=4:seed=42:rarity=2:rarity_scope=initial'
BASE='fleet:budget=official:iters=20000:chains=4:seed=42:rarity=0'
./target/release/procon-eval --case-files "$HARD"  --candidates "$CANDS" --baseline "$BASE" --kinds 'S,P,P,P,P,P,P,P' --jobs 1 --output-dir /tmp/rarity-scope-hard12    --quiet
./target/release/procon-eval --case-files "$HOLD" --candidates "$CANDS" --baseline "$BASE" --kinds 'S,P,P,P,P,P,P,P' --jobs 1 --output-dir /tmp/rarity-scope-holdout24 --quiet
```

注: `--case-files` はディレクトリを受け付けないため、ファイルの明示リストを渡す。

## W/T/L（公式辞書順 V→D→B→−T、タイ時は回答時間で決着、それでも同点は T）

方向の根拠: aggregate.json の全決着盤面（hard12: 4、holdout24: 9）で V/D 同点時の rank1 は `total_balls` が大きい側 → B は大きい方が良い。下の表は全盤面について本ツールの rank 順と突き合わせて検証済み（不一致 0。hold-24-a4-d10-mixed-s5015 の A vs C のみスコア・回答時間とも実質同点の T）。

| 比較 | hard12 (12盤面) | holdout24 (24盤面) | Δ合計 (holdout24, B−A / C−A) |
|---|---|---|---|
| A vs B | A 8W / 0T / 4L | A 11W / 0T / 13L | ΔV 0 / ΔD −2 / ΔB +8（B が劣る） |
| A vs C | 4W / 0T / 8L ※時間差のみ | 12W / 0T / 12L ※時間差のみ | ΔV 0 / ΔD 0 / ΔB 0（スコア完全一致） |
| B vs C | B 4W / 0T / 8L | B 11W / 0T / 13L | ΔV 0 / ΔD +2 / ΔB −8 |

※ A vs C は 36盤面すべて V/D/B が等しいため、表の W/L は回答時間の揺れだけで決まっている。スコアとしての実質勝敗は **0W / 36T / 0L**。

hard12 の Δ合計（B−A）: ΔV 0 / ΔD −3 / ΔB +7。

## ΔV/ΔD/ΔB の読み

- A と B のスコア差が出た盤面は hard12 で 11/12、holdout24 で 17/24。差の大半は B 軸（hard12: 9盤面、holdout24: 16盤面）。
- D 軸の差は 3盤面のみ（hard-24-a4-d7-s9007 −1、hard-24-a6-d9-s9008 −2、hold-24-a4-d10-s5015 −2。すべて B が D を下げて負け）。V 軸の差は 0。
- つまり scope=both のレアリティ割引は主に「ボール数を増やす」方向に働くが、辞書順の上位軸（V, D）を下げる副作用のほうが効いて、決着盤面では 28盤面中 15 で A が勝った。

## 回答時間

| 候補 | hard12 max | hard12 median | holdout24 max | holdout24 median |
|---|---|---|---|---|
| A-rarity0 | 8.53s | 3.03s | 39.70s | 2.34s |
| B-both | 9.53s | 2.84s | 44.27s | 2.35s |
| C-initial | 8.60s | 2.85s | **65.46s** | 2.33s |

- holdout24 の最大 65.46s は C の `hold-32-a7-d10-mixed-s5017`。同盤面の A は 33.77s（B は 33s 台）。直列実行・load1 30前後の共用機であり、単発の壁時間差を実機の優劣とは扱わない（README の方針どおり）。
- A と C は計画が同一でも壁時間は揺れる（median 差 0.04〜0.18s、最大 52s）。

## 最悪の盤面別劣化（A 基準）

- A vs B（B が A に負けた盤面の代表）:
  - `hard-16-a5-d10-short-s9004`: ΔB −12（261 vs 273）
  - `hard-32-a6-d7-short-s9011`: ΔB −11（302 vs 313）
  - `hard-24-a6-d9-short-s9008`: ΔD −2 / ΔB −2
  - `hold-32-a7-d10-mixed-s5017`: ΔB −8（1121 vs 1129）
  - `hold-32-a8-d10-short-s5020`: ΔB −8（641 vs 649）
  - `hold-24-a4-d10-mixed-s5015`: ΔD −2（198 vs 200）
- A vs C: 劣化なし（全36盤面スコア同一）。
- B vs C は B−A の符号反転と同じ差分集合。

## 合法性カウンタ

| 候補 | illegal | fuel_short | rejected | timeout | 非 success |
|---|---|---|---|---|---|
| A-rarity0 | 0 | 0 | 0 | 0 | 0 |
| B-both | 0 | 0 | 0 | 0 | 0 |
| C-initial | 0 | 0 | 0 | 0 | 0 |

36盤面 × 3候補 = 108 レコードすべて `status=success` / `scored=true`。違反なし。

## 解釈

1. **scope=initial の不変性は実証**。C は A と 36/36 盤面で最終計画が完全一致（V/D/B、`agent_ball_counts`、`daily_varieties`）。14f88ac のコミットメッセージが主張する「固定経路評価は initial の下で不変」が実データでも確認された。つまり初期経路への希少バイアスは、最終計画には残らない。
2. **親の否定結果は both 側に付くことが確定**。旧測定（#417/#418, d87c66d）の ΔB−4 は「種・予測の結合」によるものだった可能性が残っていたが、今回の同源測定でも `both`（=親の結合動作）は ΔD −2〜−3 / ΔB +7〜+8 で辞書順に負けた。B のボール増加は辞書順の上位軸を下げる副作用とセットであり、採用基準を満たさない。
3. **C は A と同じ**なので、`rarity_scope=initial` を採用する動機はない。`rarity=2` 系は初期経路だけなら何も変わらず、予測まで広げると辞書順で劣る、という分解の結果。

## 判定

- **scope=initial (C) は baseline (A) に勝らない**: 36盤面スコア完全一致（差分 0）。
- **scope=both (B) も baseline (A) に勝らない**: hard12 4W/8L、holdout24 13W/11L、ΔD 負・ΔB 正で辞書順は劣勢。
- **推奨: negative（非採用）**。ステップ4の holdout2 確認バッチは実施しない（B も C も A に辞書順で勝っていないため、実施条件を満たさない）。

## 成果物

- `docs/research/rarity-scope-419/results-hard.jsonl` — hard12 の runs.jsonl コピー（63レコード。fixture 9盤面 + hard 12盤面。上の分析は `fixture:hard-` 12盤面のみ）
- `docs/research/rarity-scope-419/results-holdout.jsonl` — holdout24 の runs.jsonl コピー（99レコード。fixture 9盤面 + holdout 24盤面。分析は `fixture:hold-` 24盤面のみ）
