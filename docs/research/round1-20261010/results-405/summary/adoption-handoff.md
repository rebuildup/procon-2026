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

### 2.3 実行中（§7 時点ではすべて回収済み — 結果は §7 に反映）

convoy fleet_iters=120000 hard、convoy@20s 9 盤面展開、2 補給@20s。→ 確定結果: 120k は予算不適合で棄却（§7.2）、2 補給@20s も B −8 で棄却維持（`wsl-10s/c2-20s-s2/`）、convoy@20s 展開は §7.3-3 の負荷ノイズ修正の根拠（`wsl-10s/b3-20s-seed42/`）。

## 3. 三分類判定表（公式辞書順 V→D→B）

| 候補 | 分類 | 根拠 | 条件 |
|---|---|---|---|
| **fleet 現行**（13a0106 標準: `budget=official:budget_ms=10000:adapt=off` + `--kinds-auto`） | **本番候補（フォールバック）** | seed42 corpus D 1 位、match3 solo D 43（3 位）だが安定・予算超過 ~0.1s のみ。既存の投入実績 | なし（現状維持） |
| **fleet chains8**（PR #415） | **本番候補** | B 全 corpus で最上位（+25〜+91 vs 現行）。V/D は盤面ごと ±1 のみ（2 seed 合計で D 現行と同点、盤面別頭対頭 10W4L）。solo match3 は D 同点 B +4。seed42 で s9008 の D −1 が唯一の辞書順リスク | なし。D 変動 ±1 の監視は継続 |
| **convoy**（budget_ms=20000、補給 1 台） | **条件付き採用候補** | match3 solo 両 seed D 最上位（45/46 vs 43）。holdout 36 盤面 D 下落 0・B +99。seed7 corpus は D 1 位。W: D 未飽和・燃料制約盤面で辞書順 1 位になり得る唯一の候補 | ① budget_ms=20000 必須（10s は 24×24 以上で D を落とす）② kinds 補給 1 台 ③ deadline 裕度監視（負荷時 +1.0s 超過実績）④ D 飽和盤面では B で chains8 に負けることを許容 |
| **convoy@10s** | **棄却** | 24×24 以上で B 不足・s9007 D −1。match3 solo でも s42 decide 10.013s と一時超過 | — |
| **2 補給 kinds**（c-s2） | **棄却** | 同 kinds fleet と D 同点でも B −12。1 補給構成比 D −42（構造差）。kinds-auto が 2 補給を選択しない | —（rider 自体の illegal/fuel 0 は確認済み） |
| **width/forecast 変更**（width12/48、busy、persist） | **棄却** | default（width24・both）が最良かほぼ同等。busy の s9012 +32 は 1 盤面のみ | — |

**【確定版での三分類更新 — PR #407 途中報告 16 と整合。上表の証跡は温存】**
最終分類は §7.2/§7.5 に基づき次のとおり: **chains8 = 採用推奨**（full corpus D 下落 0・maxdaily 10.0〜10.2s で実時間・辞書順の両条件が確定。旧「本番候補」から昇格確定）／**convoy = 改善継続**（旧「条件付き採用候補」→ 対戦交通・seed 魯棒性・kinds-auto 併用が未確認のため条件付き採用には届かず。残タスクは beam 時間配分の再設計）／**convoy@10s hard 構成・fleet_iters=120000・2 補給 kinds・persist 既定化・width 変更 = 棄却・保留**（120k は `wsl-corpus/hard-convoy-iters120k/` の maxdaily 52.24s で予算不適合が確定）。fleet 現行はフォールバックとして維持。

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

（確定版の注意: 採用推奨は chains8、convoy は改善継続 — §7.2。本節の convoy 関連の条件・リスクは convoy を将来投入する場合の前提として読むこと）

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

---

## 7. 最終判定（**確定版 — 2026-10-10、全計測回収後。PR #407 途中報告 16 と整合**）

前半（§1-6）の判定と証跡は温存。本節が最終版で、§7.2 の三分類は PR #407 途中報告 16 と整合している。
本節の P1 / P3 は**測定責任者の計画番号**であり、§6 の優先度番号とは別物なので混同しないこと。

