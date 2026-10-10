# convoy の clocked 劣後原因分析（hard-32-a8-d7-short-s9012 / hold-32-a6-d6-mixed-s5023）

2026-10-10。原因分析チーム（read-only + 本ファイルのみ書き込み）。測定の新規実行はしていない。
対象: PR #407 `crates/bench/src/adapters/convoy_solver.rs`（worktree eval-wsl-405, origin/405@3d30381 と同一ソース）。

---

## 1. 結論（先に）

**主因は (a) embedded fleet の予算不足**。budget_ms 指定時の convoy は、hard-32 / hold-32 盤面では実質「embedded fleet @ (budget/4).min(4000)ms の計画」を回答している。@10s では embedded fleet の探索量が standalone fleet @10s の約 1/4 であり、この差が ball 差のほぼ全額を説明する。

**(b) beam rollout の予測誤差は観測されない**。clocked ログの中に「beam が完走して選んだ計画が実際に悪い」と示す証拠はなく、32×32 では beam がそもそも完走しない（`complete=false` → 常に pool[0] を選択）ため beam の判定が結果にほぼ参加していない。むしろ no-clock（pool[0] = 本体 fleet と同一計画、beam 完走）では convoy が fleet に勝っており、beam 自体の価値は確認されている。

確信度: (a) dominant ≈ 80–85%、(b) は「存在してもマイナー」（確信度低、直接観測なし）。

---

## 2. 行レベルの機構（コード確認済みの事実）

`crates/bench/src/adapters/convoy_solver.rs`:

| 行 | 事実 |
|---|---|
| `convoy_solver.rs:1110-1117` | `window` = budget_ms（remaining_ms と調整）。 |
| `convoy_solver.rs:1119` | `day_plans(ctx, true, started + w/3)` — **pool 生成（fleet 計画 + label search 一式）に window の 1/3 のみ割当**。 |
| `convoy_solver.rs:127-134` | embedded `FleetCandidate` は `iters=60000`（`fleet_iters` 既定）、**`budget_ms = (budget_ms/4).min(4000)`**。@10s → 2.5s、@20s → 5s、@40s 以上でも cap 4s。 |
| `convoy_solver.rs:802-808` | `day_plans` は最初に `fleet.decide(&ctx)` を `keep` する → **pool[0] は常に embedded fleet の計画**（deadline を `ctx.remaining_ms` に変換して渡す）。 |
| `convoy_solver.rs:811-816` | 盤面が label search の前提外（spots>32, patrols>8 など）なら pool = [fleet 計画] のみで早期 return。 |
| `convoy_solver.rs:461-465, 817-824` | label search は cap 1.5M labels、deadline は 8192 labels ごとにチェック。超過で `None` → `day_plans` は**そこまでに溜まった pool だけで早期 return**（fleet 計画は常に含まれる）。 |
| `convoy_solver.rs:1123-1124` | `depth = self.depth(既定 8).min(残り日数)`。 |
| `convoy_solver.rs:1128-1133` | scenario（persist / busy）ごとに残り window を均等分割して `until` を設定。 |
| `convoy_solver.rs:1082-1091` | `ahead` は 1 日展開が終わった時点で `until` 超過なら break。**このとき `reached` は increment されない**。 |
| `convoy_solver.rs:1146` | `complete &= reached == depth`（全 scenario が全 depth を完走して初めて true）。 |
| `convoy_solver.rs:1153-1156` | 偏離条件は `complete && sums[k] > sums[0]`（合計タプルの**厳密な**辞書順優越）。不成立なら `best = 0` = **pool[0] をそのまま回答**。pool 内の convoy 計画は計算して捨てられる。 |
| `convoy_solver.rs:1093-1100` | `ahead` 完走時、root ごとの best は beam 内の最大 total。 |
| `convoy_solver.rs:1049-1052` | root==0（fleet 由来経路）は beam 幅外でも常に保持（比較の公平性）。 |

`crates/bench/src/adapters/fleet_solver.rs`:

| 行 | 事実 |
|---|---|
| `fleet_solver.rs:950-970` | live（window あり）search は **wall-clock のみで打ち切り**。`iters` は live では不発動（品質 ∝ 与えた時間）。 |
| `fleet_solver.rs:1192-1214` | window = budget_ms。`adapt` は window ありのときのみ効く（embedded は `adapt:false` なので純粋に budget_ms まで使う）。 |
| `fleet_solver.rs:1219-1229` | live は chains スレッド並列、計画フェーズは window の 7/10（@10s → chain ごと ~7s）。 |
| `fleet_solver.rs:1194-1197` | コメントの収束目安: **16×16 は 0.25s から平坦、24×24 は ~2s、32×32 は 4s でも伸び続ける**。収束予算のスケール式は `(patrols × spots)² / 3`（:1198-1199）。 |
| `fleet_solver.rs:1132-1134` | `seed ^ (day<<32) ^ (k<<48)` — 日ごとに決定論的。 |

