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

## 公式条件をそのまま流す

評価器は case が宣言した値を**すべて**そのまま engine に渡します。値を推測・固定すると、
「この case を評価した」ことになりません。

| wire の値 | 扱い |
|---|---|
| `fuelLimits` | 巡回車の燃料上限。**そのまま。** 以前は `daySteps[0] * 3` を推定していたため `small-8x8` (24) を 48 で評価していた |
| `daySteps` (per-day) | `MatchSetting::day_steps_by_day` に保持され、**`MatchSim::run_inner` が day loop 内で各日の値を読む**ので `daySteps=[16,24]` なら day1 は本当に24 step 実行する |
| `daySeconds` (per-day) | **公式 validity として判定**。各日の回答時間 `daily_response_us[i]` が `day_budget_us()[i]` を超えていればその日は未受理と見なして `invalid` / `scored=false`。長さが `daySteps` と食い違う・非正なら from_wire が拒否 |
| `busyThreshold` / `jammedThreshold` | `CaseSpec::sim_config()` → `SimConfig`。**`SimConfig::default()` (2/5) を使わない** — fixture は 1/2 なので別ゲームになる |

agent の種類（巡回/補給）は wire に無いため依然 index 交替の既定。種類の決定と燃料上限は別問題です。

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
まとめて指します。identity は 2 層です。

| 層 | 経路 | 保証 |
|---|---|---|
| 構文 | `CandidateSpec::parse → id` | 記書順のまま完全に往復。registry 無しで id を echo できる |
| **canonical identity** | `CandidateRegistry::resolve`（= `build` / `build_all`） | 重複キーを拒否し、パラメータを `CandidateFactory::params()` の schema 順に並べ替える |

canonical order の SoT は factory の schema だけで、`loop` なら `iterations / initial / seed`
の順です。並べ替えは validation の後に行われるので、未知キーは既に拒否済みで
どの present key も必ず 1 つずつ残ります。

したがって **書き順は identity ではありません**:
`loop:iterations=32:initial=greedy` と `loop:initial=greedy:iterations=32` は
同一 candidate です。同じ batch に両方を書くと duplicate error になり、
二重評価されません。キーの重複
（`loop:iterations=32:iterations=128:initial=greedy`）も parser と registry の
両方で明示的に拒否されます — どちらの値が有効かが曖昧なため。

解決後の `Candidate::id()` が durable identity であり、`RunRecord.candidate_id` /
`AggregateReport` / レポートはすべてそれを読みます。

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

`--cases all`（既定）は `crates/bench/fixtures/maps/` の 9 件すべてを使います。

### 3. baseline と比較する

`--baseline` に指定した candidate を基準に win / draw / lose を数えます。
両者がスコアを出した case のみ比較し、それ以外は `not compared` に落入ります
（数値でごまかさないためです）。

### provenance

`AggregateReport::build` は **異なる `source_commit_sha` が混ざると拒否します**
（`AggregateError::MixedCommits`）。最後に読んだ 1 件の SHA で黙って上書きすれば、
別 commit の数値を同一 evidence として順位に混ぜられてしまいます。
`report.source_commit_shas` は**リスト**なので schema 上も複数 commit を表現できます。

