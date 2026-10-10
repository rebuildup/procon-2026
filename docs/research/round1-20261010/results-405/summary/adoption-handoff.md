# 採用判定ハンドオフ — convoy / fleet 現行 / fleet chains8（2026-10-10）

作成: 採用判定担当（Issue/PR agent への引き渡し用）。
判定は公式辞書順 **V → D → B**（-T は参考値）で行う。

## 1. 固定ソース SHA

| ソース | SHA | 内容 |
|---|---|---|
| convoy | `3d30381` | `origin/405` 先頭。PR #407 本体 |
| fleet chains8 | `e79d786` | `origin/409` 先頭。PR #415 |
| merge + 計測 | `4c72187`（merge）／`20d0ae0`（計測データ commit） | 本 worktree `eval-convoy-415`。convoy@3d30381 と chains8@e79d786 の merge に WSL 計測 results を追加したもの。本ドキュメントの再集計はすべて `4c72187` のバイナリによる |
| 406 セッション証跡 | `6ab5b63` | `origin/406` `docs/research/round1-20261010/convoy-holdout-406/` |

## 2. エビデンス一覧

### 2.1 既出（406 セッション / PR #407・#415 / origin/403）

- **holdout 36 盤面（fleet-371 corpus-hard 12 + corpus-holdout 24、budget_ms=20000、補給 1 台、seed42）**: convoy 26W / 4T / 6L（V→D→B 辞書順）。D が下がった盤面 0、B 合計 +99（hard: D 1838 対 1834 / B 2915 対 2866、holdout: D 3452 対 3452 / B 8929 対 8879）。違法・燃料不足・拒否 0。
- **予算感度（逐次実行）**: 20s で 7/2/1、10s で 5/1/4（s9007 は D −1）。**24×24 以上では 10 秒は足りない**。
- **回答時間**: convoy@20s の 1 日最大 20.4〜21.0 秒（高負荷下、budget を最大 1.0 秒超過）。@10s は 10.5 秒。
- **PR #415**: match3 chains8 B med 51→55。
- **origin/403 analysis.md §6-7**: convoy の退避 fleet 計画 pool[0] は clocked 時 window/3 + budget_ms/4（上限 4s）の弱計画。D 飽和盤面の beam B 予測誤差は副次要因。

### 2.2 WSL 独立計測（results-405/、raw runs.jsonl を再集計した値）

**b10s（18 fixtures × 3 候補 @10s）— 全 18 盤面合計**

| 候補 | seed42 V/D/B | seed7 V/D/B |
|---|---|---|
| convoy@10s | 211 / 1447 / 3090 | 211 / **1447** / 3055 |
| fleet 現行 | 211 / **1448** / 3111 | 211 / 1445 / 3128 |
| chains8 | 211 / 1447 / **3136** | 211 / 1446 / **3149** |

- V は全候補同点。**D は seed で逆転する**: seed42 は fleet +1、seed7 は convoy +2。差は 1〜2 点で、辞書順ではこの差が優先される。
- 9 corpus 盤面（hold/hard）のみの B 合計: seed42 convoy 2967 < fleet 2990 < chains8 3015。seed7 convoy 2934 < fleet 3007 < chains8 3029。
- 32×32 の 2 盤面（s9012 / s5020）で convoy B −24〜−38（D 飽和盤面）。
- 盤面別の chains8 vs fleet 頭対頭: seed42 3W3L、seed7 7W1L（s9008 は chains8 が D −1 で負け、seed7 s9003 は D +1 で勝ち）。chains8 は盤面ごとの D を ±1 動かすことがある。
- 回答時間: convoy max daily 11.05s（seed42）/ 10.71s（seed7）。fleet 10.04s、chains8 10.03s。

**b2-20s（s9012、32×32 hard）**: convoy@20s B388 < fleet@10s 396 < chains8 415（D 189 同点）。**20s でも convoy は D 飽和盤面の B では負ける**。

