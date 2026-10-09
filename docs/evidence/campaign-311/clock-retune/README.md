# clock retune — 公式タイムクロックへの coverage 再チューニング (Issue #342)

- **source SHA**: `f11eee3d6110d899e8e22d6745f8bfc81a1eae29`（`342-retune` @ `f11eee3`、base `6431b25` = release-0-4-0 tip / parity マージ済み）。`runs.jsonl` / `aggregate.json` の全 8 レコードに同一 SHA をスタンプ済み。
- **harness**: `procon-eval`（`crates/bench/src/bin/procon-eval.rs`）release profile、`jobs=1`（逐次）、`--kinds P,S,P,S,P`、`--offline --locked`、`CARGO_INCREMENTAL=0`。詳細は `environment.json`。
- **計測日**: 2026-10-09。実行前に `uptime` で負荷確認（load average 3.39、上限目安 ~8 未満）。

## 重要な前提（必読）

1. **絶対値は MatchSim 内部値**。official server 受理 = evaluator 正しさではない（`parity-remeasure` README 同様、本マシンに公式サーバーバイナリは無く live smoke は未実施）。正確性は official_rule_parity テストスイート + `official-server-parity/` A–F の一致で固定。
2. **全体優越は主張しない**。4 ケースのみ。delta のみを報告する。
3. 応答時間の単位: **`response_time_us` はマイクロ秒 (µs)**、score タプルの `total_response_time_ms` はミリ秒（軸 4）。全レコードで µs→ms 整合を確認済み（135310µs→135ms、11676µs→11ms 等）。

## 背景: 2 段階原因分解（step 1 — 修正前に実施、両者とも実測）

同一ケース（`fixture:701` 24×24、kinds `P,S,P,S,P`、day0 `daySteps=146`）で、(A) evaluator だけの差分と (B) planner だけの差分を分離した。探索的プローブは両ツリーに一時 example を置いて実行し、**コミット前に削除済み**。

### A. Evaluator-only diff（同一 wire plan を旧・新 MatchSim で再生）

plan は **pre-parity planner（`d666079` ツリー）が 701 park=none day0 で出力し、旧 normalizer が `Σ weighted == daySteps` に Exactly 正規した wire 本体**（#226/#342 の official server プローブで受理規則が固定済み = officially-accepted）。これを element 展開して両エンジンの `MatchSim::step` に 1 ブロックずつ投入。

| day0 結果 | 旧 MatchSim（要素クロック） | 新 MatchSim（タイムクロック） |
|---|---|---|
| 最終 pos（patrol 3 台） | (22,5) / (2,23) / (0,6) | **同一** |
| 獲得 balls（18/15/19） | 52 | **同一（18/15/19）** |
| day0 獲得 series | 20/20 | **同一（1..=20）** |
| day 終端 fuel | 347 / 320 / 347 | 347 / **169** / 347 |

唯一の差分は patrol#2 の燃料（320 → 169）: 補給接触の**ティック合わせ**がクロック意味論で変わるため（pos・取得は完全一致）。**同一 plan のスコアは崩壊しない** — 崩壊の主因は evaluator 側ではない。（新エンジンが公式一致であること自体は parity ブランチの 7 シナリオ検証済み。）

### B. Planner-only diff（同一ケースで新旧 planner の出力比較）

新エンジン（タイムクロック）上で、旧 planner（`6431b25` の coverage park=none）と pre-parity planner の day0 出力を比較:

| planner（動作クロック） | day0 移動要素 / agent | 移動の weighted 合計 | idle（wire が切り捨てる余剰待ち） |
|---|---|---|---|
| pre-parity（要素クロック上で正常動作） | 121 / 120 / 99 / 103 / 103 | **146 / 146 / 146 / 146 / 146（予算を過不足なく消費）** | 0 |
| 旧 planner（新クロック上で崩壊） | 31 / 46 / 5 / 7 / 9 | 63 / 79 / 10 / 13 / 12 | **83 / 67 / 136 / 133 / 134** |
| 再チューン後（本修正） | 98 / 99 / 106 / 109 / 110 | 146 / 146 / 146 / 146 / 146 | 0 |

