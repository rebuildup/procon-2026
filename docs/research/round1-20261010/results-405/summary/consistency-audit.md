# results-405 証跡整合性監査（consistency-audit）

- 作成: 2026-10-10（結果回収エージェント）。対象: `measurement-summary.md` の主張と元 JSONL（`results-405/**/runs.jsonl`、`origin/406@b575754 convoy-holdout-406/*.runs.jsonl`、`wsl-10s/round1-solo.jsonl`）の突合。
- 方法: すべての数値を元 JSONL から再集計（`python3 -I`、辞書順 (V,D,B) 降順、`fixture:openapi-sample` 除外）。**measurement-summary.md は本ファイルで訂正値を確定するまで変更していない**。
- 固定 SHA: WSL 側 `6ab5b63...`、e2e のみ `4c72187...`。4f 側は `origin/406@b575754`（source SHA 3d30381 相当）。

## 1. 監査 1: 「convoy の V/D 悪化ゼロ」と b10s seed42 の D −1

**結論: 「悪化ゼロ」は 4f 36 盤面 @20s × fleet(clockless iters) 条件に限定した主張。WSL @10s 条件では D −1 が 1 盤面ある（V は維持）。**

- 4f @20s（holdout24 + hard12 rerun merge）: convoy vs fleet で V-loss 0 / D-loss 0。確定。
- WSL b10s seed42（17 盤面 @10s、fleet は **budget_ms=10000 指定**）: convoy D 合計 1445 vs fleet 1446。D −1 の盤面は `hard-16-a3-d9-short-s9003`（convoy 13/105/115 vs fleet 13/106/113）。
- team-lead 伝言の「convoy 1417 vs fleet 1418」は**数値誤り**。正しくは **1445 vs 1446**（元 JSONL 再計算値）。
- 同一盤面 s9003 の条件別値（元 JSONL）:

| 条件 | convoy | fleet |
|---|---|---|
| 4f @20s convoy / clockless-iters fleet | **D106** B117 | **D105** B114 |
| WSL @10s convoy / budget_ms=10000 fleet | **D105** B115 | **D106** B113 |
| WSL clockless convoy（smoke, budget_ms=0） | D107 B114 | — |
| WSL clockless-iters fleet 120k | — | D105 B112 |

- この盤面は理論上限 117（13×9）に対し D105〜107 でゆれ、**予算・駆動方式の両方に感受性**がある唯一の盤面。4f @20s では convoy が D+1 勝ち、WSL @10s では convoy が D−1 負け。つまり D−1 は「convoy の 10s での探索不足」と「fleet 側が budget_ms 指定時に D が 1 伸びる（clockless とは別経路）」の両方が絡む条件依存差であり、候補固有の D 悪化とは言えない。
- 追加: 4f @20s convoy vs fleet での D 改善 4 盤面（s9003 D106>105、s9007 139>138、s9008 180>179、s9010 189>188）はすべて hard で、**holdout 24 盤面は D 飽和のため差が B-only に完全限定**。

## 2. 監査 2: chains8「D 中性」の範囲（盤面別 D 差分表）

**結論: 「D 中性」は fleet との D 合計比較でのみ成立。盤面別・比較相手別には ±1 の上下があり、convoy との比較では D で劣後（match3 solo では −2〜−3）。**

b10s 両 seed の盤面別 D 差分（dD = 左 − 右、0 以外のみ記載）:

| seed | 盤面 | dD(chains8−fleet) | dD(chains8−convoy) | dD(convoy−fleet) |
|---|---|---|---|---|
| 42 | hard-16-a3-d9-s9003 | 0 | +1 | **−1** |
| 42 | hard-24-a6-d9-s9008 | **−1** | **−1** | 0 |
| 42 | その他 15 盤面 | 0 | 0 | 0 |
| 7 | hard-16-a3-d9-s9003 | **+1** | **−1** | **+2** |
| 7 | その他 16 盤面 | 0 | 0 | 0 |

- D 合計（2 seed 合算）: convoy 2890 / fleet 2889 / chains8 2889。chains8 vs fleet は同値、chains8 vs convoy は −1。
- **match3 solo では convoy 比較の D 劣後が大きい**: convoy D45(s42)/D46(s7) vs chains8 D43 → chains8 は convoy に対し D −2〜−3 を B+4（55 vs 54/59…ただし s42 は convoy B59 が上）と引き換えている。辞書順では match3 solo の勝者は convoy（D が上）。
- chains8「D 中性」が正しいのは「vs fleet（chains4）」という比較に限定される。#415 の PR 主張（「D は med 変わらず」）は fleet 比較なので整合。