**c-s2（2 補給 kinds S,S,P… @10s seed42）**: convoy vs fleet（同 kinds）は 9 corpus で 3W / 3L / 3T、B −12。1 補給構成（b10s）との D 差は −42（巡回車が減る構造差）。rider の illegal / fuel_short は 0。

**d-*/d2-*（width・forecast 汎化、default=width24・both 比較）**: width12 −18、width48 −14、forecast=busy −3（14 盤面）/ +34（s9012 を含む 15 盤面、そのうち +32 が s9012）、forecast=persist −19。**default が最良かほぼ同等**。busy の s9012 単盤面 +32 は 1 盤面の信号であり採用根拠にしない。

**wsl-corpus（fleet iters 比較、hard 21 盤面）**: iters 60000→120000 で D 1864→1867（+3）、B 2987→3057（+70）。max daily 3.22s→9.10s（10s 収まる）。※共有メモの D 1834→1837 / B 2866→2936 は 406 セッションの 12 盤面集合の数値で、盤面集合が異なる。

**round1-solo（match3 実戦形状・観測交通）**

| 候補 | s42 V/D/B | s7 V/D/B | decide 最大 |
|---|---|---|---|
| convoy@10s | 10 / **45** / 59 | 10 / **46** / 54 | 10.013s（初日、一時超過） |
| fleet 現行 | 10 / 43 / 51 | 10 / 42 / 51 | 10.027s |
| chains8 | 10 / 43 / 55 | 10 / 43 / 55 | 10.027s |
| convoy@20s | 10 / **46** / 54 | 10 / **46** / 54 | 7.174s |

- match3 では **convoy が両 seed で D 最上位**。@20s にすると s42 は D +1（45→46）で辞書順も強化、B は −5。

### 2.3 実行中（判定時に結果未確認 →「未検証」扱い）

convoy fleet_iters=120000 hard、convoy@20s 9 盤面展開、2 補給@20s。

## 3. 三分類判定表（公式辞書順 V→D→B）

| 候補 | 分類 | 根拠 | 条件 |
|---|---|---|---|
| **fleet 現行**（13a0106 標準: `budget=official:budget_ms=10000:adapt=off` + `--kinds-auto`） | **本番候補（フォールバック）** | seed42 corpus D 1 位、match3 solo D 43（3 位）だが安定・予算超過 ~0.1s のみ。既存の投入実績 | なし（現状維持） |
| **fleet chains8**（PR #415） | **本番候補** | B 全 corpus で最上位（+25〜+91 vs 現行）。V/D は盤面ごと ±1 のみ（2 seed 合計で D 現行と同点、盤面別頭対頭 10W4L）。solo match3 は D 同点 B +4。seed42 で s9008 の D −1 が唯一の辞書順リスク | なし。D 変動 ±1 の監視は継続 |
| **convoy**（budget_ms=20000、補給 1 台） | **条件付き採用候補** | match3 solo 両 seed D 最上位（45/46 vs 43）。holdout 36 盤面 D 下落 0・B +99。seed7 corpus は D 1 位。W: D 未飽和・燃料制約盤面で辞書順 1 位になり得る唯一の候補 | ① budget_ms=20000 必須（10s は 24×24 以上で D を落とす）② kinds 補給 1 台 ③ deadline 裕度監視（負荷時 +1.0s 超過実績）④ D 飽和盤面では B で chains8 に負けることを許容 |
| **convoy@10s** | **棄却** | 24×24 以上で B 不足・s9007 D −1。match3 solo でも s42 decide 10.013s と一時超過 | — |
| **2 補給 kinds**（c-s2） | **棄却** | 同 kinds fleet と D 同点でも B −12。1 補給構成比 D −42（構造差）。kinds-auto が 2 補給を選択しない | —（rider 自体の illegal/fuel 0 は確認済み） |
| **width/forecast 変更**（width12/48、busy、persist） | **棄却** | default（width24・both）が最良かほぼ同等。busy の s9012 +32 は 1 盤面のみ | — |

