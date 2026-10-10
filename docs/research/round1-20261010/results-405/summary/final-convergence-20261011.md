# 最終収束記録 — 比較計測（2026-10-11・本戦当日）

本指示により新規計測を全面停止し、確定済み成果のみで判断を固定する。
本ファイルが最終分類の一次記録。詳細ログは `measurer-progress.md`、背景は `conditional-selection.md` §7 と `adoption-handoff.md`。
**本番構成（release-0-5-1 確定分）への変更・提案は本記録に含めない。**

## 1. 最終分類（公式辞書順 V→D→B）

### 採用済み（本番 release-0-5-1）

- **なし（今回の計測からは）**。本番は既存構成を維持（§3 指示どおり、chains8 / fuel 軸 / bail は未採用のまま）。

### 有効だが本番未採用

| 候補 | 根拠（条件・V/D/B・証跡） | 未採用の理由 |
|---|---|---|
| **fleet chains8**（`e79d786`=origin/409・PR #415 未マージ） | clocked @10s seed42/7 × 18 盤面: V 同点・D は盤面別 ±1 のみ・B 合計 +25/+21（`wsl-10s/b10s-*/`）。full corpus D 下落 0・maxdaily 10.0-10.2s（`wsl-10s/b4-chains8-full/`）。**offline では s9008 dV−4 の D-loss が実在**（`wsl-corpus/param-chains8/`・決定論）だが fuel_need=50 併用で D+5/B+37 回復（`wsl-corpus/f3-offline-fuel/`） | PR #415 が本戦前に merge されていない。R1（clocked 32×32 → chains8）は規則レベル確定枠だが実装取込は別判断 |
| **chains8 @ budget 2.5-5s**（E3 予算カーブの派生） | chains8 は 2.5s で B 飽和: s9012 c8@2500=414 ≈ @20k=425・s5023/s5020 全予算フラット（`wsl-10s/e3-budget-ladder/`・静穏・jobs 2） | 単一実行・盤面 4 枚のみ。s9008 はノイズ域で飽和点不明。**axis-4 余裕は第 4 軸のみの改善であり辞書順上位 3 軸が全て同点のときしか効かない** |
| **convoy depth=2**（24×24） | depth ラダー: s9008 d2 (20,**180**,259) が dV 最大値維持・d8 (20,179,233)・d1 は dV−2（`wsl-10s/d-depth-ladder/`・Q3 静穏再実行は本日中止） | B は負荷混淆未解消（d8 はケース内最後尾実行）。match3 データ（d2-3 飽和）との整合はあるが corpus 2 盤面のみ |
| **bail=on**（convoy・`eb2f5de`=origin/432・PR #435 DRAFT） | 爆発盤面（pool==1 全日型）で axis-4・B 両改善: s5020 +57 / s5023 +39・maxday 33-50s→≤8.9s（`bail-435-verify` 側 `eval-bail-435/`・load 9-13）。fixture 9 盤面は 8 draw + B−1 | **s9012 で B −23 → 静穏再実行でも −30 再現 = 構造的劣後**（`eval-bail-s9012-quiet/`・load 4-8）。部分完走型盤面での B 損が解決されていない。cluster=on 未実装。本番 convoy を使う盤面クラスとの適合が未評価 |

### 劣後・棄却（確定）

| 対象 | 根拠・証跡 |
|---|---|
| convoy@10s hard 構成 | s9003 D−1・s9007 D−1（`wsl-10s/b10s-seed42/`、origin/406 sequential-budget） |
| 補給 2 台 kinds | D 同点でも B −8〜−12・1 補給比 D −42・kinds-auto 不選（`wsl-10s/c-s2/`、`c2-20s-s2/`） |
| fleet iters=120000 | maxdaily 52.24s・予算不適合（`wsl-corpus/hard-convoy-iters120k/`） |
| convoy clockless 大盤面 | 24×24-a5+/32×32 で harness timeout 425s（同上） |
| persist / width 変更の既定化 | default 最良かほぼ同等（`wsl-10s/d-forecast=*` `d-width=*` `d2-*`） |
| chains8 seed123 | 9/24 盤面 invalid/timeout・maxday 188-253s・探索時間が seed で爆発（`wsl-corpus/param-chains8-seed123/`） |
| convoy@20s の既定化（10s 構成に対する全面的切替） | 60s 期限累積超過リスク（maxdaily 23.71s 実測 `wsl-10s/b3-20s-seed42/`）＋対戦交通未検証 |

### 結論未確定（本戦終了時に持ち越し）

