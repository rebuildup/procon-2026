# パラメータ最適化の分析枠（バッチ 1 準備・既存データのみの棚卸し）

2026-10-10。測定実行はしていない。measurer2 のバッチ 1（chains ラダー offline・chains8 seed 魯棒性・clocked 予算カーブ・convoy width/depth スクリーニング）の結果は `results-405/wsl-corpus/param-*` と `wsl-10s/param-*` に順次出る想定で、本書はその分析の受け皿を作るもの。

**判定ルール（本書とバッチ分析で共通）**:
- 採用判定は公式辞書順（V → daily_varieties_sum → B）。**D（dV）非劣化を必ず監視**する。B が増えても dV が 1 でも下がるセルは採用候補から外して個別審査する。
- B 差 ±20 未満は有意性が弱い（同一候補再測定ノイズ 0–7 balls、異バッチ間 ±10–18。出典: `d-forecast=*` vs `d2-*` の同候補再測定、b10s vs b2 の fleet@10s 406↔396 / chains8 433↔415）。
- **スクリーニング 9 盤面**（b10s の hard/hold 9: s9003, s9005, s9008, s9012, s5001, s5013, s5016, s5023, s5020）で選んだ設定は、**検証 27 盤面**での再確認まで採用推薦しない。match3 系単独データ（`convoy-shape.jsonl` / `convoy-seeds.jsonl`）は 16×16・S,P,P,P 特化の補助証拠としてのみ扱う。
- clockless は決定論なので 1 実行で確定。clocked は seed を変えた 2 実行を推奨。

## 1. 調整可能パラメータ全体表

### 1.1 fleet（`crates/bench/src/adapters/fleet_solver.rs:44-64`）

| パラメータ | 既定 | 役割 | 実測感度 | 未測定・注意 |
|---|---|---|---|---|
| `budget` | 必須 official | 検証ゲート。official 以外はビルド拒否 | — | — |
| `iters` | 20000 | offline 決定論探索長（1 日あたり）。live（budget_ms>0）では不発動 | hard corpus 60k→120k: B +1〜+32（s9009 +32, s9012 +17）、**dV +1 が 2 盤面**（s9007 138→139, s9010 188→189）。16×16 は ±。wall は 2.6–3.1 倍 | holdout 27 盤面は 60k のみ。chains 数との積で効く |
| `budget_ms` | 0（=clockless） | live の wall-clock cap（1..59000）。指定すると iters を無効化 | 収束目安は 16×16 ≈0.25s / 24×24 ≈2s / **32×32 は 4s でも伸びる**（`fleet_solver.rs:1194-1197`）。実測は @10s 単点（B 406–425, s9012） | **予算カーブは未測定** → バッチ 1。chains スレッド並列と window 共有（計画フェーズ 7/10 分割, `:1229`）に相互作用 |
| `adapt` | on | budget_ms>0 のとき収束推定で早期打ち切り | **未測定**（既存 live 測定はすべて adapt=off） | convoy 側 embedded は adapt=false 固定（`convoy_solver.rs:133`）。小盤面で回答短縮（第 4 軸）を狙うが B への影響不明 |
| `seed` | 42 | 探索 seed。`seed ^ (day<<32) ^ (k<<48)` で日別決定論 | @10s s9012: s42=406 vs s7=425（同バッチ別実行, ±10–20 域） | chains 数を増やすと chain 内 seed 分散の相対影響は減る想定。chains8 seed 魯棒性 → バッチ 1 |
| `chains` | 1 | 探索 chain 数（1..16）。2+ で明日計画の比較が有効化。offline は逐次、live は 1 chain/core（明示時は上限引き上げ, Issue #409） | **clockless: chains4 ≫ chains1**（holdout 33 case: 20 勝 0 敗 13 分, B 合計 +174）。**@10s: chains8 vs chains4 = 3 勝 3 敗 12 分, B +25**。32×32 の公式勝者は chains8 4/6 | chains12/16 未測定 → バッチ 1。live 上限は core 数 |

### 1.2 convoy（`crates/bench/src/adapters/convoy_solver.rs:48-87`）

