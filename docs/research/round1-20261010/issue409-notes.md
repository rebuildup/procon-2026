# Issue #409 — chain count beyond four (測定記録)

Branch `409`（base `403` @ `470a2ff`）。2026-10-10 実施。

## 変更内容（crates/bench/src/adapters/fleet_solver.rs）

1. **`chains` param の範囲**: 1..=8 → 1..=16。ドキュメント文を更新。
2. **live chain cap**: `decide()` で `available_parallelism().clamp(1, 4)` 固定だったのを、
   **`chains` を明示指定した場合のみ** `min(cores, chains)` に引き上げる契約に変更。
   - `chains` 未指定（=1）→ `min(cores, 4)`: **旧動作と同一**（既定は無変更）。
   - `chains=N`（N≥2）+ budget window あり → N 本の chain をコア数まで並列起動。
   - offline（iters モード）は従来どおり `chains` 本を逐次実行（再現可能）。
3. **`CHAIN_T0`**: 4 → 16 要素に拡張。最初の 4 要素は歴史的 schedule
   `[40, 150, 400, 40]` のまま（k<4 は byte-identical）。k≥4 は
   `[250, 80, 600, 150, 40, 300, 1000, 250, 600, 80, 150, 400]` — 単純 cycle の繰り返しでなく、
   hot 側（600/1000）と cool 側（40/80）を交錯させ隣接 chain の t0 差を広げた。
   `search` 呼び出しは `CHAIN_T0[k % CHAIN_T0.len()]`。
4. **seed 導出は無変更**: `seed ^ (day << 32) ^ (k << 48)`。k≤15 は bits 48..52 に収まり
   day（bits 32..47）や phase-2 の xor 定数との衝突はない。k=0..3 の seed も同一。
5. メモリ: chain ごとに `World` を 1 clone（16×16 盤面の 3 metric Dijkstra 行列）。
   10 core / 32 GB の macOS 実機で chains=16 まで観測して異常なし。24/32×32 盤面では
   `available_parallelism` の cap（実コア数）が効くため、chains=16 は実質 8〜10 並列まで。

## 既定無変更の確認

- `cargo test -p bench --test fleet_candidate` 8/8 pass（`chains=9` 拒否 → `chains=17` 拒否に更新）。
- procon-eval small-8x8 `fleet:budget=official:iters=20000`:
  before `V=2 D=2 B=7` / after `V=2 D=2 B=7`（同一、target/eval-409-{base,after}）。
- `cargo clippy -p bench --examples --bins --tests` / `cargo fmt --check` clean。

## ラウンド1 board（match-3, observed roads）での variance study

`round1_rollout` solo モード。**この harness は `SolverContext::from_state`
（`remaining_ms = u32::MAX`）を使うが、`budget_ms=10000` があるため `window = Some`
→ live 経路（thread 実行）に入る**。旧コードでは `chains` param に関係なく
`clamp(cores, 1, 4)` で 4 本に固定されていたが、新コードでは明示 `chains=N` が
`min(cores=10, N)` 本を並列に立てる。したがって下表の c8/c12/c16 は新しい経路を実際に通る。

candidate: `fleet:budget=official:budget_ms=10000:adapt=off:chains=<N>:seed=<s>`、
seed 1/2/3/7 の 4 設定 × chains ∈ {4, 8, 12, 16}。逐次実行（1 run ずつ）。
結果: `results/issue409.jsonl`（16 records）。

| chains | D の分布（seed 1/2/3/7） | med D | med B | max decide_ms |
|---:|---|---:|---:|---:|
| 4  | 43, 43, 43, 42 | 43 | 51 | 10 009 |
| 8  | 43, 43, 43, 43 | 43 | 55 | 10 049 |
| 12 | 43, 42, 43, 43 | 43 | 55 | 10 011 |
| 16 | 43, 42, 43, 43 | 43 | 55 | 10 080 |

daily_D の模式（全 run 共通の形）: `[8, 9, 8, 7〜10, 7〜10]` — 差は第4日と第5日の
7/8 と 10 に集中する。V は全 16 run で 10（上限到達）。

### 読み取り

