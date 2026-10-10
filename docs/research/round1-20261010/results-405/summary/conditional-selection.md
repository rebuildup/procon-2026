# 盤面条件別の候補優劣と「条件付き選択」の評価（既存データのみの再集計）

2026-10-10。測定の新規実行はしていない。生データは `docs/research/round1-20261010/results-405/` 配下の JSONL を再集計した。
勝者は**公式辞書順（V → daily_varieties_sum → B）**で判定し、B 合計とは明確に区別する。
**注記: B 差が ±20 未満のセルは有意性が弱い**（同一バッチ再測定ノイズ 0–7 balls、異バッチ間 ±10–18、clockless は決定論。後述 §5）。同点は「同点」と明記する。

測定条件の凡例:
- **@10s / @20s**: `budget_ms=10000 / 20000` の wall-clock モード（seed42・seed7 の 2 実行バッチあり）
- **clockless**: `budget_ms` 指定なしの決定論モード（iters ベース）

## 1. 盤面規模 × 測定条件（軸 1）

### 1.1 clocked @10s・補給 1 台（9 corpus 盤面 × seed42/seed7、18 セル）

各セルは `(V, dV, B)`。太字は公式辞書順の勝者タグ。

| 盤面 | サイズ | seed42 (V,dV,B) / 勝者 | seed7 (V,dV,B) / 勝者 |
|---|---|---|---|
| hard-16-a3-d9-s9003 | 16 | convoy (13,105,115) / chains8=fleet (13,106,113) → **fleet=chains8 同点**（dV +1 で convoy 敗北） | convoy (13,105,116) / **convoy** |
| hard-24-a5-d7-s9005 | 24 | fleet (20,140,212) > chains8 208 > convoy 202 → **fleet** | chains8 204 > convoy=fleet 202 → **chains8** |
| hard-24-a6-d9-s9008 | 24 | convoy (20,180,278) > chains8 261 > fleet 250 → **convoy** | chains8 (20,180,277) > fleet 257 > convoy 255 → **chains8** |
| hard-32-a8-d7-s9012 | 32 | chains8 (27,189,433) > fleet 406 > convoy 369 → **chains8** | fleet 425 > chains8 418 > convoy 387 → **fleet** |
| hold-16-a4-d7-s5001 | 16 | convoy (13,91,188) > chains8=fleet 186 → **convoy** | chains8 189 > convoy=fleet 188 → **chains8**（B +1） |
| hold-24-a5-d4-s5013 | 24 | fleet=chains8 218 > convoy 217 → **fleet**（B +1） | 3 候補 (20,80,218) → **同点** |
| hold-24-a6-d10-s5016 | 24 | chains8 473 > convoy 472 > fleet 471 → **chains8**（B +1/+2） | chains8 474 > fleet 473 > convoy 471 → **chains8** |
| hold-32-a6-d6-mixed-s5023 | 32 | fleet 461 > convoy 460 > chains8 447 → **fleet**（B +1） | chains8 460 > fleet 457 > convoy 433 → **chains8** |
| hold-32-a8-d10-s5020 | 32 | chains8 676 > fleet 673 > convoy 666 → **chains8** | chains8 675 > fleet 673 > convoy 664 → **chains8** |

B 合計（参考・公式勝者数とは別軸）: seed42 — chains8 2735 / fleet 2712 / convoy 2706（9 盤面合計）。seed7 — chains8 2714 / fleet 2713 / convoy 2654。

**規模別の公式勝者数（18 セル）**: 16×16 — convoy 2 / chains8 2 / 同点他 2。24×24 — chains8 3 / fleet 2 / convoy 1 / 同点 2。**32×32 — chains8 4 / fleet 2 / convoy 0**。
32×32 では convoy が公式勝者ゼロ。差は s9012 で −27/−38、s5023 で −1/−24、s5020 で −7/−9。

### 1.2 clocked @20s（同一実行 b2-20s-s9012 + round1-solo）