旧 planner の 3 つの欠陥（すべて `pos` が移動中も旧セルのまま再決定することに起因）:

1. **飛行中フィラー待ち**: 各未着要素が実課金の `Wait` を 1 つずつ吐く（wire 本体では待ち連長として課金）→ 平地移動の実効コスト 2 → 3+。
2. **二重予約**: 旧座標から同一方向の Move を再発行しキューが積まれる → 目標オーバーシュート。
3. **長距離足の飢餓**: `route.cost <= day_steps - used[]` の `used[]` が飛行中移動を二重計上し、残予算が想定より早く尽きる → 長い足を途中で放棄（701 day0: 1 patroller は移動 5 回で終端 (5,0) 等）。701 全体では D 26 / V 12 に崩壊。

**最小再現テスト**: `crates/bench/tests/coverage_candidate.rs` の `planner_decides_from_landing_cells_and_packs_the_tick_budget`（2 スポット 1 行、12 ステップ日）。base `6431b25`（旧 planner + 新クロック）では patrol が (5,0) に 1 ball で止まり移動 5 + 待ち 7 が interleaved（テストは失敗）、修正後は (6,0) で 2 ball・移動 6 連続 + 末尾待ちのみ（成功）。

## 修正内容（step 2）

`crates/bench/src/adapters/coverage_solver.rs` の `decide()` 内ループのみ（engine / 他 adapter は不 touch）。per-element ループを **shadow timeline** に置換:

- 各 agent は**自分が空くティック（着地ティック）でのみ**決定する。決定は着地後の `pos` / `fuel`（真理値）から。移動は出発地形の step コストだけ agent を占有し、`clock[i]`（= 消費 weighted 予算 = 次に空くティック）を進める。
- 着地反映は Q6 順（燃料 → 移動 → 取得 → 補給）で、**シミュレータ本体のプリミティブ**（`transitions::phase_acquire` / `refill_fuel_if_supply_present`）をそのまま呼ぶため、取得・1日1個キャップ・在庫・補給の台帳は公式実装と同一。
- 補給車はペア patrol の**着地セル**（`landing[i]`、飛行中の移動先）を追従する（旧 `dir.step(pos)` の意図を明示化）。
- 予算内に着地できない Move は発行しない（`legal_step` の `step_cost <= 残余` が `ReserveReject::Truncated` を不可能にする）。
- 各 agent の答えは**連続した移動列**として出力され、予算消化後は末尾のみ待ち → `Σ weighted == daySteps` に正規化され、`normalize_day_plan` が不動点（全計測で `normalized_action_changes == 0`）。
- 副作用（意図的・実測で有利）: series 主張（claim）集合が日単位で永続化された。旧 per-element ループはステップ毎に claim を捨てており、これが #331 が `park=far` で補償していた「series 収束」欠陥の温床だった。再チューン後は **両 park モードが同一の day-scoped claim 方針を共有**し、`park=far` は同じ計測最良方針の明示的エイリアス（701 で両モード完全同一スコア、`park_far_matches_park_none_under_the_spreading_window` テストで固定）。

## 再計測（step 3 — 本 SHA、P,S,P,S,P、全 8 run `success`）

| case | candidate | V | D | B | response µs | 軸4 ms | illegal / fuelshort / rejected |
|---|---|---:|---:|---:|---:|---:|---|
| 701 (24×24, 10d) | park=none | 20 | **188** | **429** | 135,310 | 135 | 0 / 0 / 0 |
| 701 | park=far | 20 | **188** | **429** | 134,311 | 134 | 0 / 0 / 0 |
| medium-16x16 | 両 park | 4 | 4 | **11** | 722 / 677 | 0 | 0 / 0 / 0 |
| large-32x32 | 両 park | 4 | 4 | **14** | 11,676 / 12,517 | 11 / 12 | 0 / 0 / 0 |
| small-8x8 | 両 park | 2 | 2 | 7 | 130 / 134 | 0 | 0 / 0 / 0 |