| パラメータ | 既定 | 役割 | 実測感度 | 未測定・注意 |
|---|---|---|---|---|
| `budget_ms` | 0（=clockless 決定論） | live wall-clock cap。embedded fleet に (budget/4).min(4000)、pool 生成に 1/3、beam に残り | s9012: @10s B369 → @20s B388（embedded 2.5→5s）。match3: @10s (10,45,59) → @20s (10,46,54)（**B −5 でも dV +1 で公式勝ち**） | 32×32 では beam 不完走 → 実質 fleet@budget/4（`analysis/convoy-clocked-degradation.md`）。40s 以上も embedded は 4s cap |
| `fleet_iters` | 60000 | embedded fleet の offline 探索長。**live では不発動** | clockless: 60k は 16×16 で強い（honsen-like 17 勝 2 敗 8 分）。120k は 16×16 mixed・**24×24-a5+ と 32×32 で harness 425s timeout**（`wsl-corpus/hard-convoy-iters120k/`） | clocked では原理的に無意味（時間切れ支配）。clockless の大盤面は構造的に timeout（S3） |
| `fleet_chains` | 4 | embedded fleet の chain 数 | **未スイープ** | live は min(cores, chains)。budget カーブ実験と独立に 1 段階だけ確認する価値あり |
| `seed` | 42 | embedded fleet seed（本体 fleet と同一派生） | match3: 16 seed で B 54–59 の**双峰**（`convoy-seeds.jsonl`）。s9012: s42=369 vs s7=387（±18） | 双峰の D45/D46 は dV 差。seed 平均取りは公式辞書順に整合しない点に注意 |
| `depth` | 8 | 比較時に先読みする日数（0=当日のみ） | match3: d0 (10,40,54) → d1 (10,45,59) → d2 (10,45,60) → d3 (10,45,59)。**depth≥1 が必須、2–3 で飽和**。既定 8 と同等 | 24/32×32 では残り日数と時間配分で実質 cap（S3）。corpus での depth ラダー未測定 → バッチ 1 |
| `forecast` | both | 先読日の道路予測: persist / busy / both（合算） | @10s 14 board: both が best 11・persist best 1（s5013 +1）・busy best 1（s5020 +3）。**既定 both から動かす根拠なし** | both は scenario 数 2 → beam 時間半分ずつ。32×32 では scenario 削減が完走率を上げるが B 利得は未確認（P3 で検証予定） |
| `width` | 24 | beam の日末状態保持数 | match3: w1 (10,41,52) / w4 42 / w8 44 / **w24 (10,45,59)** / w48 同 / w96 (10,43,53) 劣化。corpus @10s: w12 は s5023 −10・s5016 −5、w48 は s5020 +4・s5023 −13 | 24 維持が現データ最良。w48 の 32×32 逆転は ±20 内で再現性未確認 |
| `labels` | 1,500,000 | 1 巡回日 label search の上限 | **未測定**。到達時は pool 崩壊（S2・`convoy_solver.rs:461-465`） | 増やすと時間超過リスク、減らすと pool 崩壊前倒し。単独スイープの優先度は低い |

### 1.3 内部固定値（パラメータ化されていない = §2 の構造課題）

| 定数 | 位置 | 内容 |
|---|---|---|
| `CHAIN_T0[16]` | `fleet_solver.rs:142-148` | chain 初期温度配列（40–1000 の決め打ち、`k % 16` 循環） |
| embedded fleet 予算 | `convoy_solver.rs:130` | `(budget_ms/4).min(4000)` の比率と cap がハードコード |
| pool 生成 deadline | `convoy_solver.rs:1119` | window の 1/3。scenario 均等分割 `:1128-1133` もハードコード |
| lookahead fleet | `convoy_solver.rs:135` | `iters=20000, chains=1, adapt=true` 固定 |
| deadline 粒度 | `convoy_solver.rs:461-465` | label search の時間チェックが 8192 labels ごと（超過オーバーランの源、実測 day0 +0.9–1.0s） |
| beam root 保護 | `convoy_solver.rs:1049-1052` | root==0（fleet 由来経路）を常に beam 内保持 |

## 2. コード変更が必要な構造課題（実装担当への引き渡し形式）

### S1. SA 温度スケジュール（CHAIN_T0）の内部固定