## 4. 確認済み / 未検証の分離

**確認済み**
- V は全候補・全計測で同点（差分 0）。
- convoy の D 下落は 406 セッション 36 盤面で 0、WSL seed7 corpus では D +2/+1。
- chains8 の B 優位は 2 seed × 18 盤面で一貫（B 合計 +25/+21 vs 現行）。
- convoy の退避 fleet pool[0] 弱計画（clocked 時）と D 飽和時 beam B 予測誤差のメカニズム。
- rider の安全性（illegal / fuel_short / rejected 0）。
- 10s では 24×24 以上の盤面で convoy 予算が不足。

**未検証（採用判断に残る不確実性）**
1. seed 魯棒性 — D の seed 間逆転が 1〜2 点で起きており、2 seed では不十分。
2. 他チームが動く試合（対戦交通）での挙動 — solo 観測交通のみ。
3. Windows native での回答時間（+1.0s 超過が公式環境でどう出るか）。
4. rider@20s、2 補給@20s（実行中）。
5. kinds-auto が convoy 有利盤面を判別できるか（選盤 gate）。D 飽和判別 gate は未実装。
6. forecast=busy の s9012 +32 の再現性。
7. convoy fleet_iters=120000 hard（実行中）。

## 5. Issue/PR agent への引き渡し

- **Candidate ID（実行可能なもの）**
  - chains8: `fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=8`（`--chains 8` + `--kinds-auto`）
  - convoy: `convoy:budget=official:budget_ms=20000:seed=42`（`--budget-ms 20000`、kinds `S,P,P,P…` 補給 1 台、`--kinds-auto` 併用）
  - fallback: `fleet:budget=official:budget_ms=10000:adapt=off`（13a0106 標準）
- **kinds 制約**: convoy は補給 1 台（2 補給は棄却済み）。全車巡回・交互構成は 406 記録を参照（交互 26W4L6 だが本番は kinds-auto 前提）。
- **deadline 裕度**: convoy@20s は負荷下で最大 1.0 秒超過（20.4〜21.0s 実績）。@10s でも 11.05〜11.51s の観測あり。chains8/fleet は 10.03〜10.09s とほぼ収まる。convoy を出す場合は超過監視と budget_ms の更なるヘッドルーム検討を PR に明記。
- **フォールバック**: 判定が確定しない盤面・環境差が出た場合は fleet 現行（chains8 を含めるなら PR #415 採用後の chains8）に戻す。
- **既知リスク**: ① rider 2 補給の D −42（kinds-auto 判別が必須）② 予算超過 ~1s（負荷時）③ D 飽和盤面の beam B 予測誤差 ④ chains8 の盤面別 D ±1 ⑤ seed 逆転（D 差 1〜2 点）⑥ Windows native 未計測。

## 6. 次の検証優先度（測定責任者への指示）

1. **P0 — convoy@20s × seed5 本以上 × 9 corpus**（V/D 逆転の頻度を確定。辞書順 1 位率を chains8 と比較）
2. **P0 — 実行中 3 計測の回収**（convoy iters=120k hard、convoy@20s 9 盤面、2 補給@20s）→ このドキュメントの判定を更新
3. **P1 — Windows native での convoy@20s 回答時間**（公式環境の超過幅確定。DB がダメなら budget ヘッドルーム案）
4. **P1 — 対戦交通（他チームが動く試合）での convoy@20s vs chains8**（match3 形状 + 別形状 1 つ）
5. **P2 — forecast=busy を hard-32 系（s9012 以外の 32×32）で再確認**（+32 が再現するか。再現すれば D 飽和盤面限定の部分採用候補）
6. **P2 — kinds-auto / D 飽和判別 gate の仕様確定**（実装可能かの feasibility チェックのみでよい）
7. **P3 — rider@20s の安全性確認**（illegal/fuel 0 の再確認）