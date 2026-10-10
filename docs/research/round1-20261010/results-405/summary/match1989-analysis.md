# Arena match 1989 分析 — B 最大化の余地評価（第 1 段階）

- 作成: 2026-10-10 / match 1989 分析担当（read-only 分析）
- データソース:
  - `.worktrees/arena-wsl-13a0106/target/arena/runs.jsonl` の `arena_match_id=="1989"`（1 レコード、agent 7・7 day の開局 state + movement 摘要）
  - `.worktrees/arena-wsl-13a0106/target/arena-run-13a0106-wsl-loop.log` の match 1989 区段（kinds 試算 + in-match fleet day plan）
  - 得点表（ユーザ伝達）: 自班 B=448 / D=147（満点）vs 上位 B=483 → 差 35
- 構成: kinds=`S,P×6`（agent0=Supply, agent1..6=Patrol）、fuel 上限 126、day_steps=[82,67,100,91,78,80,93]（計 591 step）
- **盤面（replay JSON）は未取得。地域別回収量・ball 分布は本分析では確定できない**（→ §6）

---

## 1. agent × day movement テーブル（開局時 fuel 付き）

### 1.1 浪費が疑われる agent-day（waits ≥ 15 または未計上 gap ≥ 50）

| day | agent | kind | 開局 fuel | pos | moves | waits | runs | 計上計 | gap | dirs |
|----|------|------|----------|-----|-------|-------|------|--------|-----|------|
| 0 | a0(S) | S | 126 | 641 | 54 | 22 | 4 | 80 | 2 | [0,9,25,8,6,6] |
| 2 | a0(S) | S | 126 | 199 | 40 | 18 | 3 | 61 | 39 | [2,5,7,23,3,0] |
| 2 | a1(P) | P | **6** | 849 | 31 | 35 | 1 | 67 | 33 | [3,5,9,10,3,1] |
| 2 | a2(P) | P | 117 | 674 | 53 | 0 | 0 | 53 | **50** | [5,23,10,6,4,5] |
| 2 | a3(P) | P | 83 | 938 | 50 | 0 | 0 | 50 | **50** | [10,22,12,4,2,0] |
| 2 | a4(P) | P | 126 | 199 | 50 | 0 | 0 | 50 | **50** | [5,4,12,22,6,1] |
| 2 | a5(P) | P | 87 | 1022 | 46 | 1 | 1 | 48 | **52** | [7,12,9,2,1,15] |
| 3 | a0(S) | S | 126 | 826 | 42 | 25 | 2 | 69 | 22 | [5,12,1,7,16,1] |
| 3 | a2(P) | P | 45 | 113 | 29 | 41 | 1 | 71 | 20 | [2,0,1,3,10,13] |
| 3 | a3(P) | P | 21 | 125 | 38 | 17 | 1 | 56 | 35 | [1,2,1,11,16,7] |
| 4 | a1(P) | P | 31 | 199 | 36 | 15 | 2 | 53 | 25 | [7,6,2,7,11,3] |
| 4 | a2(P) | P | **2** | 448 | 9 | **65** | 2 | 76 | 2 | [0,4,5,0,0,0] |
| 4 | a4(P) | P | 30 | 100 | 19 | **46** | 1 | 66 | 12 | [0,5,9,4,1,0] |
| 5 | a3(P) | P | 34 | 1022 | 28 | 26 | 1 | 55 | 25 | [9,2,2,3,2,10] |
| 5 | a4(P) | P | **1** | 113 | 33 | 20 | 2 | 55 | 25 | [2,1,4,4,11,11] |
| 6 | a0(S) | S | 126 | 819 | 11 | **76** | 2 | 89 | 4 | [0,0,0,0,4,7] |

### 1.2 日次合計

| day | steps | moves | waits | runs | 未計上 gap |
|-----|-------|-------|-------|------|-----------|
| 0 | 82 | 433 | 37 | 7 | 97 |
| 1 | 67 | 255 | 23 | 8 | 183 |
| 2 | 100 | 314 | 61 | 6 | 319 |
| 3 | 91 | 336 | 85 | 6 | 210 |
| 4 | 78 | 224 | **133** | 9 | 180 |
| 5 | 80 | 268 | 53 | 6 | 233 |
| 6 | 93 | 317 | 94 | 7 | 233 |
| **計** | 4137 (agent-step) | **2147** | **486** | 49 | **1455** |

