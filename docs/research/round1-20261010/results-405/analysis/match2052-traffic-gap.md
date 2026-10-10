# match 2052 実戦 B 乖離の調査 — 評価環境差の確定（2026-10-11）

比較計測セッションによる調査記録。結論: **乖離はシミュレータ再現誤差でも計画生成時の予測誤差でもなく、オフライン評価環境に他チームの道路渋滞が存在しないことによる環境差**。

## 1. 試合の同定

| 項目 | 値 |
|---|---|
| Arena lobby match id | **2050**（coordinator log・runs.jsonl） |
| 採点 table id | **2052**（lobby ↔ table は +0..2 ずれ得る。本件は +2） |
| 盤面 | 24×24 / 5 台 / 9 日 / official terrain（finals24 型） |
| daySteps | `[48,54,66,72,72,72,72,72,72]` / daySeconds 60×9 |
| fuelLimits / 閾値 | 60 / **busyThreshold=1, jammedThreshold=2**（実 fixture 3/8 より極端に敏感） |
| 出発時刻 (departure) | `1791652500` = 2026-10-10 17:15 UTC |
| 初期配置 | `[419,130,354,137,336]` |
| 実戦結果 | 進捗 B=206 / D=144 / rank 3 / rating 1595→1583 / 回答累積 493.3s（≈9日×55s） |
| 上位 | 1位 243・2位 235 — D 上位 3 チーム同点の B 勝負盤面 |

同定根拠: departure 一致 + daySteps 完全一致 + 回答時間シグネチャ。「進捗」ラベルは他チームによる流用が観測されているため rating 連鎖単独では同定しない。

## 2. 切り分け結果

| 世界 | B | 証跡 |
|---|---|---|
| 実戦（サーバー採点） | **206** | API `results`（api-match-2052.json） |
| 実機 planner の日別内部 replay 合計 | **206** | coordinator log の `replay_balls` 日別 [24,27,18,22,22,23,19,24,27] |
| planner の predicted_ends vs 翌日実観測 | **全 9 日完全一致** | log 対 runs.jsonl（arena-run-2050.json） |
| pre-match kinds trial（オフライン的 9 日先読み） | 244 | log `agent kinds trial ... balls=244` |
| offline 新規探索 players=1 | **241** | eval-recalc-players1-2.runs.jsonl（V16 / dailyV 144 / 負荷下計測・別セッション報告 248 と整合） |
| offline players=2 | **未完**（worker 中断で timeout・再実行は本日中止） | — |
| offline players=2（別セッションの参照値） | 約 264 | players が交通量平均の分母に効くため players 増で B 上がる方向は機構と整合 |

- **シミュレータ再現誤差: ゼロ**。実機 replay は観測 road state（日開始に公開される）を使い、日別 B も翌日状態も_server 実績と厳密一致。
- **計画生成時の予測誤差: ゼロ**。planner は渋滞を見た上で 206 を正確に予測した。
- 乖離 35〜42 balls（**約 17%**）の正体: オフライン世界には他チーム 9 チームの道路滞在が存在しない。offline エンジンは他チーム滞在を 0 とみなし、wire `players` は交通量平均の分母にしか効かない（`crates/engine/src/transitions.rs` `phase_traffic_update` の注釈どおり）。spot 在庫はチーム毎（guideline「在庫」・`tools/r1sim.py` 実装メモ）のため、他チームの影響は道路状態のみ。

## 3. 日別分解（trial 244 → 実戦 206 の −38 の内訳）

| day | trial（先読み） | 実戦（replay） | Δ |
|---|---|---|---|
| 0 | 18 | 24 | +6 |
| 1 | 24 | 27 | +3 |
| 2 | 28 | **18** | **−10** |
| 3 | 33 | **22** | **−11** |
| 4 | 27 | 22 | −5 |
| 5 | 28 | 23 | −5 |
| 6 | 25 | 19 | −6 |
| 7 | 29 | 24 | −5 |
| 8 | 32 | 27 | −5 |

