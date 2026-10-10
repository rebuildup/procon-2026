# Issue #408 — fleet 翌日道路予測（traffic param）A/B 証跡

親Issue: [#408](https://github.com/rebuildup/procon-2026/issues/408) / branch `408` (base `403`@`470a2ff`) / worker-408 execution_generation 2 (2026-10-10)。

## 0. 位置付け

fleet のフェーズ2（各チェーンの計画をリプレイし、翌日の計画をその終状態から立てて比較する）において、翌日の道路状態を予測する param `traffic` を追加した。既定（param なし）は現行 fleet と同出力。

- 実装: `crates/bench/src/adapters/fleet_solver.rs`（param 解析 + `overlay_roads` + フェーズ2 World 選択のみ。スコア重み・SA・変異・レグ指標は無変更）
- テスト: `crates/bench/tests/fleet_candidate.rs`（param 解析 / 既定同値 / persist 冪等性+合法性）
- provenance: すべて `engine_replay`（`round1_rollout` による盤面再生。実戦の他チーム交通は `traffic=observed` の記録値で再現、自チーム_share は近似 — README §0 の格付けに従う）

## 1. 機構

フェーズ2の翌日 World は `replay()` の返す `check.state` から作られる。リプレイは `MatchSim::default()`（`team_count=1`, 閾値 2/5）で1日分を進めるため、日境界の `phase_traffic_update` が **自チームの stay-step のみ**（他チーム分は未知のまま 0）で全道路を再分類する。実装前に確認した実態:

- 観測で Busy/Jammed だった道路も、リプレイ後は「自チームが立っていなければ Smooth」に戻る（当日 World の観測 overlay が翌日 World に継承されない）。
- 自チームが滞在した道路は stay-step が team_count=1 で割られずそのまま分類されるため、実比 ~10倍に過大評価され Jammed 寄りになる。閾値もsim既定 2/5 で、本盤面の 1/2 ではない。

`traffic` の3値:

| 値 | フェーズ2翌日 World の道路 | 意味 |
|---|---|---|
| `off` (既定) | リプレイ後のまま（上記の歪みあり=現行動作） | 互換 |
| `persist` | `ctx.state`（当日観測）の道路状態を overlay | 翌日=当日観測の持ち越し。公式平均 (全チーム/チーム数) は未知のため、最後の観測が最良推定 |
| `own` | `off` と同一コードパス（自チーム stay 由来の分類を残す） | リプレイで計算できる唯一の自力シグナル。team_count 補正は **solver から players が見えないため不実施**（コメントで文書化のみ。親Issueは「チーム数で除算」を求めるが MatchState/MatchSetting は players を運ばない） |

SA内の tomorrow 推定（`evaluate()` の greedy、1試行あたり数千回）は **全モードで当日の道路コストのまま**（変更なし）。`persist` の予測「翌日=当日観測」と今日の World のコストは一致するため SA内は既に整合し、`own` 用の行列作り直しは反復あたりのコストが割に合わないため見送った（コメント文書化済み）。予測の主戦場はフェーズ2の計画選択。

## 2. 回帰（既定同値）

- `procon-eval --cases small-8x8 --candidates 'fleet:budget=official:iters=20000,wait'` を変更前 (`470a2ff`) と変更後 (`84a906c`) で実行: V2/D2/B7, daily [2], ball配分 [4,0,3], illegal/fuel_short/rejected 0 — **決定論フィールドは完全一致**（response_time_ms のみ実行時揺れ 25→19ms）。
- 701 fixture: `fleet:budget=official:iters=300:seed=42` ≡ `…:traffic=off`（seed 42/3, chains=2, decide バイト一致 — `fleet_traffic_default_is_identity`）。
- `cargo test -p bench --test fleet_candidate` 10/10 pass, clippy (`-D warnings`) / fmt clean。

## 3. Round-1 第3試合 A/B（traffic=observed, kinds S,P,P,P）

### 3.1 LIVE 設定（budget_ms=10000:adapt=off — 本番相当, 9 runs）

`docs/research/round1-20261010/results/issue408.jsonl` にハーネス summary 原文。

| mode | seed | V | D | B | daily_D |
|---|---|---|---|---|---|
| off | 1 | 10 | 43 | 51 | [8,10,8,8,9] |
| persist | 1 | 10 | **44** | 51 | [8,10,8,**9**,9] |
| own | 1 | 10 | **44** | 51 | [8,10,8,**9**,9] |
| off | 3 | 10 | 43 | 51 | [8,10,8,8,9] |
| persist | 3 | 10 | 43 | 51 | [8,10,8,8,9] |
| own | 3 | 10 | 43 | 51 | [8,10,8,8,9] |
| off | 42 | 10 | 43 | 51 | [8,10,8,8,9] |
| persist | 42 | 10 | 43 | 51 | [8,10,8,8,9] |
| own | 42 | 10 | **44** | 51 | [8,10,8,**9**,9] |

集計: V 10/10/10（不変）、D 43/43.3/43.7、B 51/51/51。

**重要な解釈上の注意**: `own` は実装上 `off` とバイト同一のコードパスなのに seed 1/42 で off と異なる結果 (D44) を出した。`budget_ms` モードは壁時計で打ち切るため実行ごとに探索が非決定的で、**±1 D の実行時揺れが最初から存在する**（ origin/403 の budget.jsonl `10000ms-0..2` が同じ candidate 3連続で同値だったのは、マシン空き状況が揃ったときの偶然。本計測は #403系の他タスクと並行で負荷が変動した）。つまりこの 9-run 表のモード間差 (≤+0.7 D) は揺れ幅と同程度で、**シグナルとは言えない**。

### 3.2 決定論モード（iters=20000:chains=2 — 壁時計を排した同一計算量比較, 9 runs）

| mode | seed | V | D | B | daily_D | off との差 |
|---|---|---|---|---|---|---|
| off | 1 | 10 | 40 | 51 | [8,8,7,8,9] | — |
| persist | 1 | 10 | 40 | **53** (+2) | [8,8,6,**9**,9] | B+2 |
| own | 1 | 10 | 40 | 51 | [8,8,7,8,9] | **完全同一** |
| off | 3 | 10 | 43 | 51 | [8,10,8,8,9] | — |
| persist | 3 | 10 | 43 | 51 | [8,10,8,8,9] | 同一 |
| own | 3 | 10 | 43 | 51 | [8,10,8,8,9] | **完全同一** |
| off | 42 | 10 | 42 | 51 | [8,8,8,8,10] | — |
| persist | 42 | 10 | **43** (+1) | **47** (−4) | [8,10,8,8,**9**] | D+1/B−4 |
| own | 42 | 10 | 42 | 51 | [8,8,8,8,10] | **完全同一** |

- `own` ≡ `off`: 3 seed すべてバイト一致計画 — 実装通り（機構上、フェーズ2で両者は同じ World を作る）。A/B 表 3.1 の own 列の差はすべて壁時計揺れ。
- `persist`: 3 seed 中 2 seed で計画が変化。score 差は **B +2 / B −4 / D +1** — 符号が揺れ、一貫した利得なし。V は不変。

## 4. 判定（正直な結論）

**Negative（効果なし・Close 推奨）**、条件付きの観察あり。

1. **本番設定（budget_ms=10000）での D/B 進 улучшは立証できなかった**: D は +0/+0/+0.7（揺れ幅内）、B は ±0。親Issueの受入条件「D または B が増える」をこの盤面で満たす証拠は得られなかった。
2. 決定論比較でも `persist` の効果は符号不定 (B+2/0/−4, D0/0+1)。機構自体は動作している（計画が実際に変わり、日別 D が動く日がある）が、**予測（翌日=当日観測）がこの盤面で外れている**可能性が高い: 観測では busy/jammed 道路が日ごとに大きく移動し（day3 busy14→day4 busy17 など）、持ち越し仮定は翌日の実道路と一致しない。
3. `own` は `players` が solver から見えないため補正を実装できず、未補正（team_count=1）のままでは `off` と区別できない。チーム数を solver に渡す改修（別 Issue: SimConfig/コンテキスト経由）が先になる。
4. 回帰面は完全クリア（既定バイト一致・10/10 テスト・clippy/fmt）。param は将来の予測改善のための足場として残す価値があるが、**このまま本番設定に入れる根拠はない**。

## 5. 再現コマンド

```bash
# LIVE 設定（本番相当）
cargo run -q --release -p bench --example round1_rollout -- \
  data/round1/derived/round1-match3.case.json data/round1/20261010T044644Z-p28796/events.jsonl \
  label=408-persist-s1 mode=solo traffic=observed \
  candidate='fleet:budget=official:budget_ms=10000:adapt=off:traffic=persist'

# 決定論比較
cargo run -q --release -p bench --example round1_rollout -- \
  data/round1/derived/round1-match3.case.json data/round1/20261010T044644Z-p28796/events.jsonl \
  label=det mode=solo traffic=observed \
  candidate='fleet:budget=official:iters=20000:seed=1:chains=2:traffic=persist'

# 回帰
cargo test -p bench --test fleet_candidate
PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" cargo run -q -p bench --bin procon-eval -- \
  --cases small-8x8 --candidates 'fleet:budget=official:iters=20000,wait' --baseline wait \
  --output-dir target/eval408-after --quiet
```

生出力: `target/rollout408/408-{off,persist,own}-s{1,3,42}.jsonl`（worktree 内、未コミット）。