### 7.1 実戦条件の定義と重み付け

実戦に近い条件を次の 3 つとし、判定はこの順で重み付けする。

| 条件 | 内容 |
|---|---|
| (a) 構成 | `--kinds-auto` + `budget_ms=10000` の現行投入構成で動くこと |
| (b) 交通 | 対戦交通（他チームが動く試合）での優位 |
| (c) 期限 | 60s wall-clock 期限に対する余裕 |

判定原則: **実戦条件に適合する証拠を優先し、offline-only の強さ（clockless 決定論優位、solo 観測交通での優位等）は採用根拠の重みを下げる**。未検証の優位性は確定として扱わない（引用には必ず証跡パスを添える）。

### 7.2 候補別の判定基準表（確定）

凡例: ✅ = 条件確認済み（証跡あり）、⬜ = 未確認、⚠ = 確認したが条件付き/懸念あり。

| 判定クラス | convoy | chains8 | fleet 現行 |
|---|---|---|---|
| **採用推奨**（実時間制約と辞書順優位を確認済み） | 実時間: solo@20s ≤7.2s ✅（`wsl-10s/round1-solo.jsonl`）／負荷時超過 ⚠（b3 maxdaily 23.71s を含む、§7.3-2）。辞書順: holdout D 下落 0・B+99 ✅、match3 solo D 最上位 ✅。**対戦交通・seed 魯棒性・kinds-auto 併用が未確認のため採用推奨には届かない → 改善継続** | **現時点の位置 = 採用推奨**。実時間: full corpus maxdaily 10.0〜10.2s ✅（`wsl-10s/b4-chains8-full/`）。辞書順: hard 12 盤面 D1837 vs 1834・B+92、holdout 24 盤面 D3452 同値・B+104、**D 下落盤面 0** ✅。B 優位の大きさは負荷依存（§7.3-3）だが、robust な match3 B+4 と V/D・deadline 中立性で downside なし | 実戦条件 (a)(b)(c) 全適合 ✅（現行投入標準 13a0106）。chains8 採用時のフォールバック |
| **改善継続**（優位だが条件依存・未検証） | **現時点の位置**。残タスクは **beam 時間配分の再設計**（P3 実測: persist で s9012 B+18〜38、`wsl-10s/p3-persist-s9012/` — beam が時間配分で律速されていることが実証済み）。他の未検証: 対戦交通（⬜）、seed 魯棒性（⬜）、kinds-auto 併用動作（⬜） | 対戦交通（⬜）のみ残る（採用推奨の残課題） | chains8 置換検証は完了（B 優位は負荷依存のため、置換根拠は match3 B+4 と V/D・deadline 中立性） |
| **棄却・保留**（悪化確認 or 制約不適合） | convoy@10s hard 構成（D −1 実測: s9003 `wsl-10s/b10s-seed42`、s9007 `origin/406 sequential-budget`）／2 補給 kinds（D 同点でも B −8〜−12、1 補給比 D −42。@20s 再確認: `wsl-10s/c2-20s-s2/`）／**fleet_iters=120000**（maxdaily 52.24s で 10s 予算不適合・D 崩落、`wsl-corpus/hard-convoy-iters120k/`）／persist 既定化（default 最良、busy は s9012 単盤面のみ）／width 変更（default 最良） | 該当なし | 該当なし |

chains8 full corpus の測定条件注記: baseline は決定論的な `fleet iters=60000 chains=4`（clockless）、chains8 側は `budget_ms=10000`（clocked）で条件差がある。D 比較には影響しない（clocked 側が不利になる方向）ため「D 下落 0」の判定は保全される。

### 7.3 明示的に反映するリスク項目（監査値）

1. **V/D 悪化**
   - convoy: clockless@20s では **0**（`origin/406 convoy-holdout-406/` 36 盤面）。@10s では s9003 D−1（`wsl-10s/b10s-seed42/runs.jsonl`: convoy D105 vs fleet 106）＋s9007 D−1（`origin/406 .../sequential-budget-10s-20s.runs.jsonl`）。
   - chains8: 盤面別 **±1**（seed42 s9008 D−1、seed7 s9003 D+1 — `wsl-10s/b10s-*/runs.jsonl`）。
