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

canonical order の SoT は factory の schema だけで、`loop` なら
`iterations / initial / seed / lookahead` の順です（`lookahead` は #193 で
`seed` の**後**に追記されたので、既存 id の形はバイト単位で不変）。
並べ替えは validation の後に行われるので、未知キーは既に拒否済みで
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
loop                                  iterations / initial / seed / operators / lookahead / walk
multiloop                             grid / kmeans / seed / refine / walk
greedy
nearest
astar                                 policy
wait                                  （baseline — 全段0）
```

問題の例に挙がっている ID はすべて実行可能です:

```text
loop:iterations=32:initial=greedy
loop:iterations=128:initial=greedy
loop:iterations=64:initial=random:seed=42
loop:initial=sweep-row
loop:initial=sweep-col
loop:iterations=32:initial=greedy:operators=extended
loop:iterations=32:initial=greedy:lookahead=crossday
multiloop:grid=2x2
multiloop:kmeans=4:seed=42
greedy
nearest
astar:policy=step
```

`initial` の候補: `greedy`（既定）/ `identity` / `reverse` / `sweep-row` / `sweep-col` / `random`。
`reverse` は入力順の完全な反転（`identity` の逆）で、座標にも seed にも依存しない決定的ツアーです。
`initial=random` の時は `seed` が必須で、同じ seed は常に同じツアーを返します
（xorshift64* を使い、外部 RNG crate に依存しません）。

`operators` の候補: `classic`（既定）/ `extended`（#191）。
`classic` は #191 より前の移動集合そのままで、`operators` を省略した既存 ID の
挙動は変わりません。`extended` は古典 7 手の 3-opt と or-opt（長さ ≤3 の
セグメント移動）を追加します。パラメータは id の末尾に追記するため
既存 canonical ID の形は変わりません。

`lookahead` の候補: `none`（既定）/ `crossday`（Issue #193）。
adapter は毎日 live state から再解決するという day contract を保ったまま、
1 解決の計画日数だけを切り替えます。

| 値 | `SolveProblem.days` | 歩く leg |
|---|---|---|
| `none`（既定・省略） | `1`（当日のみ） | `plans[0]` — 従来どおり |
| `crossday` | 残り日数 = `num_days − day` | その計画の当日 leg（計画は当日を先頭に anchor するため index 0） |

multi-day の leg 分割は `solver-loop::distribute_plan` の round-robin
（day は `i % days`、day 内の agent は位置順 round-robin — Issue #210 で
`gcd(days, n_agents) > 1` の coupling が生む空 leg を解消）を使う dormant pathを
外部から起動するだけで、`score.rs`・evaluate / scoring semantics は変更しません。
1 日の case では `crossday` の残り日数が 1 になるので `none` と厳密に一致します
（`distribute_plan` は `days = 1` / 単一 agent の形状で旧挙動と byte-for-byte 同一）。

`walk` の候補（`loop` / `multiloop` 共通、Issue #215）: `greedy`（既定）/ `astar`。
solver が day-start に決めた **target list の歩き方** を選びます。
`greedy` は従来の `plan_walk` そのままで（step index で target を切り替える
`step % len` 貪欲歩行）、`walk` を省略した既存 ID はすべてこれに一致します。
`astar` は同じ target list を **リスト順に 1 target ずつ A\* で辿り**、
到着で次 target、全到着 or day 終了で待機します（`#207` / `#213` の振動を
構造的に排除）。片 leg の A\* が予算・到達不能で失敗した場合は
`step_towards` 単独歩行へ degrade し、それも進まなければ待機します。
パラメータは id の末尾に追記され、既存 canonical ID の形は変わりません。

`multiloop` は `solver-loop::MultiLoopComposer` を evaluator に接続した
領域分割候補（Issue #190）。**`grid` と `kmeans` のどちらか一方だけ**を指定します。

| key | 値 | 規則 |
|---|---|---|
| `grid` | `<rows>x<cols>`（例 `2x2`） | 各辺 1..=32（`MapSize::MAX`）、`rows*cols <= 64`。`grid` と `kmeans` は排他 |
| `kmeans` | `k`（例 `kmeans=4`、`k=4` ではない） | 1..=64。`grid` と `kmeans` は排他 |
| `seed` | u64 | `kmeans` が在る時 **必須**、`grid` の時 **拒否**（`loop` の `initial=random` と同じ規則） |
| `refine` | `true` / `false` | region 毎の 2-opt refinement。省略時の既定は composer 自身の既定どおり `true` |
| `walk` | `greedy` / `astar` | target list の歩き方（`loop` と同一、Issue #215）。省略時の既定は `greedy`（従来の `plan_walk`） |