`procon-eval` は「rankable な run が 1 件でも無い」場合に **exit 1** を返します
（records は先に書かれるので evidence と failure signal が両立します)。

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
// 混ざった commit はここで AggregateError::MixedCommits になる
let report = bench::AggregateReport::build(runs, Some("wait"))?;
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

  "team": "player-a",            // この run がプレイした team
  "status": "success",           // success | invalid | timeout | panic
  "timeout": false,
  "status_detail": null,
  "last_acquisition_at": [1, 7],

  "illegal_action_count": 0,     // 非法 action が 1 つでもあれば invalid / scored=false
  "fuel_short_count": 0,         // うち燃料不足由来
  "day_budget_exceeded_days": [], // daySeconds 超過日（0-based）。空でなければ invalid
  "daily_response_us": [830, 402, 385],
  "daily_varieties": [1, 2, 1]
}
```

### status の意味

| status | 実行 | 数値 | 順位対象 (`scored`) |
|---|---|---|---|
| `success` | 完走、全日が契約を満たした | あり | **はい** |
| `invalid` | 契約違反 / 非法 action / case 棄却 | 記録する | **いいえ** |
| `timeout` | wall deadline 超過 | なし | いいえ |
| `panic` | candidate が panic | なし | いいえ |

`scored` が真正の判定で、**順位・W/D/L は必ず `is_scored()` で絞る**必要があります。

### `invalid` は順位から外す

`MatchSim` は不正な action を `Wait` へ **downgrade して試合を完走させる**。完走は
「公式に受理される」こととは別物です（Q&A Q6 は無効 action を含む回答を丸ごと棄却する）。
そこで engine は次の計数を返します:

- `illegal_action_count` — 範囲外・通行不能・燃料不足で downgade された action 数
- `fuel_short_count` — うち燃料不足由来

これらが 1 つでもあれば `status = invalid` / `scored = false` にして順位対象から外します。
数値自体は `variety` 等に残るので、何を間違えたか追えます。

### isolation

### harness deadline vs official day window

これは**別物**です。1つの全体 deadline で N 個の daily budget を代替できません —
全体を伸ばせば1日で暴走した solver が `success` になり、縮めれば遅くない run まで kill されます。

| 概念 | フィールド | 用途 |
|---|---|---|
| harness deadline | `EvalConfig::harness_deadline` / `--harness-timeout-ms` (旧 `--timeout-ms` alias) | 動かなくなった worker を kill する保険。順位には使わない |
| 公式 day window | `CaseSpec::day_budget_us()` → `RunRecord::day_budget_exceeded_days` | 各日の `daySeconds`。超過した日があると `invalid` / 順位対象外 |

#### harness deadline は **case から導出**する（既定）

フラットな既定値は誤ります。`daySeconds = [60,60,60]` は**合計180秒の合法な回答時間**なので、
60秒のキャップは「各日 budget 内・合計60秒超」の合法な run を途中で kill します。

| ポリシー | 内容 |
|---|---|
| `HarnessDeadline::CaseWindow { margin }`（**既定**） | `sum(daySeconds) + margin`。合法な run は切られない。hang は公式 window 終了後に kill |
| `HarnessDeadline::Fixed(ms)` | 明示 override。早く fail したい場合だけ使う |

CLI: `--harness-margin-ms`（既定 5000）で導出の余白を調整。
`--harness-timeout-ms`（`--timeout-ms` alias）を指定した場合だけ `Fixed` になり導出は無効。

> 導出モードは hang した candidate を公式 window + margin ぶんだけ生かします。
> 大量スイープでまずい場合は `--harness-timeout-ms` で明示的に絞れますが、
> **絞った分だけ合法な run が timeout になり得ます**。

回帰テスト: `harness_deadline_is_derived_from_the_cases_official_window`（`[60,60,60]` が60秒を超える）と
`a_flat_deadline_kills_a_legal_run_but_the_case_window_does_not`（同一入力で Fixed は kill / CaseWindow は成功）。

#### record.team の正規化

`Evaluator::stamp_identity` が identity（candidate / case / sha / team）を**親側の値で1箇所に
正規化**します。よって `evaluate_one` の戻り値を見て `--team` の引き渡しを判定することはできません —
それでは親が正しければ子が間違っていても通ってしまいます。正しい検査は worker 自身が返す JSON で、
それが `worker_reports_the_team_it_was_given`（`tests/canonical_evaluation.rs`）です。

### isolation

| モード | timeout 時 | 用途 |
|---|---|---|
| `Isolation::Process` | **worker プロセスを kill して reaping**。計算資源は残らない | batch の既定。`procon-eval` は自動検出 |
| `Isolation::Thread` | record を返すが**スレッドは止まらない** | 同一プロセス内の test-only candidate 限定 |

process isolation は worker バイナリに**その algorithm がコンパイル済みであることが前提**です。
テスト専用の candidate は worker に存在しないので、そのテストは thread isolation になります。

worker は成否によらず **stdout に `RunRecord` を 1 行**出す（出力が無い＝kill された）。

### failure isolation

各 (candidate, case) は独立した実行単位と wall deadline の下で走ります。
1 つの candidate が panic しても、hang しても、行動契約を破っても、
その run の記録になるだけでバッチ全体は失いません。timeout は上記の通り worker を
kill するので、後続 run の CPU/timing を汚染しません。

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