2. **wall-clock 予算超過**（60s 期限への余裕とセットで評価）
   - 監査値: convoy **+0.47〜1.05s**（下端 = WSL b2-20s maxdaily 20.47s、上端 = 406 高負荷 21.0s）、chains8/fleet **+0.1〜0.7s**。
   - 追加観測: b3 逐次でも convoy@20s maxdaily 23.71s（+3.71s、`wsl-10s/b3-20s-seed42/`）と予算を大きく超過した日がある。chains8 full corpus は 10.0〜10.2s に収束（`wsl-10s/b4-chains8-full/`）。
   - 60s 期限: 1 日あたりの最大超過 × 5 日 = 累積最大 ~5.25s（監査値ベース、b3 観測を入れると ~18.5s）を期限余裕から差し引いて評価する。convoy 採用時は超過監視を必須化。
3. **B の実行間変動（負荷ノイズ修正 — 確定）**
   - 負荷下で **±10〜18**（`origin/406`: jobs=3 並走時の負けは CPU 競合と記録）。clockless は決定論。
   - **低負荷再測（b3）で B 順位が反転した**: s9012 は b2（高負荷）で convoy 388 < fleet 396 < chains8 415 だったが b3 で convoy 412 > fleet 407 > chains8 403（`wsl-10s/b2-20s-s9012/` vs `wsl-10s/b3-20s-seed42/`）。chains8 の corpus B 優位も反転（b10s: chains8 3136 > fleet 3111 → b3: chains8 3111 < fleet 3124）。**つまり s9012 劣勢も chains8 の corpus B 優位も負荷依存であり、B 差の多くは負荷ノイズ**。
   - **robust な根拠は match3 B+4（chains8 55 vs fleet 51、`wsl-10s/round1-solo.jsonl`）と convoy の match3 型 D 改善（45/46 vs 43）のみ**。B 差 ±20 未満は有意性が弱いとして、盤面別判断は常に V/D が先行する。

### 7.4 実戦条件適合の証拠確認

| 候補 | (a) kinds-auto + 10s 構成 | (b) 対戦交通 | (c) 60s 期限 |
|---|---|---|---|
| convoy | ⬜ kinds 指定 `S,P,P,P` 前提。`--kinds-auto` 併用での選盤動作は未検証 | ⬜ solo 観測交通のみ（`wsl-10s/round1-solo.jsonl`） | ⚠ 超過累積 ~5.25s を見込む必要（§7.3-2） |
| chains8 | ✅ `budget_ms=10000` で動作（`wsl-10s/b10s-*/runs.jsonl`） | ⬜ 未計測 | ✅ 超過小（負荷時 +0.1〜0.7s） |
| fleet 現行 | ✅ 現行投入標準（Arena 13a0106） | ✅ 投入実績あり | ✅ |

offline-only の強さの重み下げ適用例: convoy の match3 solo D 最上位（`round1-solo.jsonl`）と holdout B+99（`origin/406`）は**決定打には使えない**。convoy の採用は対戦交通での D 優位確認（P0/P1 計測）が揃って初めて確定する。

### 7.5 最終判定の分岐表（確定 — 実測で全分岐が解決）

| 分岐トリガ | 実測結果 | 最終判定 |
|---|---|---|
| **P1（embed ラダー）** | **外れ（実測確定）** — `wsl-10s/p1-embed-rung/` では想定の優位は確認されず | 現行 convoy のまま。追加修正は保留 |
| **P3（persist）** | **+25 以上のバンドに実測で到達（実測確定）** — persist で s9012 B+18〜38（`wsl-10s/p3-persist-s9012/`: s9012 B407 vs default 369 = +38） | **beam 時間配分の再設計**が convoy 改善継続の残タスクとして確定（beam が時間配分で律速されていることが実証） |
| **chains8 full corpus** | **D 下落盤面 0（実測確定）** — hard D1837 vs 1834・B+92、holdout D3452 同値・B+104、maxdaily 10.0〜10.2s（`wsl-10s/b4-chains8-full/`） | **chains8 = 採用推奨として確定**（実時間・辞書順の両条件が揃った） |