- 計上率 = (moves+waits+runs) / steps = **64.8%**。残り 1455 agent-step は movement 摘要のどこにも計上されない。

---

## 2. 燃料待ち・停滞の分解

### 2.1 燃料飢餓 waits（開局 fuel ≤ 45 の Patrol）

| day | agent | 開局 fuel | waits |
|-----|-------|----------|-------|
| 1 | a1 | 35 | 10 |
| 2 | a1 | **6** | 35 |
| 2 | a5 | 43 | 7 |
| 3 | a2 | 45 | 41 |
| 3 | a3 | 21 | 17 |
| 4 | a1 | 31 | 15 |
| 4 | a2 | **2** | 65 |
| 4 | a4 | 30 | 46 |
| 4 | a5 | 44 | 4 |
| 5 | a3 | 34 | 26 |
| 5 | a4 | **1** | 20 |
| **計** | | | **286** |

（fuel ≤ 31 に狭めると 198。全 Patrol waits 486 のうち 6 割近くが飢餓日に集中）

- day 4 a2（fuel 2）: moves 9 / waits 65 / dirs=[0,4,5,0,0,0] — ほぼ全日停止。day 5 開局で pos 327 / fuel 126（完全補給済み）→ 当日は補給到着を待っていた。
- day 4 a4（fuel 30）: moves 19 / waits 46。ただし **day 5 開局 fuel 1** — 当日は補給されないまま day 5 も飢餓（fuel 1 で moves 33 / waits 20、日中に補給されて回復）。
- day 2 a1（fuel 6）: moves 31 と waits 35 が共存 → 日中に補給到着し前半停滞・後半移動の形状。

### 2.2 意図的に計画された枯渇（ソルバは知っていた）

loop log の in-match day plan の `predicted_ends` が**翌日の実開局 state と完全一致**している（例: day3 plan 予測 end=`(448,2)` → day4 開局 a2 pos 448 fuel 2。day4 plan 予測 end=`(113,1)` → day5 開局 a4 pos 113 fuel 1）。

つまり燃料枯渇は偶発ではなく、**solver が end-fuel を 1〜6 まで削る計画を意図的に選んでいる**。低 fuel 開局日は翌日の model_balls も概ね落ちておらず、solver は「枯渇リスク < 回収増」と判断しているが、day 4 のように補給が間に合わず waits が爆発する日が出ている（day4 model_balls 44 = 7 日中最低）。

### 2.3 Supply（a0）の idle

| day | moves | waits | 備考 |
|-----|-------|-------|------|
| 0 | 54 | 22 | |
| 1 | 36 | 3 | |
| 2 | 40 | 18 | |
| 3 | 42 | 25 | |
| 4 | 44 | 2 | |
| 5 | 41 | 0 | |
| 6 | 11 | **76** | 後半アイドル。開局時 a0/a3/a5/a6 が pos 819 に密集、a3/a5/a6 は fuel 126 済み |

- Supply waits 計 **146**（全 486 の 3 割）。day 6 の 76 は補給任務完了後の純アイドル（dirs=[0,0,0,0,4,7]、moves 11）。
- **並行 escort 判定: 実装されていない（単一補給局の逐次対応）**。day 4 の a2(448) と a4(100) が同時飢餓で合計 111 waits → 単一 S が時分割で対応している構図。S が 1 台だと物理的に同時到達できず、kinds 試算でも S×2 は試していない（log 上は `S,P×6` と `P×7` の 2 押のみ）。→ replay JSON 取得後、`S,S,P×5` の D 影響（daily_variety は variety 数に依存するので S 追加で D が落ちる可能性）を確認する価値がある。

---

## 3. 未計上 gap（1455 agent-step）の解釈メモ

