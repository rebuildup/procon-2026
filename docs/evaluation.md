# ソルバ評価（canonical evaluation）

大量の解法・パラメータ・ケースを**継続的かつ再現可能**に比較するための、唯一の評価経路です。
Issue #178 が定めるもので、実装本体は `crates/bench` にあります。

## 経路は一方向で固定されている

```text
candidate solver
    ↓   行動・計画のみ（採点しない）
actions / decisions
    ↓
engine::MatchSim
    ↓
final MatchState / RunResult
    ↓   評価器が採点する
canonical evaluator
    ↓
official score tuple
    (variety, daily_variety_sum, total_balls, response_time)
    ↓
durable result（runs.jsonl）
```

**不変条件: solver は自分自身を採点しない。**
candidate が返せるのは `engine::model::AgentActions`（行動）だけであり、
競技結果は必ず `engine::MatchSim` の最終状態から evaluator が算出します。
solver の内部ヒューリスティック（例: `solver-loop` の `ScoreReport::judge`）は
探索のための評価関数であって、採点とは無関係です — evaluator はそれを読みません。

## スコアと順位

スコア型は `domain::OfficialScore`、比較は `domain::compare_scores` です。
**辞書順で、重み付き和に潰しません。**

| # | 指標 | 方向 | 定義 |
|---|---|---|---|
| 1 | `variety` | 大きいほど良い | マッチを通じて取得した系列の異なり数 |
| 2 | `daily_variety_sum` | 大きいほど良い | 各日の種類数の累積 |
| 3 | `total_balls` | 大きいほど良い | 総玉数 |
| 4 | `response_time` | **小さいほど良い** | 各日の最終回答時間の累積 |

先に差が出た段階が最終決定で、以降は比較しません。

### response_time と wall-clock elapsed は別物

| 項目 | 単位 | 用途 |
|---|---|---|
| `response_time_us` | µs | **4段目の評価軸**。各日 `decide()` に費やした時間の合計。公式の「各日の最終回答時刻の累積」の offline における近似。 |
| `elapsed_us` | µs | 全体の wall-clock。シミュレーション本体の実行時間も含む。環境ノイズを含むため**比較には使いません**。 |

第4位は測定値であり、再現実行の間で振れます。したがって**再現性の保証は第1〜3位まで**
（+ `last_acquisition_at`、`daily_varieties` の内訳）です。
`case.deterministic_projection()` は測定値を除いた順位だけを返します。

> `last_acquisition_at`（最新の初回取得時刻）は診断用の証拠であり、順位の段ではありません。
> 募集要項「順位決定」の第4位は応答時間です。

## 標準手順: solver 開発 → corpus 評価 → baseline 比較

### 1. candidate を決める

candidate ID の形は `アルゴリズム[:キー=値]...` で、アルゴリズム + パラメータを
まとめて指します。パラメータは記書順のまま保持され、`parse → id` で完全に往復します。

```bash
just eval-list
```

登録済み:

```text
loop                                  iterations / initial / seed
greedy
wait                                  （baseline — 全段0）
```

問題の例に挙がっている ID はすべて実行可能です:

```text
loop:iterations=32:initial=greedy
loop:iterations=128:initial=greedy
loop:iterations=64:initial=random:seed=42
loop:initial=sweep-row
loop:initial=sweep-col
greedy
```

`initial` の候補: `greedy`（既定）/ `identity` / `sweep-row` / `sweep-col` / `random`。
`initial=random` の時は `seed` が必須で、同じ seed は常に同じツアーを返します
（xorshift64* を使い、外部 RNG crate に依存しません）。

**1 candidate = 1 crate ではありません。** 新しいアルゴリズムは
`bench::CandidateFactory` を実装して `CandidateRegistry` に登録するだけです。
単一 crate が複数アルゴリズムを出ることも、単一アルゴリズムが多数のパラメータ集合を
持つこともできます。数十アルゴリズム × 多数パラメータは、追加の crate ではなく
ただの ID の列です。

### 2. corpus 評価を回す

```bash
# 既定の candidate 集合を全 fixture に適用し wait と比較
just eval

# コマンドを直接叩く
PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" \
  cargo run -q -p bench --bin procon-eval -- \
    --cases small-8x8,medium-16x16,series-many \
    --candidates 'loop:iterations=32:initial=greedy,greedy,wait' \
    --baseline wait \
    --output-dir target/eval
```

`--cases all`（既定）は `crates/bench/fixtures/maps/` の 8 件すべてを使います。