## 3. 監査 3: 辞書順スイート勝者と B 合計の区別

辞書順判定は**盤面ごとの (V,D,B) 比較の集計**であり、合計 V/D/B の大小とは独立。各スイートの勝者（per-board W/D/L ベース、両者に勝ち越した候補）:

| スイート | 条件 | 辞書順スイート勝者 | 参考: B 合計 1 位 |
|---|---|---|---|
| b10s seed42 | @10s | **chains8**（vs fleet W3 D11 L3、vs convoy W6 D7 L4） | chains8 B3131 |
| b10s seed7 | @10s | **chains8**（vs fleet W7 D8 L2、vs convoy W7 D8 L2） | chains8 B3144 |
| b2 s9012+small | convoy@20s / 他@10s | **chains8**（vs fleet W1 D8 L0、vs convoy@20s W1 D7 L1） | chains8 B530 |
| c-s2 | @10s kinds S,S,P×6 | **fleet**（convoy は W3 D10 L4） | fleet B2669 |
| 4f holdout24 | @20s | **convoy**（W17 D3 L4） | convoy B8929 |
| 4f hard12 | @20s | **convoy**（W9 D1 L2、rerun merge） | convoy B2915 |
| match3 solo | @10s / @20s | **convoy**（D45-46 > chains8 43 > fleet 42-43） | convoy B59(s42) |

- team-lead 指摘の「seed7 では convoy が D 1 位（1445）だが B 合計は chains8（3144）」を確認。ただし**辞書順スイート勝者は chains8**（convoy は fleet に W1 L5 でしか勝ち越せず、chains8 に 2 勝 8 敗）。D 合計 1 位 ≠ スイート勝者。
- 「convoy が D 合計で上でもスイート全体では負ける」例として b10s seed7 は有用: D の利得（s9003 +2）が 1 盤面のみで、B の損失が hard 2 盤面 + holdout 2 盤面に分散している。

## 4. 監査 4: 測定条件ラベルの棚卸し

元 JSONL の candidate_id から確定した条件。**measurement-summary.md で「fleet」の駆動方式を区別していなかった点が最大の曖昧さ。**

| 証跡 | convoy | fleet |
|---|---|---|
| 4f holdout/hard 36 盤面 | budget_ms=20000（wall-clock） | **iters=60000 clockless**（budget_ms 指定なし・クロスマシン決定的） |
| WSL wsl-corpus（A 系） | smoke のみ budget_ms=0 | iters=60000/120000 clockless chains=4 |
| WSL b10s / b2 / c-s2 | budget_ms=10000 / 20000 | **budget_ms=10000 指定**（wall-clock・非決定的）chains4/8 |
| round1-solo（match3） | budget_ms=10000 / 20000 | budget_ms=10000 **と 20000** の chains4/8 |

- **4f fleet（clockless）と b10s fleet（budget_ms=10000）は別物**。例: s9003 で clockless fleet D105/B114、budget_ms=10000 fleet D106/B113。b10s の比較結果を 4f の結果と直接混ぜてはいけない。
- round1-solo には **chains8@20s（budget_ms=20000:adapt=off:chains=8）が 2 seed 分存在する**（前回報告書では未掲載）。
- 前回報告書のスイート表・各セクションの @10s/@20s/clockless 表記は、candidate_id と照合して誤りなし。ただし「fleet60k-c4」という表記が 4f では clockless、b10s では budget_ms=10000 を指す状態になっており、**A セクションと C セクションの fleet は別条件である旨の注記を追加するべき**。

## 5. 監査 5: budget 超過（実時間）と非決定性の分離

### 5a. day max の予算超過は wall-clock 候補に共通の粒度問題

| 候補 | 予算 | day max | 超過幅 |
|---|---|---|---|
| convoy b10s s42 | 10s | 11.05s | +1.05s |
| convoy b2 | 20s | 20.47s | +0.47s |
| fleet@10s b10s s7 | 10s | 10.71s | **+0.71s（前回報告書では未指摘）** |
| fleet@10s b10s s42 | 10s | 10.04s | +0.04s |
| chains8 b10s | 10s | 10.09s | +0.09s |

- convoy だけが超過しているわけではない。応答時間測定（ハーネス計測値）には solver 内部 deadline チェック粒度＋IO が含まれ、超過幅 0.04〜1.05s は候補差というより実装の deadline 管理差。**全スイートで bad=0・スコアへの影響なし**。アルゴリズム性能差とは区別して記述すべき。

