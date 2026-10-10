# results-405 WSL 計測サマリー（結果回収・集計）

- 作成: 2026-10-10（結果回収エージェント）。**本ファイルは既存 runs.jsonl / origin/406 証跡の集計のみ**。測定の再実行は行っていない。
- 対象 worktree: `eval-convoy-415`（merge origin/405@6ab5b63 [convoy] + origin/409@e79d7869 [fleet chains=8 / PR #415]）、source_commit_sha は全レコード `6ab5b63...`（e2e のみ `4c72187...`）。
- 勝敗定義: 公式辞書順 `(V, D, B)` の降順比較（`docs/evidence/campaign-311/fleet-371/compare.py` と同一ロジック）。`fixture:openapi-sample` は常に除外。
- 悪化の分類: **V-loss**（V が下がる）＞ **D-loss**（V 同値で D が下がる）＞ **B-only-loss**（V/D 同値で B のみ下がる）。
- 実行環境: WSL2 上のこの worktree の `target/release`。4f 側の証跡は `origin/406@b575754` `docs/research/round1-20261010/convoy-holdout-406/`（holdout 24 + hard 12 盤面、budget_ms=20000 の convoy と fleet60k-c4）。

## 0. 全スイート一覧

| スイート | 場所 | 盤面 | 予算 | 候補 | 主要結論 |
|---|---|---|---|---|---|
| A | wsl-corpus/holdout-fleet-iters60k-chains4 | holdout 24 + small 8 | iters 駆動 | fleet60k chains=4 | Mac 4f と盤面ごと完全一致（後述） |
| A' | wsl-corpus/hard-fleet-iters60k-chains4 | hard 12 + small 8 | 同上 | 同上 | 同上 |
| B | origin/406（4f 公表済み） | holdout 24 / hard 12 | convoy@20s | convoy, fleet60k-c4 | convoy 17勝3分4敗 / 9勝1分2敗、V/D 悪化 0 |
| C | wsl-10s/b10s-seed42, b10s-seed7 | holdout 5 + hard 4 + small 9（17） | @10s | convoy / fleet / chains8 | chains8 が B 総量最大、convoy は 10s では B で逆転される |
| D | wsl-10s/b2-20s-s9012 | hard s9012 + small 9 | convoy@20s | convoy@20s / fleet@10s / chains8@10s | chains8@10s が s9012 B 最大（415） |
| E | wsl-10s/c-s2 | 17 盤面 | @10s, kinds S,S,P×6 | convoy / fleet | 補給 2 台では差が B-only に縮小（convoy 優位消失） |
| F | wsl-10s/d-width=12/48, d-forecast=persist/busy | holdout 5 + small 9 | @10s | convoy 変体 | 既定（width24・2予測合算）が最も頑健 |
| G | wsl-10s/d2-persist, d2-busy | holdout 5 + hard s9012 + small 9 | @10s | 同上 | busy 予測は hard 盤面で +B（単一 seed で非決定的） |
| H | wsl-corpus/hard-fleet-iters120k-chains4 | hard 12 + small 8 | iters=120k | fleet60k vs 120k | W10 D8 L2、D +3 / B +70、実行時間 3 倍 |
| I | wsl-corpus/smoke-convoy | small 9 + hard 1 | 時計なし | convoy 既定 | smoke 合格（bad 0） |
| M | wsl-10s/round1-solo.jsonl | 第3対局 solo・観測交通 | @10s / @20s | convoy / fleet / chains8 | #415 B+4 を 2 seed で完全再現 |
| pending | wsl-corpus/hard-convoy-iters120k | hard 12 | @10s 相当 | convoy fleet_iters=120k | **実行中**（未出力） |
| pending | wsl-corpus/holdout-fleet-iters60k-1chain | holdout 24 | iters 駆動 | fleet chains=1 | **証跡なし**（未確認） |

全スイートを通じて illegal / fuel_short / rejected / timeout は **WSL 側 0**。4f hard fleet の初回 run のみ `hard-16-a4-d9-short-s9002` が (0,0,0) bad=1（=ハーネス異常。hard-rerun で 3 盤面再取得済み、本集計は rerun merge 値を使用）。

## 1. A/B: 共通 corpus（4f 36 盤面 vs WSL fleet baseline）

### A. クロスマシン再現性（同一候補 fleet60k chains=4, seed42）

| 盤面群 | 4f | WSL | 盤面ごとの差異 |
|---|---|---|---|
| holdout 24 | V480 / D3452 / B8879 | V480 / D3452 / B8879 | **0 盤面差異（24/24 完全一致）** |
| hard 12 | V240 / D1834 / B2866 | V240 / D1834 / B2866 | **0 盤面差異（12/12 完全一致）** |

- iters 駆動（時計なし）の fleet は Mac と WSL で盤面ごと (V,D,B) が bit 級に一致。**スコア決定に実行環境差は混入していない**。
- 実行時間は環境差が出る（holdout 総計 658.8s→401.2s、day max 15.16s→7.94s）がスコアに影響なし。

### B. 4f 公表値の検証（convoy@20s vs fleet60k-c4、すべて 4f データ内）

| 盤面群 | convoy V/D/B | fleet V/D/B | 判定 | 悪化内訳 |
|---|---|---|---|---|
| holdout 24 | 480 / 3452 / **8929** | 480 / 3452 / 8879 | **17勝 3分 4敗** | V-loss 0, D-loss 0, **B-only 4** |
| hard 12 | 240 / **1838** / **2915** | 240 / 1834 / 2866 | **9勝 1分 2敗** | V-loss 0, D-loss 0, **B-only 2**（D 改善 4 盤面含む） |

- B1（holdout）の B-only 敗北 4 盤面: `hold-16-a3-d7-mixed-s5007`（131 vs 132）、`hold-16-a5-d7-short-s5004`（199 vs 201）、`hold-24-a5-d6-short-s5010`（209 vs 210）、`hold-32-a8-d10-short-s5020`（662 vs 664）。
- B2（hard）の D 改善 4 盤面: `hard-16-a3-d9-short-s9003`（D105→106）、`hard-24-a4-d7-short-s9007`（138→139）、`hard-24-a6-d9-short-s9008`（179→180）、`hard-32-a7-d7-short-s9010`（188→189）。B-only 敗北は `hard-24-a5-d7-short-s9006`（222 vs 224）、`hard-32-a6-d7-short-s9011`（323 vs 329）。
- PR #407 本体の「holdout 17勝7分2敗・悪化0」は別盤面数（26 盤面）での値だが、方向（V/D 悪化ゼロ、B 主導の優位）は本集計と一致。
- 実行時間: convoy@20s は holdout 総計 2452.3s（day max 21.03s）、fleet60k-c4 は 658.8s（day max 15.16s）。convoy は 1 盤面あたり約 3.7 倍の wall-clock を使う。

## 2. C: b10s（@10s 実戦予算、holdout 5 + hard 4 + small 9 = 17 盤面 × 2 seed）

### 合計（2 seed 合算、17 盤面）

| 候補 | V | D | B | 総 response_time | day max |
|---|---|---|---|---|---|
| convoy@10s | 418 | **2890** | 6135 | 1241.7s | 11.05s |
| fleet@10s（chains4） | 418 | 2889 | 6229 | 1405.7s | 10.71s |
| fleet chains8@10s | 418 | 2889 | **6275** | 1581.2s | 10.09s |

### 盤面別（2 seed 合算 W/D/L、悪化は候補を「後者 vs 先者」で読む）

| 比較 | W | D | L | 悪化内訳 |
|---|---|---|---|---|
| convoy vs fleet | 5 | 18 | 11 | V-loss 0, **D-loss 1**（s42: hard-16-a3-d9 D105 vs 106）, B-only 10 |
| chains8 vs fleet | 10 | 19 | 5 | V-loss 0, **D-loss 2**（s42: hard-24-a6-d9 D179 vs 180 / s7: hard-16-a3-d9 D104 vs 105）, B-only 3 |
| chains8 vs convoy | 13 | 15 | 6 | V-loss 0, **D-loss 2**, B-only 4 |

### 盤面ごとの代表値（@10s, s42 / s7 の順）

| 盤面 | convoy | fleet | chains8 |
|---|---|---|---|
| hard-32-a8-d7-short-s9012 | 27/189/**369** , 27/189/387 | 27/189/**406**, 27/189/**425** | 27/189/**433**, 27/189/418 |
| hard-24-a5-d7-short-s9005 | 20/140/202, 20/140/202 | 20/140/212, 20/140/202 | 20/140/208, 20/140/204 |
| hard-24-a6-d9-short-s9008 | 20/180/**278**, 20/180/255 | 20/180/250, 20/180/257 | 20/179/261, 20/180/277 |
| hard-16-a3-d9-short-s9003 | 13/105/115, 13/105/116 | 13/106/113, 13/103/114 | 13/106/113, 13/104/114 |
| hold-16-a4-d7-mixed-s5001 | 13/91/188, 13/91/188 | 13/91/186, 13/91/188 | 13/91/186, 13/91/189 |
| hold-24-a5-d4-mixed-s5013 | 20/80/217, 20/80/218 | 20/80/218, 20/80/218 | 20/80/218, 20/80/218 |
| hold-24-a6-d10-short-s5016 | 20/200/472, 20/200/471 | 20/200/471, 20/200/473 | 20/200/473, 20/200/474 |
| hold-32-a6-d6-mixed-s5023 | 27/162/460, 27/162/433 | 27/162/461, 27/162/457 | 27/162/447, 27/162/460 |
| hold-32-a8-d10-short-s5020 | 27/270/666, 27/270/664 | 27/270/673, 27/270/673 | 27/270/676, 27/270/675 |
| small 9 盤面 | 3 候補全同値（road-heavy-v2 のみ 36/34/33 とゆれ） | | |

- **@10s では hard 盤面で fleet 系が convoy に B で逆転**（s9012: +37〜+56）。holdout 盤面は差 ±3 以内でほぼ縮退。
- convoy@10s の holdout 値は 4f の convoy@20s 値と同一になる盤面が多い（例: s5001 B188、s5013 B217/218）→ **D 飽和盤面では 10s で探索が収束し、予算を追加しても不変**。
- convoy@10s の day max は 11.05s で予算 10s を僅かに超過（harness の日窓内で問題なし、要記録）。総 response_time は convoy が fleet より短い（日後半の自己短縮）。
- chains8 の D-loss 2 件は各 1 点の D 減（V は維持）。B +46（2 seed 合算）と引き換え。

## 3. D: b2-20s-s9012（convoy@20s vs fleet/chains8 @10s、9 盤面）

| 候補 | V/D/B | 総 response_time | day max | s9012 の B |
|---|---|---|---|---|
| convoy@20s | 49 / 217 / 505 | 164.7s | 20.47s | **388** |
| fleet@10s | 49 / 217 / 511 | 170.1s | 10.01s | 396 |
| chains8@10s | 49 / 217 / **530** | 170.1s | 10.05s | **415** |

- convoy@20s vs fleet@10s: W1 D7 L1（敗北は s9012 B388 vs 396 のみ、勝ちは road-heavy-v2 B35 vs 33）。
- convoy@20s vs chains8@10s: W1 D7 L1（敗北は s9012 B388 vs 415）。
- convoy@10s→20s 予算倍増の効果は **s9012 の B +19（369→388）のみ**。small 盤面は不変（road-heavy-v2 は逆に −1）。
- **chains8@10s は convoy@20s を B で上回る**（530 vs 505、s9012 415 vs 388）。#415 の主張（同一予算内の探索量増が hard 盤面で効く）と整合。

## 4. E: c-s2（補給 2 台 kinds S,S,P×6、@10s、17 盤面）

| 候補 | V/D/B | 判定（convoy vs fleet） | 総 response_time |
|---|---|---|---|
| convoy | 209 / 1403 / 2656 | **3勝 10分 4敗** | 581.3s |
| fleet | 209 / 1403 / 2669 | V-loss 0, D-loss 0, B-only 4 | 790.3s |

- 敗北 4 盤面: `road-heavy-v2`（30 vs 31）、`hold-16-a4-d7-mixed-s5001`（133 vs 135）、`hold-32-a6-d6-mixed-s5023`（393 vs 400）、`hard-32-a8-d7-short-s9012`（402 vs 416）。
- 補給 2 台では D 総計が両候補完全同値（1403）。**convoy の区間途中合流メカニズムの優位は消失**（PR #407 の「補給 2 台以上は未検証」に対する最初のデータ: 非退化は確認、優位は未確認）。
- 燃料制約下の hard-16-a3-d9 は kinds 変更の影響が大: 1 補給 D105/B115 → 2 補給 D63/B63（両候補同値）。車両構成の影響が候補差より大きい。

## 5. F/G: convoy 変体（@10s, seed42）

### F: holdout 5 + small 9（13 盤面、既定 = width24 / forecast 合算 / depth8）

| 変体 | V/D/B（13盤面） | W/D/L vs 既定 | 悪化盤面（B-only 全部） |
|---|---|---|---|
| 既定 | 129 / 831 / **2121** | — | — |
| width=12 | 129 / 831 / 2103 | 0勝 9分 4敗 | road-heavy-v2(−2), s5016(−5), s5023(−10), s5013(−1) |
| width=48 | 129 / 831 / 2107 | 1勝 9分 3敗 | road-heavy-v2(−1), s5016(−4), s5023(−13) / 勝 s5020(+4) |
| forecast=persist | 129 / 831 / 2102 | 1勝 9分 3敗 | s5016(−3), s5020(−4), s5023(−13) / 勝 s5013(+1) |
| forecast=busy | 129 / 831 / 2118 | 2勝 8分 3敗 | road-heavy-v2(−2), s5016(−2), s5023(−3) / 勝 s5020(+3), s5013(+1) |

### G: d2（hard s9012 追加、14 盤面）

| 変体 | V/D/B（14盤面） | W/D/L vs 既定 | s9012 の B |
|---|---|---|---|
| 既定 | 156 / 1020 / 2490 | — | 369 |
| persist | 156 / 1020 / 2499 | 3勝 9分 2敗 | 387（+18） |
| busy | 156 / 1020 / **2524** | 4勝 8分 2敗 | **401（+32）** |

- V/D 差は全変体で 0。差はすべて B。
- width は 24 が最良（12/48 とも B 減）。PR #407 の「幅 24 最良、96 で悪化」を 12/48 側からも裏付け。
- **busy 単一予測は hard s9012 で既定（合算）より B +32**。ただし n=1 seed・1 hard 盤面で、holdout 側は ±3 のゆれ。既定変更の根拠には不十分、追試推奨。

## 6. H: 探索量スケーリング（fleet iters 60k → 120k、chains=4、hard 12 + small 8）

| 候補 | V/D/B（20盤面） | W/D/L | 総 response_time | day max |
|---|---|---|---|---|
| fleet 60k | 262 / 1862 / 2982 | — | 116.0s | 3.22s |
| fleet 120k | 262 / **1865** / **3052** | **10勝 8分 2敗** | 348.8s | 9.10s |

- D 改善 4 盤面: `hard-24-a4-d7-short-s9007`（138→139）、`hard-24-a6-d9-short-s9008`（179→180）、`hard-32-a7-d7-short-s9010`（188→189）＋ B 改善多数。B-only 敗北は `hard-16-a3-d9-short-s9003`（112 vs 114）、`hard-16-a5-d10-short-s9004`（259 vs 273）。
- **hard corpus はまだ D 未飽和**（105/117, 138/140, 179/180, 188/189 の D 余白）であり、探索量 2 倍で D 余白の大半が埋まる。B は +70。**Issue #412 の「探索限界 vs 予算不足」の切り分けに対し、hard 盤面は予算不足が主因であることを示す証拠**。
- 代价: 実行時間 3.0 倍（116s→349s）。iters 駆動なので live 予算（10s 窓）での再現性は未確認（→証拠ギャップ）。

## 7. I: smoke-convoy（時計なし budget_ms=0、seed42）

small 9 盤面 + hard-16-a3-d9: 全 10 盤面 success、bad 0、day max 3.64s。hard-16-a3-d9 は V13/D107/B114（4f の convoy@20s は D106/B117 と同水準）。smoke として合格。

## 8. M: 第3対局 solo・観測交通（round1-solo.jsonl、kinds S,P,P,P）

| 候補 | seed | V/D/B | decide_ms（日次） |
|---|---|---|---|
| convoy@10s | 42 | 10 / 45 / **59** | 10013, 5929, 4046, 3366, 2552（後半日は短縮） |
| fleet@10s | 42 | 10 / 43 / 51 | 全日 ≈10s（予算消し切る） |
| chains8@10s | 42 | 10 / 43 / **55** | 全日 ≈10s |
| convoy@10s | 7 | 10 / 46 / 54 | 9606, 5922, 4505, 3189, 2548 |
| fleet@10s | 7 | 10 / 42 / 51 | 全日 ≈10s |
| chains8@10s | 7 | 10 / 43 / **55** | 全日 ≈10s |
| convoy@20s | 42 | 10 / 46 / 54 | 5994, 4994, 4901, 4219, 4016（20s 予算でも日あたり 4〜6s で完了） |
| convoy@20s | 7 | 10 / 46 / 54 | 7174, 5923, 5729, 4340, 4019 |

- illegal / fuel_short / rejected = 0（8 run 全部）。

### 既発行証跡との整合

1. **#415 の B+4 は完全再現**: Mac（4 seed, med）chains4 B51 → chains8 B55。WSL は 2 seed とも chains4 B51 → chains8 B55（D も Mac と同値 43）。match3 盤面に限れば chains8 の効果は確定的。
2. **#407 の第3対局 convoy 値と一致**: Mac 16 seed の二峰「V10/D46/B54 ×11」「V10/D45/B59 ×5」に対し、WSL solo は seed42 が D45/B59、seed7 が D46/B54 — 両峰のどちらにも正確に一致。観測交通 solo 再現性は盤面・候補間で保たれている。
3. **@20s convoy は @10s と同点・以下**: match3 では convoy@20s（D46/B54 ×2 seed）が convoy@10s（D45/B59 s42, D46/B54 s7）に勝っていない。第3対局盤面は 10s でほぼ飽和（decide_ms も後半日 2.5〜5s で自己短縮）。
4. fleet の Mac(#407)「順調交通 D45〜46/B57〜65」は観測交通ではないため直接比較不可（WSL fleet 観測交通は D42〜43/B51）。

## 9. 決策材料（PR #407 / #415 / Issue #412 用）

### 盤面タイプ別の候補強弱

| 盤面タイプ | D 飽和状態 | @10s 最強 | 根拠 |
|---|---|---|---|
| holdout（本選型 mixed/short、16〜32 サイズ） | D ほぼ飽和（差は B-only） | convoy@20s（4f）、@10s では 3 候補ほぼ同点 | B1, C |
| hard（fuel-short、12〜32） | **D 未飽和**（105/117, 138/140, 179/180, 188/189） | **chains8@10s**、iters 増（120k）で D 余白も回収 | C, D, H |
| 第3対局（5 日・実戦型） | D ほぼ飽和（45〜46） | convoy（D で fleet/chains8 に +2〜3） | M |
| 補給 2 台（c-s2） | D 両候補同値 | 差なし（B ±15 以内） | E |
| small 9 盤面 | 飽和 | 全候補同値（road-heavy-v2 以外完全一致） | 全スイート |

### 各設定の作用範囲

- **convoy（#407）**: 利得は (a) @20s での holdout B +50、(b) hard の D 余白回収 4 盤面、(c) 第3対局 D +2〜3。@10s 実戦予算では hard 盤面 B で fleet 系に逆転される（s9012 −37〜−56）。wall-clock は日後半に自己短縮するため総計では fleet を下回る（b10s: 626s vs 791s）が、day max は予算 10s を僅かに超過（11.05s）。@20s では fleet の約 3.7 倍（4f holdout 2452s vs 659s）。V/D 悪化は全スイートで 1 件も未観測（D-loss 1 件: b10s s42 hard-16-a3-d9 D105 vs 106 のみ、V は維持）。
- **chains=8（#415）**: 効果は B 主導（match3 +4、b10s 2 seed +46、s9012 +27/+19）。D は中性〜微小悪化（−1 ×2 盤面）。明示指定時のみ有効で既定無変更。**リスク最小の即効改善**。
- **iters 120k**: hard の D 余白を 4 盤面で回収し B +70。ただし wall-clock 3 倍であり live 10s 窓での実現性未確認。#412 の「予算不足 vs 局所最適」判定: **hard 盤面は予算不足が主因（探索量 2 倍で D/B とも改善）**。局所最適仮説を支持する証拠は本計測にはない。
- **convoy 変体（width / forecast）**: 既定（width24・合算）が最頑健。busy 単一予測は hard 1 盤面で +32 B だが n=1。

### 予算・実行時間の要点

- fleet は iters 駆動でクロスマシン完全決定的。convoy は wall-clock 駆動で実行環境により結果がゆれる（4f と WSL で同盤面 ±数 B）。
- convoy@10s の day max 11.05s（b10s）は予算を僅かに超過。@20s では 20.47s。harness 日窓（60s）内で安全。
- convoy は日後半に自己短縮する（match3 decide_ms: 10.0s → 2.5s）ため、盤面によっては 20s 予算が実効 4〜6s しか使わない。

## 10. 証拠ギャップ（未計測条件）

1. **convoy `fleet_iters=120000` hard 12 盤面** — 実行中（`wsl-corpus/hard-convoy-iters120k/`、`/tmp/setE.log`）。convoy 側の飽和判定はこれまで未確定。
2. **fleet chains=1 holdout 交叉検証**（`holdout-fleet-iters60k-1chain/`）— 証跡・実行プロセスとも未確認。pending として記録。
3. chains=8 の **full corpus（holdout 24 + hard 12）** 計測（現状は holdout 5 + hard 4 × 2 seed のみ）。
4. chains=8 × iters=120k の組み合わせ（探索量と chain 数の効果の分離）。
5. fleet 120k を **live 10s 窓（budget_ms=10000）** で動かした場合の decide_ms 収まり（現状は iters 駆動のみ）。
6. convoy の **@10s × 複数 seed を full corpus** で（b10s は 5+4 盤面のみ。hard 全 12 盤面 @10s の convoy 値は smoke 1 盤面分しかない）。
7. d-forecast=busy の追試（複数 seed・hard 全盤面）。既定変更判断には不足。
8. 補給 2 台（c-s2）の 20s 版・chains8 版。kinds 差と候補差の交互作用は 1 条件のみ。
9. 順調（smooth）交通での WSL 再現（Mac #407 値と直接比較できる条件未取得）。
10. `--kinds-auto` / adapt=on の本番構成での convoy vs fleet vs chains8（本計測は kinds 固定）。
11. Windows 実機・外部 Arena の公式評価値（本集計はオフライン procon-eval のみ）。

## 11. 付録: データ所在

- WSL: `docs/research/round1-20261010/results-405/{wsl-corpus,wsl-10s}/*/runs.jsonl`（各 aggregate.json 併設）
- 4f: `origin/406@b575754 docs/research/round1-20261010/convoy-holdout-406/{holdout,hard,hard-rerun}.runs.jsonl`
- 集計スクリプト: `/tmp/wsl405-analysis/analyze.py`（再集計可能。リポジトリには置いていない）