701 の 10 日別 D: `[20, 20, 18, 20, 20, 20, 16, 20, 14, 20]`（崩壊時は日により 0 を含む低止まり）。

### BEFORE/AFTER（同一ケース軸、kinds 統一 P,S,P,S,P）

| case | candidate | BEFORE post-parity (`59a5502`, parity-remeasure) | BEFORE pre-parity (`18253a4`, coverage-park-remeasure 引用) | AFTER 本 SHA |
|---|---|---|---|---|
| 701 | park=none | 12 / 26 / 26 | 20 / 194 / 377 | **20 / 188 / 429** |
| 701 | park=far | 17 / 40 / 41 | 20 / 198 / 358 | **20 / 188 / 429** |
| medium-16x16 | 両 park | 4 / 4 / 4 | 4 / 4 / 9 | **4 / 4 / 11** |
| large-32x32 | 両 park | 3 / 3 / 3 | 4 / 4 / 14 | **4 / 4 / 14** |
| small-8x8 | 両 park | 1 / 1 / 2 | 2 / 2 / 7 | **2 / 2 / 7** |

- **701 は崩壊から回復**: V 20（pre-parity と同値）、D 188（pre-parity 194 の −6）、B 429（pre-parity 377 を上回る）。目標「D が 26 を大きく上回る」は達成（188）。予測ではなく `procon-eval` の観測値。
- 小中規模フィクスチャは pre-parity 数値に一致または上回る（medium B 11 > 9、large 4/4/14 = pre-parity 同値、small 同値）。
- search 候補（`search:trials=16:seed=42:budget=official`）は本修正の対象外（別ワーカの担当）。parity-remeasure 時点の search 701 = 17/57/58 なので、coverage 2 モードは再チューン後、同一ケース軸で search を V/D/B すべて上回る（応答時間は coverage が ~135ms vs search ~1715ms）。

### W/D/L vs `park=none` baseline（同一バッチ、valid-only）

| candidate | W | D | L |
|---|---:|---:|---:|
| `coverage:budget=official:park=far` | 1 | 2 | 1 |

スコアの V/D/B は全ケース完全一致。唯一の 1 敗は large-32x32 の軸 4 応答時間（12,517µs vs 11,676µs → 12ms vs 11ms）で、負荷タイミングの揺らぎ域。**計画的な優劣は無し**（同一方針のエイリアス）。

## 残るギャップ（正直な申告）

- 701 D 188 は pre-parity 要素クロックの 194 に 6 未達。diff は day 6/8（16/14）に集中 — 崩壊時は同日 4/1 前後だったので大部分は回復しているが、短め day（59/53 ステップ）でのターゲット選択の詰めに余地がある。
- `park=far` がエイリアス化したため、#331 由来のパラメータ選択肢は実質 1 本。旧 `park=far` 特有の「意図的な分散」を別の制御（例: claim 強度）として復活させる価値は、701 で分散無しに D 188 が出ている以上、現時点で計測上の根拠が無い。
- 応答時間は coverage で ~135ms/day（701、release、jobs=1）。旧 planner は ~99ms — shadow ループは agent 毎に Dijkstra を再計算するため僅かに重い。公式の 1 日 60 秒制約（`daySeconds`）に対して余裕は十分。

## 再現コマンド

```
PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) \
./target/release/procon-eval \
  --cases small-8x8,medium-16x16,large-32x32 \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --kinds P,S,P,S,P \
  --candidates coverage:budget=official:park=none,coverage:budget=official:park=far \
  --baseline coverage:budget=official:park=none \
  --output-dir docs/evidence/campaign-311/clock-retune \
  --quiet
```