### 5b. wall-clock モード（budget_ms 指定）の非決定性: 直接証拠

同一 candidate × 同一 seed × 同一盤面の重複実行（b10s-seed42 と b2、9 共通盤面）:

| 盤面 | fleet@10s s42 | chains8 s42 |
|---|---|---|
| hard-32-a8-d7-short-s9012 | B406 → B**396**（−10） | B433 → B**415**（−18） |
| road-heavy-v2 | B34 → B33（−1） | B34 → B33（−1） |
| その他 7 盤面（small） | 全 SAME | 全 SAME |

- b2 は 19:18–19:27 実行（setb2.log）、b10s はそれ以前。両者で値が動いたのは探索量が大きい hard 2 盤面のみ。**CPU 負荷・実行タイミングによる wall-clock 変動と結論**（同一盤面・同 seed で B ±10〜18）。
- 対照的に clockless-iters fleet は 4f と WSL の 36 盤面が完全一致（マシン差ですら変動しない）。
- 归结: (1) @10s/@20s の B 差が ±20 未満の盤面別比較は負荷ノイズと同程度（有意性弱い）。(2) スイート内の同時実行比較は有効。(3) **b10s と b2 の絶対値をまたいだ比較（例: chains8 433 vs convoy@20s 388）は −18 の変動込みで解釈が必要**。
- convoy の同条件重複実行は本証跡に存在しない（b10s/c-s2 は kinds が違い比較不能、4f rerun は初回が (0,0,0) で比較不能）→ convoy の負荷非決定性は**未測定**。4f@20s と WSL@10s で同盤面の B がゆれる（後述 6c）のは予算差と環境差が混在した結果であり、分離できない。

## 6. 監査 6: 数値照合（measurement-summary.md vs 元 JSONL）

### 訂正が必要（誤記・不正確）

| # | 報告書の記述 | 元 JSONL の事実 | 訂正 |
|---|---|---|---|
| 6a | 「4f hard fleet の初回 run のみ (0,0,0) bad=1」 | hard-rerun.runs.jsonl は 6 行 = convoy+fleet × {s9002, s9004}。**convoy も s9002・s9004 の 2 盤面が初回 (0,0,0)**（rt=0.0s、ハーネス異常）。fleet は s9002 のみ初回異常、s9004 は first (13,130,273) = rerun 同値 | 「4f hard 初回 run は convoy 2 盤面（s9002, s9004）・fleet 1 盤面（s9002）が (0,0,0)。hard-rerun で convoy/fleet 両方を再取得、fleet s9004 は first と同値」 |
| 6b | H1「D 改善 4 盤面」 | 列挙は s9007(138→139)・s9008(179→180)・s9010(188→189) の **3 盤面**。dD=+3 と一致 | 「D 改善 3 盤面」。また D+3/B+70 は **hard12 のみの寄与**（small 8 盤面は 60k/120k 完全同値、20 盤面合計 = hard12-only 値と同一） |
| 6c | 「@10s では hard 盤面で convoy は fleet 系に s9012 −37〜−56」 | b10s s42: fleet +37 / chains8 +64。s7: fleet +38 / chains8 +31 | 劣後幅は **−31〜−64**（b10s 値。b2 の別条件値と混同しないこと） |
| 6d | 「convoy@10s の holdout 値は 4f convoy@20s 値と同一になる盤面が多い」 | 5 盤面で比較: D は **5/5 完全一致**（91/80/200/162/270）、B は 1/5 のみ一致（s5001 188）。B は s5023 +4、s5020 +4、s5016 +4、s5013 −2（@10s の方が高い盤面も 3 つ） | 「D は全一致、B は 1/5 のみ一致で ±2〜4 ゆれ（wall-clock モードの環境差を含む）」 |
| 6e | 「match3 では convoy@20s が @10s に勝っていない」 | s42: @20s (10,46,54) vs @10s (10,45,59) → **@20s が D+1 で辞書順勝ち**。s7: 両方 (10,46,54) 同値 | 「s42 は @20s が D+1 で勝ち、s7 は同値。ただし convoy@10s s42 の B59 は @20s B54 より 5 上（非決定性/予算差）」 |
| 6f | fleet@10s daymax の超過は convoy のみ指摘 | fleet@10s s7 daymax 10.71s も +0.71s 超過 | 5a の表のとおり補足 |
| 6g | セクション8の match3 表に chains8@20s が無い | round1-solo.jsonl に `budget_ms=20000:chains=8` が 2 seed 分ある: **V10/D43/B55**（@10s と完全同値。decide_ms は全日 ≈20s で予算消し切り） | 追記: **chains8 は match3 で 10s 飽和（20s でも B55/D43 不変）**。辞書順では convoy@20s (10/46/54) > chains8@20s (10/43/55) — 両 seed。20s 予算下の match3 では convoy が辞書順勝者 |