- **課題**: chain 間多様性の源流である初期温度配列 `CHAIN_T0[16]` が内部定数で、chains 数を増やしても温度プロファイルは `k % 16` の循環でしか増えない。
- **根拠**: `fleet_solver.rs:142-148`。chains8 の利得（32×32 公式 4/6、B +25）が温度多様性によるものか chains 数そのものによるものか未分解。chains12/16 実験はこの配列の全 16 温度を使うため、ラダー結果に温度効果が混入する。
- **期待利得**: 温度多様化だけで chains8 相当を chains4 で得る、または chains12/16 の利得予測の分解。32×32 で chains8 − chains4 = +25 balls の増幅余地。
- **検証条件**: offline 9 スクリーニング盤面・chains4 固定・iters60000 決定論で T0 配列 A/B（現行 vs 広範囲サンプリング版）。辞書順で D 非劣化を確認し、B 差は ±20 基準で判定。

### S2. pool 崩壊守備（embedded fleet 予算比率のハードコード）

- **課題**: convoy の pool 生成 window（1/3）のうち embedded fleet が (budget/4).min(4000) を消費するため、label search 系に残る時間が 32×32 で不足し、pool が [fleet 計画のみ] または部分集合に崩壊しうる。比率 1/4 と cap 4s はコード定数。
- **根拠**: `convoy_solver.rs:127-134, 1119`。daily_response_us が 10s 使い切りなのに出力が fleet@2.5s 相当（s9012 B369 ≒ fleet iters60k 373 ≪ fleet@10s 406）という痕跡。詳細は `analysis/convoy-clocked-degradation.md` §3.4（tracing 未収集につき pool サイズ実測は未）。
- **期待利得**: s9012 で +30〜40 balls（fleet@10s 同等への回復）。s5023/s5020 でも +数〜+30。R1 規模 gate（32×32 で convoy を外す）を不要にする可能性。
- **検証条件**: ①P0: tracing `convoy day plan`（`convoy_solver.rs:1157`）を収集し pool/complete/chosen を実測 ②P1: `fleet:budget_ms=2500/5000:adapt=off` 単体ラダーで pool[0] 期待値を確定 ③P2: 比率変更版をスクリーニング 9 + 検証 27 で A/B。D 非劣化と day0 オーバーラン（現状 +0.9–1.0s）の監視必須。

### S3. beam 時間配分と depth 制限の適応化

- **課題**: pool 生成 1/3・beam 2/3・scenario 均等分割がハードコードで、32×32 では beam が完走せず残り 6.7s を捨てている（complete=false → chosen=0 → pool[0]）。lookahead 用 fleet（iters=20000:chains=1:adapt=true）も固定でパラメータ化されていない。
- **根拠**: `convoy_solver.rs:1119, 1128-1133, 135, 1146, 1153-1156`。daily_response_us が 10.0–10.1s で使い切る一方、depth=6 を ~3.3s/scenario で完走できる見込みがない。
- **期待利得**: 大盤面で beam 時間を pool[0] 生成に転用（S2 と同根・同時改修が自然）、または完走する深さへの適応で deviation 利得の復活。match3 の depth データ（d2/d3 で飽和）から、小盤面では depth 削減で時間余剰を作れる。
- **検証条件**: `convoy:budget_ms=10000:forecast=persist`（scenario 1 つで beam 時間 2 倍）を s9012 で 1 実行（P3）+ tracing。予測: 改善 +0〜+15。+25 以上なら beam 完走時の deviation 利得が大きい = S2 単独では不十分。depth ラダーの corpus 測定はバッチ 1 で 24×24 を 1 盤面以上入れることを推奨。

### S4. 並行 escort の表現制限

- **課題**: lead 補給は「最大 1 巡回に touch → 別の 1 巡回を escort」、追加補給は「1 巡回を全日 escort」固定。riders 割当は `(b+1+k) % np` の制約付き。2 補給同時 touch や補給 2 台の協調計画が表現できない。
- **根拠**: `convoy_solver.rs:857-899`（コメントと riders フィルタ）。補給 2 台盤面（c-s2）では convoy と fleet が B 差 ≤14 で混在し、1 補給のような差が表れていない。
- **期待利得**: c-s2 型（補給 2 台）での計画空間拡大。未確定ゾーン（32×32: −14/−7/+7、chains8 未測定）の解像。
- **検証条件**: c-s2 の 9 盤面 @10s で変更版 convoy vs fleet vs chains8（chains8 未測定の補完も兼ねる）。2 補給では盤面構成自体が変わるため 1 補給の結果と直接比較しない。

## 3. 条件付きパラメータ選択の仮枠（R1/R2 の拡張草案）

