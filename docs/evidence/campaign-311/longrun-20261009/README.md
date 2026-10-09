# 大規模検証キャンペーン（Windows/WSL 測定セッション）— 最終報告

- **測定 SHA**: `36adfc5cceb97bad521e8e19f8d134a308a0ce93`（`eval-longrun-20261009` = release-0-4-0 + origin/342 + origin/343。**評価器パリティ修正 (#355) 前**の MatchSim — 全数値は暫定扱い）
- **パリティ後再検証 SHA**: `5bf57af`（eval-newengine-20261009 = #355 正式パリティ ENGINE 上に #349/#341/#344 を最小移植。2 conflicts は search_trials の `fuel_aware:false` 維持 + `park`/`lookahead` 取り込みで解消）
- **harness**: `procon-eval` release、`--jobs 12`（jobs 2〜16 を 3 rep で較正、jobs≥8 で CPU 99%、jobs=12 が最速 165rec/11.4s）。軸4 は `--jobs 1` の SEQ で別計測。
- **決定論**: jobs=2..12 で全候補 tiers1-3 完全一致。マージ 593 重複キーで tier-conflict 0。
- **証跡**: `target/longrun/{A,B,C,D,FEAS,KINDS,HOLDOUT,SEQ,NEW,VEH}/*/runs.jsonl`、マージ済み `target/longrun/merged-*/`（aggregate/pairwise/by_condition/provenance）。実行記録は各 `batch.json`（コマンド/SHA/jobs/wall/CPU/RAM サンプル）。
- **再実行**: `python3 scripts/longrun_eval.py --group <A|B|C|D|FEAS|KINDS|HOLDOUT|SEQ|NEW|VEH> --jobs 12`（完了バッチは SHA 不一致以外スキップ）。マージは `cargo run --release -p bench --example eval_merge -- --input target/longrun/A ... --baseline 'coverage:budget=official:park=none' --out target/longrun/merged-X --axes 3`。
- **ツール本体**: Issue #356 / Draft PR #357（branch `356`）。

## 1. スクリーニング（group A: 133 boards × 11 candidates、軸1-3 のみ、vs `coverage:park=none`）

| candidate | W/D/L |
|---|---|
| `search:trials=16:lookahead=next` | **121W/42D/0L**（round-robin 996W/311D/22L で全候補中 1 位） |
| `search:trials=64:lookahead=next` | 101W/32D/0L（vs trials=16:la では **40W/86D/7L** で上位） |
| `search:trials=16`（=`park=far` 同点） | 104W/59D/0L |
| `anytime:budget_ms=1000` | 6W/158D/1L |
| `coverage:park=far` | 2W/162D/1L（唯一の勝ち: `fixture:701` D+4/B−19） |
| `coverage:park=far:fuelaware=true` | 2W/155D/**6L** |
| `multiloop:kmeans=5` / `loop:iters=128` / `astar:policy=step` | ほぼ全敗 |

## 2. HOLDOUT（seed 101–108、探索に未使用、133 boards）

`search:trials=64:lookahead=next` **99W/34D/0L**（敗北 0）。`search:trials=16:lookahead=next` 94W/39D/0L。`search:trials=16` は 86W/45D/**2L**（open:seed=107 / road-heavy:seed=104:large、いずれも B−1）→ lookahead なしの trials=16 は僅差で負け得る。`fuelaware=true` は 1W/129D/**3L** — 退化は再現（small-8x8 / openapi-sample / arena-20261008T143339Z、D−1・B−3〜4）。`park=far` / `anytime:0` は 1W/132D/0L。

## 3. anytime 予算と改善量（B 群・23 boards）

budget_ms = 1000 / 5000 / 15000 で **W/D/L 完全同一**（1W/0D/22L vs baseline）。予算を増やしても改善は 0 で、応答時間だけ 2.7s→15.9s 平均に増加。**anytime の予算増は無駄**。当日系列のみの受理条件がボトルネック（search の (V,D,B) 辞書順受理と構造的に異なる）。

## 4. 軸4（応答時間）と採用ゲート（FEAS: jobs=1 無競合）

| candidate | 1日 decide 最大 | 判定 |
|---|---:|---|
| `search:trials=16:lookahead=next` | 6.3 s | 安全 |
| `search:trials=64:lookahead=next` | 22.6 s | 安全（60s 窓内） |
| `search:trials=128:lookahead=next` | 48.0 s | **非推奨**（余裕なし。並列時は daySeconds 超過 invalid 3件） |
| `search:trials=256:lookahead=next` | 67.5 s | **invalid 1件**（road-heavy:seed=4:large day0 超過） |
| `coverage:park=none` | 0.37 s | 安全 |

SEQ（133 boards、jobs=1）: search64+la 平均 3.2s/ケース・p95 12.2s・max 24.0s。coverage 系は平均 36ms。**16/32/64 trials は期限安全、128 以上は不採用**。

## 5. 車両構成・kinds 軸

- `fixture:701`: 4P/1S で D=200/B=451（search16:la）vs 3P/2S D=199/B=395 → 巡回車比が B を大きく動かす。
- KINDS 群: 全候補で patrol-heavy > balanced > supply-heavy の B 順。fuelaware の退化は kinds 非依存で小盤 3 固定。
- 7 台 32×32（priority-4p3s/5p2s/6p1s、パリティ ENGINE 上）: 4P3S が総 B 最大。6P1S は燃料切れリスクで D 減。

## 6. パリティ ENGINE 上の再検証（5bf57af、優先盤面 19 boards × 8 候補 × 2 kinds）

傾向は旧評価器と同一: `search:trials=32/64:lookahead=next` が 26W/12D/0L（vs coverage:park=none）、trials=16:la 25W/13D/0L。**lookahead 優位はパリティ後も維持**。fuelaware は 2W/31D/**5L** に悪化。701 は coverage D=40/B=41 に対し search:trials=32:la D=59/B=59（旧評価器との絶対値比較は不可 — シミュレータ意味論が別物）。

## 7. 外部参照: fleet（branch 371 @ 5ddb153）の独立検証

測定セッション worktree `/.worktrees/fleet-verify`（SHA `5ddb153`、公式パリティ ENGINE 込み release 系）で fleet 開発セッションの申告を全検証 → **すべて再現**（anytime 対 60W/3D/1L、chains=4 では敗北 0、V/S 劣化なし、B +49〜81%、448 run 無違反）。701 同一 SHA 直接比較で `fleet:budget_ms=10000` が V20/D200/**B576**・3.1秒/日（対最良対照 B+27%）。証跡: `.worktrees/fleet-verify/target/fleet-verify/VERIFICATION.md`。

## 8. 結論と引き渡し

1. **最有力（offline 軸1-3）**: `search:trials=64:budget=official:lookahead=next` — HOLDOUT 敗北 0・全候補 round-robin 1 位・軸4 も 60s 窓内。**候補開発担当へ: #341/#344 の Ready 化判断材料**。live は trials≤64。
2. **不採用根拠**: anytime 予算増（効果 0）、fuelaware=true（3〜6 ケース退化）、search trials≥128（期限）、park=far 単体（現 baseline 対同点）。
3. **fleet**: 測定セッションとしての反証なし。Arena 専任の投入判断に委ねる（本セッションは投入しない）。
4. **未確定事項**: ① 全数値は MatchSim — 公式サーバー受理（response 受理・渋滞下スコア）は未検証。② fleet の実績は Arena 台帳（#326）参照。③ 旧評価器（36adfc5）数値と新評価器（5bf57af 以降）の絶対値は混在禁止。