`day_steps − (moves+waits+runs)` が日をまたいでほぼ均一（day1 は全 agent 23〜29、day6 は全 agent 4〜6）な一方、day2 は 50 前後で揃う。均一性から**記録Window/集計方式のアーティファクト（テレメトリの部分記録または計画終了後の無 action 時間）**の可能性が高い。燃料で説明できない例（day2 a2: fuel 117 なのに gap 50）があるため「燃料枯渇による記録停止」ではなし。

- もし gap が実 idle なら、総 idle = waits 486 + gap 1455 = **1941 agent-step（全 step の 47%）** となり、B 天井は一気に上がる。
- もし記録アーティファクトなら、B 残余は waits 486 ベースで考えるべき。
- **判定は replay JSON 取得後のみ可能**（→ §6 の確認項目 1）。

---

## 4. kinds 試算 484 vs 実績 448（−36）の分解

loop log の数値:

| 区分 | plans (chains) | day 別 balls | 合計 |
|------|----------------|--------------|------|
| kinds 試算 `S,P×6`（登録前・09:15） | chains=1 | 77,66,77,76,42,68,78 | **484** |
| kinds 試算 `P×7`（同） | chains=1 | 24,26,33,31,25,25,28 | 192（報告値 190, D134） |
| in-match 実行 day plan（09:17–09:23） | **chains=4** | 85,57,66,69,44,56,71 | **448** |
| 実績 B | — | — | **448** |

**結論: 実行側のロスは 0。** 根拠は 2 点:

1. in-match 各日の `replay_balls` 合計 = 448 = 実績 B（7 日の和がぴったり一致）。
2. 各日 plan の `predicted_ends`（pos, fuel）が翌日の実開局 state と 7 agent 分すべて一致（day0→day1 で確認: (826,126),(29,71),(113,35),(826,126),(938,112),(826,126),(1022,104)）。

つまり −36 は**実行が計画を外したのではなく、登録前 kinds 試算（chains=1・day0 開局盤面のみで評価）が楽観だった**ことを示す。chains=1 の試算プランが chains=4 の実行プランより本当に +36 良かったのか、試算の前提（ball 再生仮定）が楽観だったのかは**試算プランを実 replay していないと確定しない**（→ §6 確認項目 3）。

補助証拠: D 側は `daily_variety 147 = variety 21 × 7` で満点。`P×7` は D134/B190 しか出せず、`S,P×6` 選択は正しい。

---

## 5. B 残余量の推定（仮説明記）

上位差 = 483 − 448 = **35**。

| レバー | 上限（仮説 A: 待ち 1 step = ball 1 個回収） | 現実性 |
|--------|--------------------------------------------|--------|
| 燃料飢餓 waits（fuel≤45 の 286 step）を移動へ転換 | **+286** | 仮説 A の上限。ball 存在と S の補給キャパが前提。実態は未確定 |
| うち day4 a2+a4 の 111 step（同時飢餓） | +111 | S×1 では時分割しかできず、S×2 / 計画変更が前提 |
| Supply idle 146 step | （S 自体は回収しない想定。S を回収に使うと fuel 供給が崩れる） | 小 |
| 未計上 gap 1455 step が実 idle だった場合 | +1455 | §3 のとおり記録アーティファクトの可能性大。replay 必須 |
| day 別 model_balls の最低日（day4=44）を day5 水準(56)に | +12 | chains=4 の同一枠組み内の改善目安 |

**本分析から言える最小推定（仮説付き）**:

- 飢餓 waits 286 のうち **1〜2 割（≈ 35〜60）を確実に回収できれば上位差 35 は埋まる**。286 全転換は不要。
- ただし「待ち → ball」の 1:1 は**明示的な仮説**であり、次の理由で実際の転換率は下がる: (i) 待ちセル周辺に ball が残っている保証がない、(ii) S の補給能力（1 台・fuel 126）がボトルネック、(iii) 枯渇計画は solver が B 最適として選んだ結果であり、fuel 余裕を持たせると別の日の B が落ちる可能性。
- **盤面なしに確定できるのは「実行側ロス 0・試算側楽観 −36」「飢餓待ち 286 step」「S 単独の逐次補給構造」まで。** 地域別回収・ball 分布・dirs バイアスの影響（全 dirs 合計 [318,357,417,335,365,355] — dir2 過多・dir0 少なめだが盤面構造次第で正常範囲）は replay 待ち。