**no-clock 時の同一計画性**: budget_ms=0 → window=None → live=false → chains を順次実行、iters=60000:chains=4、deadline なし。standalone `fleet:iters=60000:seed=42:chains=4`（offline, `adapt` は window なしだので不発動）と seed 派生も同一 → **pool[0] は本体 fleet と同一の計画**。holdout no-clock で convoy が優位だったこと（`holdout-honsen-like.runs.jsonl`: 27 case 中 convoy 勝ち ~13・引き分け ~13・負け ≤2 を最大 2 balls）は「pool[0] が公平で beam が完走できるとき、beam deviation は正の価値を持つ」ことを示す。

---

## 3. 証拠の分割

### 3.1 盤面サイズ × 勝敗パターン（b10s-seed42 / seed7、同一実行内比較）

convoy@10s − fleet@10s（balls）:

| 盤面 | サイズ | seed42 | seed7 |
|---|---|---|---|
| hard-16-a3-d9-s9003 | 16×16 | +2 | +2 |
| hold-16-a4-d7-s5001 | 16×16 | +2 | 0 |
| hard-24-a5-d7-s9005 | 24×24 | −10 | 0 |
| hard-24-a6-d9-s9008 | 24×24 | **+28** | −2 |
| hold-24-a5-d4-s5013 | 24×24 | −1 | 0 |
| hold-24-a6-d10-s5016 | 24×24 | +1 | −2 |
| hard-32-a8-d7-s9012 | 32×32 | **−37** | **−38** |
| hold-32-a6-d6-mixed-s5023 | 32×32 | −1 | **−24** |
| hold-32-a8-d10-s5020 | 32×32 | −7 | −9 |

負けは 32×32 に集中。収束式で概算すると embedded 2.5s が収束点を下回るのはちょうど 32×32（s9012: (8×spots)²/3 ≒ 10–19s、s5023: 6×spots ≒ 8–9s）で、16×16（0.25s）では余裕、24×24（~2–5s）は境界 — 勝敗パターンと一致する。**D 飽和かどうかでは分割できない**（s9008 も dV=180=上限の飽和盤面で convoy が +28）。

### 3.2 (a) の裏付け（量的）

s9012 の balls:

| 候補 | B (seed42) | B (seed7) | 出典 |
|---|---|---|---|
| fleet @10s | 406 | 425 | b10s |
| fleet chains8 @10s | 433 | 418 | b10s |
| **convoy @10s** | **369** | **387** | b10s |
| **convoy @20s** | **388** | — | b2（同一実行: fleet@10s=396, chains8=415） |
| fleet iters=60k offline | 373 | — | corpus（実測 ~2.3s/日相当） |
| fleet iters=120k offline | 390 | — | corpus |

- convoy@10s（= fleet@2.5s 相当の計画を回答）は iters60k offline（373）と同水準で、fleet@10s（406/425）から 37–38 balls 落ちている。
- convoy@20s では embedded 予算が 2.5s→5s になり、同一実行内の差が −37 → −8 に縮んだ。embedded 予算にほぼ比例して追いつく。
- 収束コメント（`fleet_solver.rs:1194-1197`）と整合: 32×32 は 4s でも伸び続けるため 2.5s は探索量として決定的に不足する。
- 負荷ノイズの確認: fleet@10s は実行間で 406↔396、chains8 は 433↔415（±10–18）。b10s と b2 の跨ぎ比較はノイズが大きく、上記は同一実行内（b2 では convoy@20s 388 vs fleet@10s 396 vs chains8 415）で閉じて判断した。
- daily_response_us: convoy は s9012 の day0–5 で 10.0–10.1s を使い切り、最終日のみ ~3.4s。つまり**後半 6.7s（beam 分）を消費しながら出力は fleet@2.5s の計画**という「時間を捨てている」形跡がそのまま残っている（`complete=false` → chosen=0 の帰結）。

### 3.3 (b) の評価