| 項目 | 現状と残る問い |
|---|---|
| **fuel 軸（fuel_need=50:fuel_mix=on）の採否** | offline: D 下落 0・mix 合計 +44/+39（`wsl-corpus/f3-offline-fuel/` + `f3-a-slim/`）。clocked screen: c4 mix +62 / c8 mix +27 だが **s9003 dV−1（seed42）・seed7 では逆転**（`wsl-10s/q1-s9003-seed-confirm/`）。**V は全セル同点・悪化は dV（daily_variety_sum）−1 のみ**。検証 27 盤面の確認は他セッションが進行中だったが本日時点で未回収。採用判断には検証 27 + s9003 クラスの静穏 seed 反転が必須のまま |
| **chains12**（offline では B ピーク 11863・D-loss なし、`wsl-corpus/param-chains12/`） | **clocked モード未測定のまま本日中止**（Q2 は証跡出力前に停止）。R1 の差し替え判断は保留 |
| **R1 の本番適用**（clocked 32×32 → chains8） | 規則は確定だが、本番 PR 未マージ・24×24 以下の convoy 維持領域の機会損失はノイズ域 |
| **対戦交通下の候補比較** | 本セッションは solo/観測交通のみ。match 2052 分析（別セッション）が offline B は実戦上限推定（+17% 過大）ことを確定 — 候補間 B 差が実戦でそのまま効く保証なし |
| **fuel 比仮説（比 ≤1.5 → 有効）** | s5023（比 1.38）が offline fuel-only −29 / clocked c4 mix +32 で分散し、比単変数では分類不能（`f3-offline-fuel/`・`f3-clocked-fuel/`） |

## 2. 指定事項の個別記録

### match 2052（内部 replay と実戦 B=206 一致・オフライン乖離）

比較計測セッション（arena-sized-e9e8af6 worktree で offline 再計算も実施）による確定調査。**完全な記録・証跡は `analysis/match2052-traffic-gap.md`**（coordinator log / runs.jsonl 行 / API problem / 再計算 CaseSpec / 20 試合台帳を evidence 同梱）:
- Arena match 2052（24×24/5台/9日）: coordinator log の日別 replay_balls 合計 206 = 実戦 B と完全一致。predicted_ends 全 9 日で翌日実観測一致 → シミュレータ再現誤差ゼロ
- **offline eval B=241**（負荷下・他実行 248）/ **pre-match trial B=244** / **実戦 B=206**: 差 35-42 balls（~17%）は他チーム 9 チームの道路滞在による busy/jam 渋滞。offline エンジンは他チーム滞在 0 扱い（`crates/engine/src/transitions.rs` phase_traffic_update）
- 日別は day2/3 の落ち込み最大・補給経路渋滞 → 燃料 0 停滞が主因。2052 は busyThreshold=1/jammedThreshold=2 の高感度盤
- **限界: 固定 wire 再生ができていない**（2052 は recording OFF で提出 wire が失われた）。sized 20 試合集計: trial−actual は 16×16 +2.0% / 24×24 +2.6% / 32×32 −9.1%
- 含意: **offline B は実戦 B の上限推定にしかならない**。本セッションの全 B 比較も同条件内相対比較としてのみ解釈可能

### fuel_mix の s9003「V 悪化」扱いについての正確な記録

- 実測は **V（variety）同点・dV（daily_variety_sum）−1**（seed42: anchor 106 → fuel/mix 105）。公式辞書順では dV が B に先行するため**辞書順敗北セル**として扱った
- seed7 では anchor 104 → fuel 105 で**逆に dV+1**（`wsl-10s/q1-s9003-seed-confirm/`）。s9003 は dV 104-106 境界盤面で、fuel の dV 変動は ±1・seed 依存 = chains8 採用時に既に監視条件としている「盤面別 D ±1」クラスに含まれる
- offline モードでは fuel による dV 変動は全 36 盤面で 0（clocked のみで出る）

### bail=on の s9012 B 悪化

- load 9-13 実行: parent 387 / bail 364（−23）→ **静穏再実行（load 4-8）: parent 382 / bail 352（−30）で再現 = 構造的**（`eval-bail-435/`・`eval-bail-s9012-quiet/`・binary sha256 `85119c9f…`）
- 機構仮説: s9012 は parent のラベル探索が部分的に完走できる盤面で、bail が beam 情報を捨てて embedded fleet にフォールバックする分を B で失う。完全爆発盤面（s5020/s5023）は逆に +57/+39
- 判定: **bail=on は爆発盤面型で有効・部分完走型で劣後の条件付き候補**。爆発程度の事前観測 gate は未実装

### chains8 / chains12 の未解決事項