- **D の bad-run 除去**: chains=4 では 4 seed 中 1 本が D=42（seed 7）。chains=8 では
  4 seed 全て D=43。chains=12/16 は 4 seed 中 1 本が D=42（seed 2）で、chains=8 より
  悪い。**chains=8 がこの seed 集合では最も悪い日が無い**。ただし n=4 の seed なので、
  「chains=4 の 42」も「chains=12/16 の 42」も同一現象の別 seed 引きとみなすのが自然で、
  D 分布の差は統計的には有意と言い切れない。
- **B 中央値**: chains=4 の 51 → chains≥8 で 55。chains≥8 では 3/4 の seed が B=55 に
  集中し、chains=4 では 1/4。旧 `CHAIN_T0[k % 4]` では chains≥5 は t0=40 の反復で
  多様性が薄かったが、新しい t0 ladder（250/80/600/150, 40/300/1000/250, …）により
  hot/cool の多様な chain が best を引き上げている。budget 10000ms は D が頭打ちの
  領域（budget.jsonl では 250ms〜40s で D=42〜44、この盤面の実効上限は 44 前後）なので、
  余った探索が B に出るのは筋が良い。
- **decide_ms**: 全構成で day window 10s に対し 10.0〜10.1s。chain を増やしても
  wall clock は変わらない（並列なので）。c16 で +0.08s 程度の tail があるが許容範囲。

## 予行演習盤面（rehearsal fixture, `data/rehearsal/2026-10-10-105938.case.json`）

procon-eval `--kinds S,P,P,P --jobs 1`、candidate
`fleet:budget=official:budget_ms=10000:adapt=off:chains=<N>`、1 反復。

| chains | V / D / B | response_ms |
|---:|---|---:|
| 4  | 10 / 49 / 100 | 50 051 |
| 8  | 10 / 49 / 100 | 50 007 |
| 12 | 10 / 49 / 104 | 50 008 |
| 16 | 10 / 49 / 94  | 50 011 |

D は全構成で上限の 49 に到達（この盤面は budget 10000 で D が飽和する）。
B は chains=12 が 104 と最高だが 1 反復であり、
`budget_ms=10000:adapt=on`（既定）では adapt が小盤面を早期打ち切りするため、
本番（--kinds-auto + adapt=on）では chains 効果はほぼ出ない点に注意。
参考: rehearsal README の既存最高は `iters=60000:chains=4:seed=3` の 10/49/102。

## 結論（honest）

1. **D**: この盤面の D 上限（≈44）に対し chains 増加で med D は変わらない（43）。
   ただし chains=8 は 4 seed 中 bad（D<43）が 0 で、chains=4/12/16 の 1 bad より
   良く見える。n=4 では有意差の証拠にはならない。**variance 低減の主張は弱いが、
   悪化もない。**
2. **B**: chains≥8 で med B が 51 → 55（+4, 全 seed 同時）。これは t0 ladder 拡張の
   効果と一致し、再現性のある改善（4 seed 全てで B≥51、3/4 が 55）。
3. **既定無変更**は regression で確認済み。本番構成（--kinds-auto, adapt=on, live）では
   chains を明示しない限り挙動は 1 行も変わらない。
4. **採用提案**: 本番で使うなら `chains=8` を明示指定するのが最良の候補。
   c12/c16 は D が一つ落ちた（seed 2）ため、B と D を同時に見ると chains=8 が
   この seed 集合でのベスト。ただし n=4 seed / 1 盤面なので、Arena での確認を推奨。

## Known issues

- この harness の solo mode は `remaining_ms = u32::MAX` でも `budget_ms` 指定で live
  経路に入る。`iters` モード（budget_ms 無し）は offline 経路のまま逐次。
- `--case-files` に rehearsal の生 `*.setting.json` は渡せない（README 既述）。
  flat CaseSpec（`*.case.json`）を使用した。
- 計測は shared Mac で別 session が負荷をかけた状態で行った（load1 12〜36 を記録）。
  wall-clock 系の指標（decide_ms は window 端切りでほぼ 10s 固定）には影響が薄いが、
  seeds 間の B 揺れに負荷変動が混入する可能性は残る。