### 3. baseline と比較する

`--baseline` に指定した candidate を基準に win / draw / lose を数えます。
両者がスコアを出した case のみ比較し、それ以外は `not compared` に落入ります
（数値でごまかさないためです）。

`target/eval/report.md` を開くか、`target/eval/aggregate.json` を機械処理してください:

```bash
jq '.per_candidate[] | {candidate_id, wins, draws, losses, timeouts, invalids, panics}' \
  target/eval/aggregate.json
jq -s 'group_by(.case_id) | length' target/eval/runs.jsonl   # case 数
```

### 4. 回帰検知する

`runs.jsonl` は追記型の JSONL で、1 行が 1 (candidate × case) です。
単一行が壊れてもバッチ全体は失効しません。後から別条件で再集計できます:

```rust
let runs = bench::report::read_runs(Path::new("target/eval/runs.jsonl"))?;
let report = bench::AggregateReport::build(runs, Some("wait"));
```

## case の schema

```text
case_id                 一意。fixture:<name> または generated:<name>
seed                    generated case のときのみ
generation_parameters    generated case のときのみ（自由形式 JSON）
source                  fixture | generated
setting                 本体（公式の試合設定 JSON 形状）
```

固定 fixture はそれ自体が真値なので `seed` を持ちません。将来の random map
generator は `CaseSpec::generated(case_id, seed, params, setting)` を通るだけで、
評価器の再設計は不要です — `case_id` / `seed` / `generation_parameters` は
すべて `RunRecord` に保存されます。

## durable result

`runs.jsonl` の 1 行（`bench::RunRecord`）:

```jsonc
{
  "candidate_id": "loop:iterations=32:initial=greedy",
  "case_id": "fixture:small-8x8",
  "seed": null,
  "generation_parameters": null,
  "source_commit_sha": "f57b35ac…",

  "variety": 3,                  // 第1位
  "daily_variety_sum": 4,        // 第2位
  "total_balls": 5,              // 第3位
  "response_time_us": 1617,      // 第4位（低いほど良い）

  "elapsed_us": 8421,            // 証拠のみ。順位には使わない
  "scored": true,

  "status": "success",           // success | invalid | timeout | panic
  "timeout": false,
  "status_detail": null,
  "last_acquisition_at": [1, 7],
  "daily_response_us": [830, 402, 385],
  "daily_varieties": [1, 2, 1]
}
```

### status の意味

| status | 実行 | スコア |
|---|---|---|
| `success` | 完走、全日が契約を満たした | **あり** |
| `invalid` | 完走したが契約違反（→ 期待より低い実績）／ engine が case を棄却 | **あり** |
| `timeout` | wall deadline 超過 | なし |
| `panic` | candidate が panic | なし |

`scored` が真正の判定です。`status` だけでは `invalid` の有効スコア有無を判別できないため、
集計は必ず `RunRecord::is_scored()` を使います。

### failure isolation

各 (candidate, case) は独立した worker thread と wall deadline の下で走ります。
1 つの candidate が panic しても、 hangs しても、行動契約を破っても、
その run の記録になるだけでバッチ全体は失いません。

## 集計結果

`AggregateReport` は `runs.jsonl` から純粋な関数として再生成できます:

- **per-case ranking** — 各 case 内の順位（同点は同ランク）
- **baseline に対する win / draw / lose**
- **timeout / invalid / panic count**
- **score tuple summary** — 各指標の min / max / mean / n

## `procon tournament` とは別物

`procon tournament`（`crates/cli`）は**公式サーバ / プロトコル / coordinator の
E2E smoke path** です。返るのは `protocol_progress = days_completed * 100 + sum(revisions)`
で、これは「プロトコルとマッチループがどこまで進んだか」を測る接続試験であり、
**ソルバの競技性能ではありません**。フィールド名を `protocol_progress` と変更したのは
この混同を防ぐためです。

- 競技性能を測りたい → `just eval`（offline、`MatchSim` 経由）
- サーバ疎通・プロトコル統合を確認したい → `procon tournament`

## 再現性

同じ commit + 同一 candidate config + 同一 case なら、
**wall-clock timing を除く**競技得点が同一になります。
`MatchSim` には RNG・clock・I/O がなく、`step_towards` の方向探索、
`seeded_shuffle` の順序も全て決定的です。

検証: `crates/bench/tests/canonical_evaluation.rs`
（3 cases × 2 real solver variants の E2E、決定性、failure isolation を含む）。

```bash
cargo test -p bench --locked
```