- clocked ログに「beam 完走時に実際の結果より悪い計画を選んだ」直接証拠はない（tracing `convoy day plan` の pool/complete/chosen がどこにも保存されていないため、chosen の実測は不可能 — §4 P0 参照）。
- 最終日（remaining=0 → depth=0）は beam なしで pool の即時 replay 値（公式シミュレータの実測値）による argmax なので、**予測誤差は原理的に発生しない**。
- s9005（24×24, −10）は後半日で beam が完走して deviation した可能性があるが、embedded 2.5s vs 収束 ~3.3–4.8s の (a) でも説明がつく（convoy 202 ≒ iters60k offline 205）。
- no-clock では beam が完走し、D 飽和盤面（honsen-like dV=50=上限）でも balls を増やしている（s7001: 86 vs 77、s7005: 90 vs 78、s7011: 95 vs 87）— **(b) の「beam が実際より悪い計画を選ぶ」証拠ではなく逆の証拠**。
- 注意点（観察、未検証）: D 飽和日（fresh/series が全計画で共通）では team step の評価が balls×100 + end_value（最大 400、`VARIANTS` convoy_solver.rs:174、`end_value` :588-596）になり、end-state 項が ball 1 個を上回りうる。beam が深く完走するなら rollout が補正するが、depth≤1 の浅い deviation（s9012 の残り 1–2 日の日）ではこの歪みが漏れうる。優先度は低。

### 3.4 pool 生成自体の完走性（未観測のリスク）

pool 生成予算は w/3 = 3.3s で、そのうち embedded fleet が 2.5s 消費するため、label search 系（8 patrols × Tank/Escort/Touch + riders + 手配 replay）には ~0.8s しか残らない。32×32-a8 では deadline で中途打ち切り（`convoy_solver.rs:461-465`）→ pool が [fleet 計画のみ] や部分集合になる可能性が高い。この場合でも出力は pool[0] なので (a) の結論は変わらないが、**最終日 depth=0 の即時選択で convoy 計画が選べる機会が減る**ことと、P2（pool[0] 強化）実施時に pool 生成がさらに圧迫されることが影響する。実測には P0 の tracing が必要。

---

## 4. 最小検証条件の提案（実装・測定チーム向け。本分析では実施しない）

共通注意: wall-clock 結果は負荷に敏感（fleet@10s で 406↔396）。判定は必ず同一実行内の候補間比較で行う。s9012 1 盤面 × 数候補で完了する軽量なものだけを列挙する。

- **P0: tracing 収集（最優先・コード変更なし）**
  `tracing::info!("convoy day plan", day, pool, depth, complete, chosen, ...)`（`convoy_solver.rs:1157-1165`）は既に実装済み。s9012 × convoy@10s:seed42 を 1 回走らせ、ランナーで tracing を捕まえるだけ。
  判定基準: day0–5 で `complete=false, chosen=0` が支配的（予測: 6/7 日）なら (a) 確定。`chosen≠0` が複数日あれば (b) の再分割に進む。`pool` の日別サイズで §3.4 の pool 崩壊も確認できる。

- **P1: embedded 予算ラダー（コード変更なし・(a) の直接検証）**
  s9012 に `fleet:budget_ms=2500:adapt=off:seed=42` と `fleet:budget_ms=5000:adapt=off:seed=42` を実行（convoy の pool[0] と同じ予算・同じ seed 派生）。
  判定基準（可否の予測）: fleet@2.5s ≈ convoy@10s（369±15）、fleet@5s ≈ convoy@20s（388±10）なら (a) 確定。**fleet@2.5s より convoy@10s が明確に下回る（>15 balls）なら**、後半日の浅い deviation や pool 崩壊由来の (b) 系寄与が実在することになる。

- **P2: 修正案 A — pool[0] の予算強化（コード変更あり）**
  `(budget_ms/4).min(4000)` を、収束スケール `(patrols×spots)²/3` を参照するか、あるいは単純に `budget_ms − pool生成の label search 分`（例: budget−1500ms、cap 緩和）に引き上げ。fleet decide と label search の window 配分（現在 fleet が w/3 をほぼ消費）の再構成を伴う。
  可否の予測: s9012 / s5023 @10s で変更版 convoy が fleet@10s の ±5 balls に追いつく。16×16 / 24×24 で現行より非劣化（P0 の `pool` サイズで label search の圧迫を監視）。
  反証条件: 追いつかない場合、beam 側に負の寄与がある = (b) 系の修正（下記 P4）が必要。