- day2 以降の落ち込みが主体。runs.jsonl 上、**agent5 が day2・agent3 が day7 に燃料 0 のまま全日停止**（moves=0/waits=66,72）→ 補給経路の渋滞遅延 → 燃料追随破綻が B 差の主因と整合。
- day0/1 は実戦が trial を上回る（trial 先読みの近似誤差方向）。

## 4. 他盤面への展開（trial − actual、20 試合）

`pred-actual-20matches.csv`（sized 全 20 試合・lobby↔table 対応込み）:

- **16×16: +2.0%（+13/651）・ 24×24: +2.6%（+50/1930）・ 32×32: −9.1%（−148/1624）**
- 24×24 個別は −17〜+19 の中で **2052 の +38 が最大外れ値**（最も交通打撃が大きい盤面）
- 32×32 の負値は trial 先読みの近似誤差（大盤面では交通より先読み精度が支配）
- replay と server の一致は 20 試合中 17 試合が厳密一致・残り 3 試合 ±1〜3（2047: pred94/actual95、2056→2057: pred91/actual88、2062→2063: pred83/actual86）

## 5. 含意と限界

1. **offline B は実戦 B の上限推定にしかならない**（solo 交通世界）。本セッションの全 B 比較は同条件内の相対比較としてのみ解釈可能。
2. **限界: 固定 wire 再生ができていない** — 2052 は coordinator recording OFF（`--record-dir` 未設定）で提出 wire・交通 HTTP ログが失われた。以後の Arena 投入は `--record-dir` 必須。record があれば「提出行動固定再生 B = サーバー B」の検収ゲートを評価ハーネスに組み込める（engine の `phase_traffic_update` には観測総量 pre-scale のフック注記が既にある）。
3. 構造改善の方向（提案止まり・本日は実施しない）: 渋滞 1 日遅れに耐える燃料マージン/補給迂回の冗長化、公開済み滞在実績からの翌日 jam 予測の移動コスト織り込み。

## 6. 実験条件（再現に必要な全パラメタ）

| 項目 | 値 |
|---|---|
| 実戦候補 | `sized:budget=official:budget_ms=59000:convoy_ms=20000:seed=42:fuel_need=0` |
| source SHA | `e9e8af6f78b08c609aa2db1d4c9b82c3b35397bc`（release-0-5-1 系・PR #440 merge 後） |
| binary | `procon` @ worktree `.worktrees/arena-sized-e9e8af6`（記録時点・coordinator log manifest 参照） |
| kinds | `S,P,P,P,P`（`--kinds-auto` 選択・1 補給） |
| 探索時間 | 59s/day wall-clock（answer_ms 53.7-54.7s・margin ~5.2s・全日受理 revision 31→278） |
| offline 再計算 | `procon-eval --cases small-8x8 --case-files case-2052-p1.json --candidates <同候補> --kinds S,P,P,P,P --jobs 1`（resp 531s = 59s×9・budget 成立） |
| case 生成元 | API `GET /api/matches/2052` の problem → CaseSpec（players のみ 1/2 に変更・他は wire 値のまま） |

## 7. 証跡パス（本リポジトリ内・永続化済み）

- `analysis/match2052-evidence/coordinator-log-match2050.log` — 実戦 coordinator log 全文（fleet day plan / replay_balls / predicted_ends / 受理 revision）
- `analysis/match2052-evidence/arena-run-2050.json` — runs.jsonl の当該行（日開始 pos/fuel/moves/waits）
- `analysis/match2052-evidence/api-match-2052.json` — 採点 table 2052 の公式 API 記録（problem=盤面全件 + results 全 10 チーム）
- `analysis/match2052-evidence/case-2052-p1.json` / `case-2052-p2.json` — 再計算用 CaseSpec
- `analysis/match2052-evidence/eval-recalc-players1-2.runs.jsonl` — offline 再計算の生レコード（p1 B=241・p2 timeout）
- `analysis/match2052-evidence/pred-actual-20matches.csv` — 20 試合の pred/actual 台帳
- 参照（未 commit・別 worktree の生データ）: `.worktrees/arena-sized-e9e8af6/target/eval-2052/`、`.worktrees/convoy-live-3f72a37/target/arena/api/`（74 試合分の API 記録）