| 盤面 | convoy@20s | fleet@20s chains8 | fleet@10s | 公式勝者 |
|---|---|---|---|---|
| hard-32-a8-d7-s9012 | (27,189,388) | — | fleet (27,189,396) / chains8 (27,189,415) | **chains8**（convoy −27） |
| round1-match3 (16×16) | (10,46,54) | (10,43,55) | (10,43,51) | **convoy**（dV +3） |

### 1.3 clockless（決定論）

| 区分 | convoy | fleet | 公式勝者 |
|---|---|---|---|
| hard 16×16 (s9001,s9002,s9003,s9004, iters120k) | (13,107,114)/**(13,130,277)** vs (13,130,173)/(13,117,206) | (13,105,112)/(13,130,175)/(13,117,210)/(13,130,259) | convoy 2 / fleet 2（s9007 を除く 4 盤面、差は B ≤4） |
| hard 24×24-a4 s9007 (iters120k) | (20,139,174) | (20,139,174) | **同点** |
| hard 24×24-a5+ s9005/s9006 (iters120k) | **TIMEOUT（425s harness deadline 超過・失格）** | (20,140,207)/(20,140,227) | **fleet**（不戦勝） |
| hard 32×32 ×4 (s9009,s9010,s9011,s9012, iters120k) | **TIMEOUT ×4** | (27,189,339)/(27,189,271)/(27,189,330)/(27,189,390) | **fleet**（不戦勝） |
| honsen-like 16×16 ×24 (D 飽和, iters60k) | B 合計 +85（17 勝 2 敗 8 分, 勝ち幅 ≤12） | — | **convoy 優位** |
| round1-match3 16×16 (D 未飽和, iters60k) | (10,46,60) | (10,45,57) | **convoy**（dV +1） |

**重要**: clockless convoy（`fleet_iters=120000`）は 24×24-a5 以上と 32×32 で harness deadline 425s を超過し失格（`wsl-corpus/hard-convoy-iters120k/runs.jsonl` status=timeout）。clockless convoy が成立するのは 16×16（と 24×24-a4）まで。

## 2. 補給車数（軸 2）

**@10s・補給 2 台（S,S,P,...）は `wsl-10s/c-s2/` に存在**（convoy vs fleet のみ、chains8 の測定なし）。seed42 単一実行。

| 盤面 | convoy (V,dV,B) | fleet (V,dV,B) | 公式勝者 |
|---|---|---|---|
| hard-16-a3-d9-s9003（S,S,P = 巡回 1 台） | (13,63,63) | (13,63,63) | 同点 |
| hard-24-a5-d7-s9005 | (20,140,181) | (20,140,178) | **convoy**（B +3） |
| hard-24-a6-d9-s9008 | (20,180,228) | (20,180,228) | 同点 |
| hard-32-a8-d7-s9012 | (27,189,402) | (27,189,416) | **fleet**（B +14） |
| hold-16-a4-d7-s5001 | (13,91,133) | (13,91,135) | **fleet**（B +2） |
| hold-24-a5-d4-s5013 | (20,80,182) | (20,80,181) | **convoy**（B +1） |
| hold-24-a6-d10-s5016 | (20,200,386) | (20,200,386) | 同点 |
| hold-32-a6-d6-mixed-s5023 | (27,162,393) | (27,162,400) | **fleet**（B +7） |
| hold-32-a8-d10-s5020 | (27,270,586) | (27,270,579) | **convoy**（B +7） |

補給 2 台では 32×32 の convoy 劣後が −14/−7/+7 に縮み（補給 1 台は −37/−1/−7... s9012 で −37→−14）、s5020 では convoy が逆転。ただし**単一実行・chains8 データなし・盤面構成（エージェント種別）も異なるため、1 補給の結果と直接比較はできない。未確定の兆候として扱う**。16/24×24 は全て B 差 ≤3（ノイズ域）。

## 3. D 到達性・燃料制約（軸 3）

D 飽和の判定は実測値で行った: `dV == V × 日数` を満たす盤面（corpus hard/hold/honsen 全部）を「飽和」、満たさない（match3 のみ）を「未飽和」。`fuel_short_count` は全実行で 0（燃料枯渇は公式結果に現れていない。制約は計画内で効いている）。

| 区分 | 条件 | 公式勝者 | B 差 |
|---|---|---|---|
| **D 未飽和・燃料制約（match3 型 16×16）** | @10s (s42/s7) | **convoy**（dV 45/46 vs fleet・chains8 43） | B +4〜+8 |
| 〃 | @20s | **convoy**（dV 46 vs 43） | B −1〜+3 |
| 〃 | clockless | **convoy**（dV 46 vs 45） | B +3 |
| **D 飽和（hard/hold 型）** | clocked 16×16 | 混在（差 ≤3 が大半。s9003-s42 は dV −1 で convoy 敗北） | ±3 |
| 〃 | clocked 24×24 | 混在（chains8 3 / fleet 2 / convoy 1） | ±28 |
| 〃 | clocked 32×32 | **chains8 4 / fleet 2 / convoy 0** | −37〜−1 |
| 〃 | clockless 16×16 (honsen-like) | **convoy**（17 勝 2 敗 8 分） | ≤12 |
| 〃 | clockless 24/32×32 | **fleet**（convoy は timeout 失格） | 不戦勝 |

**match3 型（D 未飽和・燃料制約）では 3 条件すべてで convoy が公式勝者**。dV 差 +1〜+3 が B 差より先に決める。一方 D 飽和 clocked では優劣が規模依存（§1）。

## 4. 「条件付き選択」の評価

**成立する条件は既存データで 1 つだけ定式化できる: 「clocked（budget_ms 指定）かつ大盤面（32×32）では convoy を使わない」。** それ以外の領域（clocked 16/24×24、補給 2 台）は差がノイズ以下か単一実行で、切替の根拠にならない。

**判別情報の事前観測可能性**: 判別に必要な量はすべてソルバが `SolverContext`/`state` から計算可能。
- 巡回車数（patrols）・スポット数（spots）・マップ規模: state から取得可能。収束スケール `(patrols×spots)²/3` は `fleet_solver.rs:1198-1199` と同一の式でソルバ内で計算できる（実例: match3 は 3 巡回 × 16 スポット → (48)²/3 ≈ 0.26s。honsen-like も同形状）。
- clocked かどうか: 候補自身の `budget_ms` パラメタ（convoy では既に window として実装済み、`convoy_solver.rs:1110-1117`）。
- D 飽和は事前観測**できない**（日の経過で判明する）。「D 未飽和なら convoy」という規則は判別器にできない点に注意。

### 判別規則案

**R1（規模 gate・データ検証済みの定式化）**: 「`budget_ms > 0` かつマップ一辺 ≥ 32 → chains8 を選ぶ。それ以外は convoy（現状維持）」
- 既存 18 clocked セルへの適用結果: 32×32 の 6 セル → chains8 が公式勝者 4/6（外れた 2 セルは s5023 で fleet が勝者、chains8 は −14/−3）。convoy 現状維持の 0/6 から 4〜5/6 に改善、oracle 起票基準の合計 regret は −21 balls（chains8 選択時）。
- ≤24×24 を convoy 維持した場合の機会損失: 14 セル中公式勝者は convoy 4。ただし損失は s9005-s42 −10 と s9008-s7 −22 を除き ≤3 balls で、fleet 固定にした場合の損失（s9008-s42 −28 等）と同オーダー。**この領域はどちらに切り替えても優位を確定できない**。
- 予測精度の限界: seed 反転で s9008/s5023 の勝者が入れ替わるセルがあり、規則は「勝者候補」ではなく「convoy を外す/外さない」の 2 値としてのみ信頼できる。

**R2（収束スケール gate・実行時計算版）**: 「`budget_ms > 0` かつ `(patrols×spots)²/3 ≥ θ` → chains8。θ = 6s（@10s 時）」
- 根拠: 32×32 盤面のスケール下限は spots ≥ series 数(27) より s9012 (7×27)²/3 ≈ 11.9s、s9011 (5×27)²/3 ≈ 6.1s。24×24 最大の s9008 は ≈ 3.3s（spots ~20 と推定）。16×16 は ≤1s。
- 既存データへの適用: s9012×2 / s5020×2 を捕捉（公式勝者 chains8）、**s5023×2 を取り逃す**（spots の実値不明で下限 27 で計算。θ=6s では未達）→ 公式勝者 4/6。
- R1 との差: マップ規模の代わりに盤面内容（スポット密度）を見る。θ の校正には hard/hold corpus fixture の spots 実値の確認が必要（data/round1/derived/ 配下に hard/hold の fixture JSON を確認できていない）。θ を 6s 未満に下げると 24×24-a6+ を誤捕捉するリスク。
- R2 の利点: 規模以外の要因（スポット数が多い小盤面）にも外挿できる可能性。**未検証の外挿を確定として扱わないこと**。

**採用判定への影響（制約事項）**:
1. **V/D 悪化**: chains8 は dV を落とすことがある（s9008-s42 で dV 179 = −1）。32×32 での实测は dV 劣化なし（saturated どおり）。convoy も s9003-s42 で dV −1。公式辞書順では D ±1 が B 数十に優先するため、切替候補の D リスクは必ずセル別に確認する。
2. **wall-clock 予算超過**: convoy は day0 で 10s 予算に対し 11.0s（s9005）・10.9s（s9008）のオーバーラン実績（`day_budget_exceeded_days` は空＝公式余裕内だが、deadline チェックが 8192 labels ごとの粗さのため）。chains8/fleet は 10.0s ちょうど。マージンがタイトな本番環境では convoy 採用領域でこのリスクを持ち込む。clockless convoy は 425s harness deadline 超過で 24×24-a5+・32×32 が失格（§1.3）。
3. **B の実行間変動**: 同一候補の再測定（d-forecast=* vs d2-*）で 16/24×24 は 0–1 balls、32×32 は 0–7 balls。b10s バッチと b2 バッチの間では fleet@10s 406↔396 / chains8 433↔415（±10–18、競合候補の負荷差）。**±20 未満の差は採用根拠に使わない**。clockless は決定論。
4. 補給 2 台の 32×32 改善（−14/−7/+7）は chains8 測定なし・単一実行 → 未確定。

## 5. 参照パス（PR #407 集約用）

- clocked @10s 補給 1 台: `docs/research/round1-20261010/results-405/wsl-10s/b10s-seed42/runs.jsonl`、`b10s-seed7/runs.jsonl`
- clocked @20s: `wsl-10s/b2-20s-s9012/runs.jsonl`、`wsl-10s/round1-solo.jsonl`（match3 solo、decide_ms 付き）
- clocked 補給 2 台: `wsl-10s/c-s2/runs.jsonl`
- convoy 変種 @10s: `wsl-10s/d-forecast=busy|persist/runs.jsonl`、`d-width=12|48/runs.jsonl`、再測定 `d2-busy|d2-persist/runs.jsonl`（§5 のノイズ推定に使用）
- clockless: `wsl-corpus/hard-convoy-iters120k/runs.jsonl`（timeout 6 件）、`hard-fleet-iters120k-chains4/runs.jsonl`、`hard-fleet-iters60k-chains4/runs.jsonl`、`holdout-fleet-iters60k-chains4/runs.jsonl`、`holdout-fleet-iters60k-1chain/runs.jsonl`、`smoke-convoy/runs.jsonl`
- clockless 本戦型: `holdout-honsen-like.runs.jsonl`（honsen-like 24 + match3 + rehearsal）
- 盤面メタ（match3/honsen-like）: `data/round1/derived/round1-match3.case.json`、`data/round1/derived/honsen-like/*.json`（16×16・5 日・fuelLimits 40・16 スポットを確認）
- 関連分析: `analysis/convoy-clocked-degradation.md`（32×32 劣後の機構分析）

## 6. 未確定事項（次の測定で確定させるべき）

1. hard/hold corpus fixture の spots 実値（R2 の θ 校正に必要）。
2. 補給 2 台での chains8（§2 の 32×32 判定に必要）。
3. clocked 24×24 の convoy vs fleet vs chains8 の seed 反転再測定（s9005 −10 / s9008 +28 がノイズか構造かの判定。±20 基準では両方とも境界上）。
4. 32×32 での clocked convoy 改善版（pool[0] 予算強化、`analysis/convoy-clocked-degradation.md` §4 P2）が規則 R1 の分岐を不要にするか。

## 7. 2026-10-11 追補（バッチ 1・F 系列測定確定分）

本節は測定確定分の追補。§1–6 の再集計には手を入れない。

### 7.1 chains ラダー（offline・E1、`wsl-corpus/param-chains{8,12,16}/`）

- 決定論 offline `iters=60000` の chains ラダー（gen36 B 合計）: chains4 = 11745 → chains8 = 11813 → **chains12 = 11863（ピーク）** → chains16 = 11858。
- **D-loss は chains8 の s9008（dV 179→175 決定論）のみ**。chains12/16 では出ず。chains12 vs chains8 は 10 win / 32 draw / 2 LOSE（B-only 微減 −3/−2）。
- **R1（clocked 32×32 → chains8）への影響**: chains12 は offline で chains8 の上位互換候補（D 非劣化・B +50）。ただし **clocked `budget_ms` モードでの chains12 は未測定**であり、R1 の確定枠を差し替えるには clocked での s9008 dV 推移（基準 180）と 32×32 セルの再確認が要る。

### 7.2 seed 魯棒性（E2、`wsl-corpus/param-chains8-seed{7,123}/`）

- seed7: D-loss なし・B −12（holdout24）、maxday ≤19.0s で時間安全。
- **seed123: 9/24 盤面が invalid/timeout（maxday 188–253s）— 探索時間が seed で支配的に爆発**。seed 選定は「実行時間安全性」を辞書順と同時に評価する必要あり（live 投入候補の seed 変更は maxday 監査が必須）。

### 7.3 fuel 軸（F3-offline、`wsl-corpus/f3-offline-fuel/runs.jsonl`）

§3 fuel 軸の追加候補（仮枠）への最初の一次データ。**clockless `iters=60000`・seed42・スクリーニング 9 + fixture 9・chains4/8 × fuel_need=50 ± fuel_mix**。

- **完了セル 34 対で dV 下落 = 0**（fuel 系は辞書順 D を落とさない）。B 合計利得: fuel-only +29(c4)/+26(c8)、mix +44(c4)/+50(c8)。**mix > fuel-only が両 chain で成立**（§6.2 分解式: 重複なし・独立・希釈なし）。
- **chains8 offline の唯一の D-loss（s9008 dV−4 決定論）が fuel_need=50 で回復**（`(20,175,226)` → `(20,180,263)`・D+5/B+37）。
- 比仮説（§6.1）は部分整合: 比 2.7–3.0 の s5013/s5016/s5020 は利得 ≈0〜微減の一方、**比 1.38 の s5023 で fuel-only B −29**（D 同点）。比 ≤1.5 → 有効という仮枠はこのまま確定できない。
- s5020 c8 fuel/mix の 2 セルは負荷 timeout で欠落（静穏時 Batch A で補完予定）。検証 27 盤面での F 系確認は並走セッション（`measure-63a7e5e`・holdout 24+32・fuel_mix=on・2 seed）が進行中、clocked 分解（バッチ B）は未実施。

### 7.4 convoy bail=on（PR #435、`bail-435-verify`・`eval-bail-435/runs.jsonl`）

- ラベル爆発盤面 4 枚（s5016/s5020/s5023/s9012）・`convoy:budget_ms=20000:seed=42` parent vs bail 同一実行（load 9–13・B は ±20 ノイズ含む）。
- **axis-4 と期限安全性は構造的に確定**: parent maxday 33–50s（60s 日窓に接近）→ bail 全盤面 ≤8.9s。
- B: 爆発盤面で **s5023 +39 / s5020 +57**、s9012 −23（ノイズ境界）/ s5016 −7。fixture 9 盤面は 8 draw + 1 の B−1。静穏時 B 確認再実行が判定条件。
- R1 の「32×32 では convoy を外す」根拠の 1 つ（convoy の大盤面劣化・期限リスク）は bail=on で縮む可能性がある。cluster=on 未実装のため bail 単独での判断。