- **P3: コード変更なしで beam を完走させる方向**
  `convoy:budget_ms=10000:forecast=persist`（scenario を 1 つにして beam 時間 2 倍）を s9012 で実行。
  予測: 後半日で `complete` が増え deviation が増える。改善幅が +0〜+15 程度なら (a) dominant の裏付け。**+25 以上出るなら**「beam が完走すれば convoy 計画の利得が大きい」ことになり、pool[0] 弱さの影響被覆の見直し（P2 の設計）に反映する。

- **P4: 「D 飽和日に beam 偏離を禁止する gate」は非推奨**
  no-clock holdout で D 飽和盤面でも beam deviation が balls を増やしている（s7001 +9、s7005 +12、s7011 +8）ため、この gate はその利得を消す。gate を入れるなら「beam が完走できないと予測される日（残り depth × 1 日展開コストの経験則）で、残り window を pool[0] 生成へ再配分する」方向が証拠と整合する。P0 の tracing で日別の 1 日展開コストを実測してから設計すること。

- **P5（低優先度・観察項目）**
  `W_BALL=100` に対し end-state 項が最大 400（`VARIANTS`/`end_value`）で、D 飽和日の team step が ball より燃料 end-state を優先しうる。P0/P1 で (a) が確定した後、残差が残る場合のみ評価する。

---

## 5. 検証結果の追記（2026-10-10 後半・P1/P3 実測による結論修正）

§4 の検証条件が実測された（`wsl-10s/p1-embed-rung/`・`p3-persist-s9012/`、solver コードは本書と同一系譜 — commit 640e728/812290b は記録コミットのみで solver 変更なし）:

| 実測（s9012 seed42, V27/dV189 共通） | B |
|---|---|
| **fleet @2.5s（embedded と同一予算・chains=1）** | **416** |
| fleet @5s | 408 |
| fleet @10s（b10s/b3/c2 を通じて） | 406 / 407 / 411 |
| convoy @10s（both） | 369 |
| **convoy @10s forecast=persist（P3）** | **407** |

- **(a)「embedded fleet 予算不足」は主因説として反証された**。§1 の予測分岐（P1: convoy@10s が fleet@2.5s より明確に下回る >15 balls → (b) 系寄与が実在）が +47 で発火。2.5s の standalone fleet は既に fleet@10s 同水準であり、予算を (budget/4) から引き上げるだけの修正（S2/P2 単独）では s9012 の差は埋まらない。
- **P3 は +38 回復**（369 → 407 ≈ fleet@10s）。beam に時間が戻ると deviation が正の価値を持つことの裏付け。ただし both モードで incomplete 日の出力が pool[0]（= fleet@2.5s ≈ 416 相当のはず）であることと実測 369 の齟齬は残る — pool 生成の実効予算か、完走日の浅い deviation での予測誤差かの分解には P0 tracing（`convoy day plan` の pool/complete/chosen 日別実測）が依然必須。§2 の機構表の行はコード上の事実として正しいが、§1 の確信度配分は「(a) dominant ≈80–85%」から「beam/pool 側の時間配分・選択が主因」へ修正する。
- fleet 収束コメント（`fleet_solver.rs:1249-1253` の 32×32 は 4s でも伸びる）は s9012/chains1/seed42 の実測（2.5s=416, 5s=408, 10s=406–411）とは整合しない。収束スケール式の当てはめは boards 間でばらつくため、§1.1 の budget_ms 感度行に実測上書き済み。
- @20s（embedded 5s）: b3-20s で s9012 convoy 412/407 vs fleet@10s 407/411 — 差はノイズ以下。@20s では convoy は 32×32 でも fleet に並ぶ。

## 6. 引用データの所在

- `docs/research/round1-20261010/results-405/wsl-10s/b10s-seed42/runs.jsonl`、`b10s-seed7/runs.jsonl`（9 盤面 × convoy/fleet/chains8 @10s）
- `docs/research/round1-20261010/results-405/wsl-10s/b2-20s-s9012/runs.jsonl`（convoy@20s / fleet@10s / chains8 同一実行）
- `docs/research/round1-20261010/results-405/wsl-10s/round1-solo.jsonl`（match3 の日別 decide_ms / daily_B / daily_D）
- `docs/research/round1-20261010/results-405/wsl-corpus/hard-fleet-iters60k-chains4/`、`hard-fleet-iters120k-chains4/`、`holdout-fleet-iters60k-chains4/`
- `docs/research/round1-20261010/results-405/holdout-honsen-like.runs.jsonl`（no-clock convoy vs fleet iters60k）
- `docs/research/round1-20261010/results-405/convoy-shape.jsonl`、`convoy-seeds.jsonl`（match3 上の depth/width/seed 感度）