offline の `initial=` と live CLI の `--loop-initial-strategy` の対応は
[`docs/development.md`](development.md)（Issue #170 の節）と
`crates/cli/src/initial_strategy.rs` が SoT。

`astar` の `policy` の候補: `step`（既定）/ `conservative` / `strict` —
`solver-astar` の `CostPolicy` に対応。`step` は地形/交通量の重み付き step cost
を最小化（燃料は hard constraint）、`conservative` は残燃料が減るほど step cost
を大きくする、`strict` は残燃料が 1 以下の移動を拒否する。

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
**`--cases` は fixture 専用**です。seed 固定の generated case は `--generated` で
明示的に加えます（[generated corpus](#generated-corpusseed-固定の再生成-case) を参照）。

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

固定 fixture はそれ自体が真値なので `seed` を持ちません。generated case は
`CaseSpec::generated(case_id, seed, params, setting)` を通っており（実装は
`crates/bench/src/generator.rs`）、`case_id` / `seed` / `generation_parameters` は
すべて `RunRecord` に保存されます。

## generated corpus（seed 固定の再生成 case）

固定 fixture 9 件だけではサンプルが狭く、少数 fixture に過学習した candidate や
1 枚の map の偶然に塗り替えられた淘汰が起き得ます（Issue #185）。generated
corpus は fixture を**置き換えず補完**します。原則:

- 同じ `generator_version` + `generation_parameters` + `seed` ⇒ **バイト単位で同一**の
  case が再生成される。
- 生成結果は公式 validator / `engine::from_wire` / engine の `MatchState::new` を
  **すべて通ってから corpus に入る**（通らなければ case generation failure）。
- candidate ごとに別の seed を使わない。比較する siblings は**同一 seed set** を使う。
- 採点経路・score semantics は変更しない（生成するのは case だけで、scoring は
  既存の canonical path のまま）。

### 使い方: `--generated`

```bash
# 9 fixture + 生成 case を 1 バッチで（generated は opt-in）
PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" \
  cargo run -q -p bench --bin procon-eval -- \
    --cases all \
    --generated 'open:seed=1,road-heavy:seed=7,mountain-heavy:seed=7:size=small' \
    --candidates 'loop:iterations=32:initial=greedy,wait' \
    --baseline wait \
    --output-dir target/eval

# 燃料ストレス軸（#204）: fuel=tight で Q14 下限まで絞った同一 seed set
PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" \
  cargo run -q -p bench --bin procon-eval -- \
    --cases all \
    --generated 'road-heavy:seed=1:fuel=tight,open:seed=1:fuel=tight' \
    --candidates 'loop:iterations=32:initial=greedy:walk=astar,wait' \
    --baseline wait \
    --output-dir target/eval-tight
```

構文:

```text
<spec>  := <item> ("," <item>)*
<item>  := <archetype> (":" <kv>)+
<kv>    := "seed=" <u64>              必須。explicit seed こそが sibling 比較を
                                       1 つの case 軸に乗せるもの
          | "size=" small|medium|large  既定は archetype の既定 size
          | "days=" 1..=3               既定は archetype の既定 day 数
          | "fuel=" comfortable|tight   既定 comfortable（#185 の v1 と同一意味）。
                                       tight は燃料ストレス軸（#204）
```

- `fuel=` は archetype / size / days とは独立な軸です（Issue #204）。
  `comfortable` は v1 の不変条件 `3 × daySteps[0] ≥ fuelLimits ≥ 2 × sum(daySteps)`
  をそのまま維持し、燃料は**一度も枯渇しません**（`fuel_short_count = 0` が構造的に保証）。
  `tight` は公式 envelope（Q&A Q14: `fuelLimits ∈ 1..=3 × daySteps[0]`、
  `docs/contest/api.md:199`）の下限 `fuelLimits = daySteps[0]` に固定し、
  worst-case burn `2 × sum(daySteps)` がタンクを超えるようにする — つまり
  **fuel-blind な候補が invalid になり得る**軸です（fuel-short は候補の敗北であり
  case の欠陥ではなく、`RunRecord: status=invalid` として記録されます）。
- **fuel mode は seed set evidence に必ず含まれます**（#204 の AC）:
  `fuel=tight` は `case_id` に `:fuel=tight` として現れ（既定 `comfortable` は
  セグメントを付けず id が byte 単位で既存と同一のまま）、
  `generation_parameters.fuel` として両モードとも `RunRecord` に行単位で保存されます。
  したがって (3) generated seed set を復元するだけで、その evidence が
  comfortable 用か tight 用かは常に判別できます。

- `--cases` は **fixture 専用**。`--cases all` は今も「bundled 9 件」の意味のままで、
  既存の evidence の意味が勝手に変わることはありません。
- 1 回のバッチが 1 つの `Corpus` を構築し、その corpus を全 candidate がそのまま
  使います（`TournamentRunner::run` は candidates × cases の直積）。つまり
  **sibling は自動的に同一 case / 同一 seed set** です。これは
  `one_selection_is_one_shared_case_set`（`procon-eval` のテスト）と
  `bundled_fixtures_and_generated_cases_evaluate_in_one_batch` で検証されています。
- 同じ spec を 2 回書くと `duplicate case_id` として corpus 構築時点で拒否されます。

### archetype 一覧

`crates/bench/src/generator.rs` のテストが各 archetype の materiality を固定 seed で
検証しています（名前だけの別称にならないようにするのが目的です）。

| archetype | 地形 / 配置 | spots | brands | 既定 size | 既定 days |
|---|---|---|---|---|---|
| `open` | ≥90 % plain、pond / road なし | 6–14 | 2–6 | medium | 1 |
| `mountain-heavy` | ≥50 % mountain + くり抜いた plain 十字路 | 4–10 | 2–4 | medium | 1 |
| `pond-obstacle` | ≥10 % の pond ブロブ（十字路は温存） | 6–12 | 2–6 | medium | **2** |
| `road-heavy` | ≥15 % の**連結した** road 格子（8 セルおきの行 / 列） | 8–16 | 2–6 | medium | **3** |
| `many-series` | plain + mountain 散布 | 8–16 | ≥6 | medium | **3** |
| `few-series` | mountain の壁で 2 分け | 6–12 | ちょうど 1 | medium | 1 |
| `sparse-spots` | 少ない spot を広く間隔を空けて配置 | 3–6 | 2–4 | small | 1 |
| `dense-spots` | 18–26 個の spot を密クラスタに | 18–26 | 2–6 | large | **2** |

`road-heavy` は「道路を使い倒すか避けるか」が選択になるよう、**本当に道路が通った
map** を生成します（road は燃料 2 / plain は 1、交通量は yesterday+today で再分類）。
既存の `road-heavy` fixture は 4/256 セルしか道路がありませんでした（#187 で
`road-heavy-v2` が新設され、旧 case は `medium` として legacy 保持）。generated 側は
最初から本当に道路が通った map を生成します。

### day structure は一次の軸（fixture が持たない欠落）

bundled fixture は **`road-heavy-v2`（3 日、#187）以外が単日**（`daySteps` /
`daySeconds` が 1 要素）で、単日側は `daily_variety_sum ≡ variety` になり第 2 段が
差分ゼロ、end-of-day refill・yesterday/today 交通量分類・per-day budget 判定が
評価されません。そこで:

- `generation_parameters.days` が day 数を明示的に持つ（`daySteps` / `daySeconds`
  は両方ともこの長さ、per-day 値は公式範囲 `W+H ..= (W+H)*4` 内）。
- 複数 archetype（`road-heavy` / `many-series` = 3 日、`pond-obstacle` /
  `dense-spots` = 2 日）を**既定で multi-day** にし、`:days=` で全 archetype を
  1..=3 日に指定できる。
- multi-day は **front-loaded**（`daySteps[0]` が長く、後半は `W+H` 下限）。
  燃料は `daySteps[0]` の 3 倍が上限（Q14）なのに**日をまたいで補給されない**
  ため、均一な `[B,B,B]` は候補者が燃料切れして全件 `invalid` になります。
  既定の `fuel=comfortable` では generator が
  `fuelLimits >= 2 × sum(daySteps)`（1 行動の最大燃料消費 2 × 総 step 数）
  を**生成時の不変条件**として検査し、違反は case generation failure にします。
  （`fuel=tight` はこの下限を意図的に外します — 下の fuel mode 節参照。）

### determinism と case id

```text
case_id = generated:v<generator_version>:<archetype>:seed=<seed>[:size=<class>][:days=<n>][:fuel=<mode>]
```

`:size=` / `:days=` は archetype の既定と異なるときだけ、`:fuel=` は既定
（`comfortable`）以外のときだけ付くので、既定なら
`generated:v2:road-heavy:seed=7` のように短く —
それでも (version, archetype, seed, size, days, fuel) は id から一意に復元できます。
`generation_parameters` は `{generator_version, archetype, seed, size, days, fuel}` を
JSON で保持し、`RunRecord` にそのまま保存されます。

generator の出力は `GENERATOR_VERSION` に束縛されています。**出力が変わる変更を
したら version を上げる**こと。v1 の (params, seed) を黙って書き換えると、
過去の evidence が別物として読み替えられてしまいます。

version 履歴:

- **v1** — #185 の初版 + #204 の fuel 軸。fuel 軸は既定 params の出力を変えない
  ため version は据え置き（`fuel` キーなしの v1 record は `comfortable` と解釈）。
  **v1 = 旧（誤）地形表の world**: `cells` の wire コードが公式表と入れ替わっていた。
- **v2** — #226。地形タブレットを公式表（0=平地/1=道路/2=山地/3=池）に修正し、
  authored 出力の全セルを意味保存で permute（`0→0, 1→2, 2→3, 3→1`）したため、
  同一 `(params, seed)` の raw bytes が変わる。terrain の意味は v1 と同一、
  コードのみ異なる。**v1 の既存 evidence は historical（旧地形表 world）のまま
  書き換えず、re-baseline は #206** で扱う。

### seed set をどう記録するか

screening / confirmation は結果を別条件のまま直接比較しません。比較可能な 2 つは
**次が全部一致しているときだけ**比較します:

1. `source_commit_sha`
2. fixture corpus（`--cases` の選択）
3. **generated seed set**（`--generated` の spec 文字列、または records の
   `case_id` + `seed` + `generation_parameters`。**fuel mode もここに入ります** —
   `case_id` の `:fuel=tight` セグメントと `generation_parameters.fuel`）
4. generator version（`generation_parameters.generator_version`）
5. scoring semantics（official 4 段組）
6. timeout policy（`--harness-timeout-ms` / `--harness-margin-ms`）

`runs.jsonl` は (3)(4) を行ごとに持っているので、seed set は evidence から
直接復元できます（fuel mode 列つき）:

```bash
jq -r 'select(.case_id | startswith("generated:"))
       | [.case_id, .seed, .generation_parameters.generator_version,
          .generation_parameters.fuel] | @tsv' \
  target/eval/runs.jsonl
```

推奨 funnel（Issue #185 の原則）:

| 段 | corpus | 目的 |
|---|---|---|
| smoke | bundled fixtures + generated 数 seed | まず壊れていないか |
| screening | archetype ごとに複数 seed | 多様な map で足切り |
| confirmation | 生存者のみ seed 数を増やす | 偶然の map 特性を排除 |
| regression | 負けた case / edge case を**固定 fixture に昇格** | 回帰を永続化 |

fuel 系 hypothesis は funnel 全段を **既定 `fuel=comfortable`** で回し、
**confirmation 段だけ `fuel=tight` の seed set** を追加で回します（Issue #204 の方針）。
どちらの seed set で取った evidence かは上記 (3) に必ず現れるので、
comfortable と tight の結果を読み違えることはできません。

wall-clock（第 4 段 `response_time`）のノイズだけで淘汰しないこと。
第 1〜3 段を主 evidence とし、第 4 段は同点時の追加 evidence です。

### failure semantics: case generation failure ≠ candidate failure

| | case generation failure | candidate failure |
|---|---|---|
| 型 | `bench::GenerationError`（`Err`） | `RunRecord` + `status = invalid/timeout/panic` |
| 発生時点 | corpus 構築時（**candidate 実行前**） | 評価中 |
| メッセージ | `case generation failure: archetype=… seed=…: <cause>` | `status_detail` |
| 記録 | **1 件も残らない**（`CaseSpec` が存在しない） | `runs.jsonl` に 1 行 |

不正な generator 出力（validator 拒否・day 数違反・燃料不足になる case・重複
`case_id` など）は corpus 構築で `Err` になり、評価は**そもそも始まりません**。
したがって「generator の欠陥が candidate の `invalid` に化ける」経路は構造的に
存在しません。検証:
`crates/bench/tests/generated_corpus.rs::a_generation_failure_and_a_candidate_failure_are_distinct_classes`、
`crates/bench/src/generator.rs` の `corpus_build_fails_fast_and_stays_a_generation_failure`。

加えて `baselines_complete_every_archetype_without_fuel_short_or_illegal_actions` が、
全 archetype × 複数 seed で baseline（`wait` / `loop:iterations=32:initial=greedy`）が
燃料切れ・非法 action なしで完走することを確認し、「生成 case が invalid なのは
generator の bug」という失敗クラスをテストで塞いでいます。

### 回帰価値の高い generated case を固定 fixture に昇格する（promotion）

screening / regression で「この case で負けた」価値が高い場合、その case を固定
fixture にして回帰を永続化します。

1. **元の case を特定する** — evidence の `runs.jsonl` から
   `case_id` / `seed` / `generation_parameters` を拾う（この 3 つで再生成可能）。
2. **同じ入力で再生成する** — `bench::generator::generate(&params)`（`params` は
   record の `generation_parameters` を `GenerationParams` に deserialize）。
   再生成結果が evidence とバイト一致しないなら、generator version が evidence
   と違います。そこで止まってください。
3. **fixture JSON として書き出す** — `crates/bench/fixtures/maps/<新しい名前>.json` に、
   fixture スキーマ（`name` / `description` / `category` ヘッダ + `MatchSetting`）
   で保存します。素の `MatchSetting` ではありません。
4. **新しい名前を付ける** — 生成時の id（`generated:v<N>:…`、現行は
   `generated:v2:…` / 旧 evidence は `generated:v1:…`）は**使いません**。
   昇格後は `fixture:<新しい名前>` という**別 id** になります。古い evidence は
   `generated:…` のまま履史として残り、新しい fixture の結果と読み違えられません。
5. **bundled 登録する** — `MatchFixtureLoader::bundled_fixtures()` に追加し、
   `category` を実態に合わせます。**これは `--cases all` の意味を 8 → 9 に変える**
   ので、専用 PR にして fixture corpus 全体の baseline evidence を作り直します。
6. **generator を触らない** — 昇格で generator の出力を書き換えてはいけません。
   各 `GENERATOR_VERSION` の (params, seed) はその version の過去 evidence を
   指しているからです（v1 = 旧地形表 world）。generator 自体を
   変える必要が出たときは `GENERATOR_VERSION` を上げ、新しい id 系として扱います。

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

generated case も同じです — 同一 `(generator_version, params, seed)` なら
**setting がバイト単位で一致**します（PRNG は in-crate の xorshift64\* で、
外部 RNG crate に依存しないため compiler / dependency の増減にも影響されません）。
これが seed set を evidence として保存できる根拠です。

検証: `crates/bench/tests/canonical_evaluation.rs`
（3 cases × 2 real solver variants の E2E、決定性、failure isolation を含む）、
`crates/bench/tests/generated_corpus.rs`
（bundled fixtures + generated の 1 バッチ E2E、provenance、failure class、
baseline 完走、multi-day）、
`crates/bench/src/generator.rs`（全 archetype の byte-equivalent 再生成と validator 通過）。

```bash
cargo test -p bench --locked
```

## 非公式 arena との二段構成（Issue #179）

この evaluator が canonical なのは **offline 評価**であり、
大量探索・parameter sweep は必ずここで行います。
上位に選抜した candidate のみを非公式練習場
（`https://procon37arena.online`）で外部検証するのが Issue #179 の役割です:

- 実行手順・安全規律・evidence schema → [`docs/contest/arena.md`](./contest/arena.md)
- arena で負けた同一盤面は `procon arena import-case` で `CaseSpec` に取り込み、
  `procon-eval --case-files` でこの evaluator 上に**同じ corpus として**載せて再実行できます。
- arena の結果は `target/arena/runs.jsonl`（`record_kind: "arena-run"`）に別 schema で
  記録され、この evaluator の `runs.jsonl`（score tuple）と合算・再集計されることはありません。

つまり: **順位も選抜も offline が正、arena は selected candidate の外部検証のみ。**