### 照合して問題なし（確定値）

- b10s 2 seed 合計（V418 / convoy D2890 B6135 / fleet D2889 B6229 / chains8 D2889 B6275）✓
- 4f holdout convoy W17 D3 L4、hard rerun merge W9 D1 L2 ✓（再計算で同一）
- H1 20 盤面の V262/D1862→1865/B2982→3052、W10 D8 L2 ✓
- c-s2 両候補 s9003 (13,63,63) kinds S,S,P…、D 総計同値 1403 ✓
- match3 solo の 8 run（前回掲載値）＋ chains8@20s 2 run ✓、bad 0 ✓
- smoke-convoy 10 盤面 bad 0 ✓
- b2 の chains8 vs fleet W1 D8 L0（前回報告書は D2 比較のみで fleet vs chains8 の W/D/L を明示していなかった — 補足）

## 7. まとめ: 3 分類

### 確定できる主張（元 JSONL で再現可能・条件明確）

1. fleet clockless-iters（chains4/8, 60k/120k）はクロスマシン・クロス実行で完全決定的（36 盤面 bit 級一致）。
2. 4f @20s convoy vs fleet(clockless): holdout 17勝3分4敗 / hard 9勝1分2敗、**V/D 悪化 0**、悪化はすべて B-only（holdout）。hard では D 改善 4 盤面あり。
3. #415 の B+4（chains4 51 → chains8 55、D43）は match3 solo 2 seed で完全再現。かつ chains8@20s でも B55/D43 不変（10s 飽和）。Mac 4 seed med と一致。
4. 辞書順スイート勝者は条件ごとに異なる: b10s/b2 = chains8、4f corpus = convoy、c-s2 = fleet、match3 solo = convoy。
5. 補給 2 台（c-s2）では convoy 優位が消失（D 完全同値、W3 L4）。s9003 は両候補 (13,63,63)。
6. hard corpus は D 未飽和（余白 105/117, 138/140, 179/180, 188/189）で、iters 2 倍（120k）により D 3 盤面 +1・B +70（hard12 のみ、small は無変化）。
7. small 9 盤面は候補・予算・マシン差にほぼ不感受（road-heavy-v2 のみ ±1〜3）。
8. wall-clock モード（budget_ms 指定）の fleet/chains8 は同条件再実行で hard 盤面 B ±10〜18 変動（b10s vs b2 の重複で実証）。

### 条件注記が必要な主張

1. 「convoy V/D 悪化ゼロ」→ **4f @20s × fleet-clockless に限定**。WSL @10s × fleet-budget_ms では D−1 ×1 盤面（s9003、V 維持）。
2. chains8「D 中性」→ **vs fleet に限定**。vs convoy では D −1（b10s 2seed）〜 −2/−3（match3 solo）。
3. b10s と b2 の絶対値比較 → wall-clock 変動 ±10〜18 を含む。スイート内比較でのみ有効。
4. convoy daymax 11.05s/20.47s → 候補固有の問題ではなく wall-clock 候補共通の粒度超過（fleet も +0.71s）。スコア影響なし。
5. match3 solo の結果 → solo・観測交通・kinds S,P,P,P 固定の条件値（対戦交通・kinds-auto とは別条件）。
6. iters120k の D+3/B+70 → hard12 のみの寄与。live 10s 窓（budget_ms）では未検証。
7. convoy@10s holdout 値と 4f@20s 値 → D は 5/5 一致だが B は 1/5 のみ一致（wall-clock 環境差が混入）。

### 訂正が必要な主張

- 6a〜6g の 7 件（上表のとおり）。特に 6a（(0,0,0) の候補範囲）、6b（D 改善 3 盤面・hard12 のみの寄与）、6g（chains8@20s の存在と「20s 予算下の match3 では convoy が辞書順勝者」）は意思決定に影響し得る。

## 8. 付録: 検証スクリプト

- 集計: `/tmp/wsl405-analysis/analyze.py`（前回）・`/tmp/wsl405-analysis/audit.py`（本監査）。いずれも `python3 -I` で再実行可能。リポジトリには未保存。
- 本ファイルのみをリポジトリに書き込み済み（`results-405/summary/consistency-audit.md`）。git 操作は実施していない。