---

## 6. replay JSON 取得後の確認項目とオフライン実験計画

### 6.1 確認項目（分析の確定条件）

1. **未計上 gap 1455 の分類**: replay の全 step action log で「計画終了後 idle」「fuel 0 強制停止」「記録除外」を区別。実 idle なら B 天井は大幅上振れ。
2. **地域別回収量と ball 分布**: 飢餓 waits が発生したセル（448, 100, 113, 125, 849, 1022 周辺）の ball 存在と他 agent の重複訪問を確認。dirs バイアスが ball 分布と整合しているか。
3. **chains=1 試算プランの実 replay**: 試算 484 が本当に実現できるか（§4 の −36 の帰属確定）。
4. **S×2 kinds（`S,S,P×5`）の D 影響**: daily_variety への種類数影響と B 増のトレードオフ。
5. **枯渇計画の検証**: end-fuel 1〜6 の計画で、補給失敗時の待ちコストを plan 評価に織り込むべきか（リスク調整）。

### 6.2 オフライン実験（import-case → solo replay）

```bash
# 1) replay JSON → CaseSpec（fail closed。wire MatchSetting が取れない shape は出力しない）
cargo run -q --bin procon -- arena import-case \
  --input arena-replay-1989.json --output target/arena/cases/match-1989.json
# （just arena-import <in> <out> も同内容）

# 2) solo replay・4 候補同一実行内比較（jobs 1・同一 corpus で絶対値比較）
cargo run -q -p bench --bin procon-eval -- \
  --cases all \
  --case-files target/arena/cases/match-1989.json \
  --candidates 'fleet:budget=official:budget_ms=10000:adapt=off:seed=42,fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=8,convoy:budget=official:budget_ms=20000:seed=42,convoy:budget=official:budget_ms=59000:seed=42' \
  --baseline fleet:budget=official:budget_ms=10000:adapt=off:seed=42 \
  --output-dir target/eval/match-1989
```

（候補文字列は run-ledger / results-405 の既存表記に準拠: fleet@10s / chains8@10s / convoy@20s / convoy@59s）

**判定基準**:

- 主: 盤面 B 総量。match 1989 の chains=4 実績 448 を baseline とし、**chains8 / convoy@59s が B +20 以上**なら採用判断へ（単盤面 ±20 の変動があるため、複数 seed or 他盤面との併読が必須 — run-ledger P1 の教訓）。
- 副指標（この分析の裏取り）: waits 総数（486 → 目標 <300）、fuel≤45 開局の agent-day 数（11 → 目標 <6）、day 別 B の最低値（44 → 目標 >55）。solo replay の day plan ログから同一指標を抽出して比較する。
- D の guard: 全候補で D=147 維持（満点からの後退は不採用）。
- 追加試験（§6.1-3/4 に対応）: kinds `S,S,P×5` と chains=1 相当の候補を同一 corpus に追加し、D 後退量と B 増を同時確認。

---

## 7. まとめ（team-lead 向け 3 点の詳細版）

1. **浪費 step**: 全 waits 486（飢餓起因 286 / Supply idle 146 / その他 54）+ 未計上 gap 1455（分類待ち）。最悪は day4（a2: 65, a4: 46, a1: 15）と day6 Supply 76。
2. **B 残余（確認可能分）**: 飢餓 waits 286 step の 1〜2 割転換で上位差 35 は埋まる見込み（1 step=1 ball 仮説、ball 存在・S キャパ前提）。実行側ロスは 0（in-match replay 予測 448 = 実績 448）、−36 は登録前試算の楽観。
3. **replay 待ち項目**: gap 1455 の分類 / 地域別回収・ball 分布 / chains=1 試算プランの実 replay / `S,S,P×5` の D 影響 / 枯渇計画のリスク評価。取得後は §6.2 のコマンドで 4 候補 solo replay → waits・飢餓日数・day 最低 B を判定基準に使用。