`summary/conditional-selection.md` の R1（clocked + マップ一辺 ≥32 → chains8）/ R2（`(patrols×spots)²/3 ≥ 6s` gate）を、パラメータ軸まで広げた仮枠。**確定枠は R1 のみ。他は検証 27 盤面での再確認前の草案**。

| 領域 | 選択仮枠 | 根拠 | 確定度 |
|---|---|---|---|
| clocked・32×32 | **chains8**（R1 確定枠）。chains12/16 はバッチ 1 次第で差し替え | 32×32 公式勝者 chains8 4/6、convoy 0/6 | 確定（規則レベル） |
| clocked・24×24 | convoy 既定（width24/depth8/forecast both）維持。chains8 との差はノイズ以下 | 18 セル中差 ≤3 が大半、s9005 −10 / s9008 +28 が逆向き | 未確定（chains ラダーで再評価） |
| clocked・16×16 | convoy 既定。depth 2–3 で同成果（時間余剰） | match3 depth データ、s9003/s5001 差 ≤3 | 未確定 |
| clockless・16×16 | convoy（iters 既定 60k） | honsen-like 17 勝 2 敗 8 分、match3 dV+1 | 比較的高 |
| clockless・24×24-a4 | convoy = fleet 同点 | s9007 (20,139,174) 両候補同点 | 未確定 |
| clockless・24×24-a5+/32×32 | convoy は timeout 失格 → **fleet iters**（60k vs 120k は dV +1 が 2 盤面あり、検証 27 で辞書順確認のうえ） | hard-convoy-iters120k timeout 6 件 | 比較的高 |
| 補給 2 台 | 現状データ不足。chains8 未測定 | c-s2 単一実行、B 差 ≤14 | 未確定（S4・c-s2 測定待ち） |

条件変数の事前観測可能性は R1/R2 と同一（patrols×spots と budget_ms はソルバが計算可能）。D 飽和は事前観測不能なため、領域分割には使わない。

## 4. バッチ結果の分析プロトコル（measurer2 の param-* 着信後、順次実行）

各 `param-*` バッチごとに以下を実施し team-lead へ報告:
1. **辞書順監視**: 既定設定 vs 変更設定を全セルで (V, dV, B) 比較。dV が 1 でも低下したセルを列挙（B 改善があっても採用候補から外す）。
2. **B 改善判定**: ±20 未満はノイズ扱い。±20 以上のセルだけ「改善/悪化」と判定し、seed 反転（clocked 2 実行）で方向が保存されるか確認。
3. **最悪ケース**: B 低下 >20 または dV 低下の盤面を列挙し、盤面条件（規模・日数・補給数）との相関を見る。条件付き採用（§3 の枠の更新）に回す。
4. **スクリーニング/検証の分離**: スクリーニング 9 盤面で選んだ設定は検証 27 盤面の結果が出るまで「草案」のまま。検証での劣化セル数が 0（辞書順）を確認して初めて採用推薦。
5. **wall-clock/timeout 監視**: rt > 予算×1.05 の日、harness deadline 超過、`day_budget_exceeded_days` を確認（convoy は既に day0 +0.9–1.0s のオーバーラン実績あり）。

## 5. 参照パス

- chains: `wsl-corpus/holdout-fleet-iters60k-1chain/runs.jsonl`（chains1）vs `holdout-fleet-iters60k-chains4/`（chains4）; `wsl-10s/b10s-seed42|seed7/runs.jsonl`（chains4 vs chains8 @10s）
- iters: `wsl-corpus/hard-fleet-iters60k-chains4/` vs `hard-fleet-iters120k-chains4/`
- clockless convoy timeout: `wsl-corpus/hard-convoy-iters120k/runs.jsonl`
- convoy depth/width（match3）: `wsl-10s/round1-solo.jsonl`、`convoy-shape.jsonl`、`convoy-seeds.jsonl`
- convoy forecast/width（corpus @10s）: `wsl-10s/d-forecast=busy|persist/`、`d-width=12|48/`、再測定 `d2-busy|d2-persist/`
- clocked 予算: `wsl-10s/b10s-seed42|seed7/`、`b2-20s-s9012/`
- 関連: `summary/conditional-selection.md`（R1/R2）、`analysis/convoy-clocked-degradation.md`（S2/S3 の機構分析）