- chains8: (a) clocked の盤面別 dV ±1（s9008 seed42 dV−1・s9003 fuel 併用時）(b) 対戦交通未計測 (c) PR #415 未マージ (d) offline s9008 dV−4 は fuel 併用で回復するが、fuel なし chains8 単体の offline D リスクは残る
- chains12: offline で chains8 上位互換の証拠（B ピーク・D-loss なし）だが **clocked 未測定のまま中止**。chains12 の実時間安全性（chains12 は 8 コア超の chain を時間配分でどう捌くか未観測）も未確認
- 双方とも本戦当日の本番投入判断には使えない状態で停止

### wall-clock 負荷汚染と無効サンプル

- 複数セッション並走が常態化し、load 1.4-27 で変動。**wall-clock モードの B は実行間 ±10-20 変動・低負荷で順位反転した実例あり**（s9012: b2 高負荷 388 < fleet 396 < chains8 415 → b3 低負荷 convoy 412 > fleet 407 > chains8 403）
- **無効サンプル**: `f3-offline-fuel/` の 19/108 runs は nice 19 圧迫で daySeconds 超過 invalid（決定論スコア自体は記録・`f3-a-slim/` の再現一致で妥当性確認済み・scored=False のまま）。`param-chains8-seed123/` は seed123 自体の時間爆発で無効（負荷ではなく候補側の問題）
- 本日確定した判定はすべて「同一実行内アンカー比較」または「決定論 2 実行一致」に限定した。異バッチ間の B 絶対比較は行っていない

### サチり判定（E3 予算カーブ）の観測改善幅と停止リスク

- 観測: chains4 は s9012 で 2500→10k に **+66**（339→405・階差 +36/+30 はノイズ超の単調増）→ ~10s 飽和。chains8 は全盤面で 2500s と 20k の差がノイズ内（s9012 +11 / s5023 +5 / s5020 +11）
- **未検証の停止リスク**: (a) 単一実行で盤面 4 枚のみ (b) ±20 ノイズで s9008 の飽和点は判定不能 (c) `adapt=off` 固定で adapt=on の早期打ち切りとの相互作用未測定 (d) 飽和後の axis-4 余裕は辞書順の第 4 軸でのみ効く（上位 3 軸同点時のみ）(e) 本判定から本番予算変更を提案しない（§3 指示）

## 3. 本番構成への影響

- **変更なし・提案なし**。release-0-5-1 の確定構成を維持。chains8（PR #415 未マージ）・fuel 軸（PR #433 未マージ）・bail（PR #435 DRAFT）はいずれも本番に入っておらず、本記録は採用判断材料の固定のみを目的とする
- 本番構成の妥当性に関わる重大な既知不具合として本計測で新たに見つかったもの: **なし**（illegal / fuel_short / rejected / day_budget_exceeded は全確定バッチで 0・池上 spot panic の再発なし）

## 4. 証跡インデックス

- 進捗ログ（全バッチの実行条件・binary sha256・load・判定）: `summary/measurer-progress.md`
- 条件付き選択の追補: `summary/conditional-selection.md` §7
- 採用判定の背景: `summary/adoption-handoff.md`（2026-10-10 確定版）
- バッチ計画と静穏ゲート: `summary/next-batch-plan.md`、`tools/run_queue_20261011.sh`（第 1 ラウンド完走）・`tools/run_queue_round2_20261011.sh`（Q1 完走後 Q3 起点で中止）
- 生データ: `wsl-corpus/{f3-offline-fuel,f3-a-slim,param-chains8,param-chains12,param-chains16,param-chains8-seed{7,123}}/`、`wsl-10s/{f3-clocked-fuel,f3-smoke,q1-s9003-seed-confirm,e3-budget-ladder,d-depth-ladder,b3-20s-seed42,c2-20s-s2,b4-chains8-full,p1-embed-rung,p3-persist-s9012}/`、`analysis/bail-435-wsl.md`
- bail 関連の生データは `bail-435-verify` worktree 側 `eval-bail-435/`・`eval-bail-s9012-quiet/`（binary sha256 `85119c9f…`・source eb2f5de）
- すべて `eval-convoy-415` ブランチ（origin push 済み・最終 f7c8177 系）に記録

## 5. 中止した計測（未完扱い・成果なし）

- Q2（chains12 clocked・32×32 12 盤面 + s9008）: 静穏ゲート通過前に **証跡未書き出しのまま停止**。chains12 clocked は未解決のまま持ち越し
- Q3（depth 静穏再実行）: 起動直後に停止・証跡なし。depth B の負荷混淆は未解消のまま
- 検証 27 盤面 fuel 確認: 他セッション（`measure-63a7e5e`）の所有タスクとして進行中だったが本セッションでは未回収