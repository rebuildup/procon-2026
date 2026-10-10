# coverage park=far 再計測 — release-0-4-0 統合後

source: `18253a4f8ba80571927d06f47a23c51550ef644b`（計測実施時点。coverage 経路の tree は `8f4cc89` とバイト同一で、差分は lineage 収束 merge のみ）。旧数値は `docs/evidence/campaign-311/coverage-park/` に保持しており、本ディレクトリは上書きしない。

統合内容（PR #332 ↔ release-0-4-0）: #312/#313 診断ツール・`search` variant 試行、#317/#324 の公式 tick refuel 正規化（`engine::wire_plan::normalize_day_plan`）を取り込み、#331 の `park=far`（`day_claimed` spreading + map-size gate + `patrol_step` 抽出 + `2c5800d` の予算切詰め）と共存させた。予算/正規化は単一経路に統一: `decide()` 末尾の `BudgetMode::Official.apply` が `normalize_day_plan` を再実行するだけで、二重実装はない。`campaign_diagnose.rs` は release 側（312）版を採用。tour-planning（32x32、Issue #333）はこのブランチに含めない。

## 701 再計測（fixture:701、10 日、`--kinds P,S,P,S,P`、release profile）

| candidate | variety | daily_variety_sum | total_balls | daily series acquired |
|---|---:|---:|---:|---|
| `coverage:budget=official:park=none` | 20 | 194 | 377 | 20,20,18,20,20,20,19,20,17,20 |
| `coverage:budget=official:park=far` | 20 | **198** | 358 | 20,20,19,20,20,20,19,20,20,20 |

旧数値（`5cdcf49` で再現確認済み。証跡 `source-sha.txt` の `badc5df` スタンプは誤りで、park=far は `badc5df` ではビルドできない）:

| candidate | variety | daily_variety_sum | total_balls | daily series acquired |
|---|---:|---:|---:|---|
| `coverage:budget=official:park=none` | 20 | 195 | 379 | 20,20,18,20,20,20,19,20,18,20 |
| `coverage:budget=official:park=far` | 20 | 198 | 358 | 20,20,19,20,20,20,19,20,20,20 |

- **axis-2 差分は維持・むしろ拡大**: 旧 +3 → 新 **+4**（day 3/index 2 で 18→19、day 9/index 8 で 17→20）。
- park=far の 701 結果は旧数値と完全一致（20/198/358、daily も同一）。統合は park=far の 701 挙動を変えていない。
- park=none の −1（day 9/index 8: 18→17）の由来を切り分け済み:
  - 日 0–7 の軌跡（取得系列・agent moves・終端位置）は旧ツリー（`bb6721f`、旧 normalizer・切詰めなし）とバイト一致。日 8 のみ差分（patrol 1 台が 40 moves → 10 moves）。
  - 旧 normalizer は当該 day-8 計画を rewrite 0 件で受理していた（`normalized_action_changes=0`）。一方、#324 で公式バイナリ検証済みの新 normalizer（`docs/evidence/campaign-311/official-tick-317/`）＋切詰めは同計画の超過尾部を Wait に落とす。daySteps=53 の 5 台構成（旧 live match 1796 と同じクラス）。
  - つまり旧 195 は**提出不可計画を含むオフライン過大評価**で、提出可能な数値は 194。park=far の計画は新 normalizer でも rewrite 0 件で、198 はそのまま提出可能。

## 14 盤コーパス（bundled 9 + 701 + generated 4、同 28 run）

| candidate | W/D/L vs park=none | V mean | D mean | B mean |
|---|---|---:|---:|---:|
| park=far | 1W / 13D / 0L | 4.29 | **19.71** | 56.9 |
| park=none（baseline） | — | 4.29 | 19.43 | 58.3 |

- 全 28 run `success`、timeout / invalid / panic / illegal action / fuel short / day-budget exceeded すべて 0。
- 旧 2W/12D/0L からの差: many-series が同点化し 1W/13D/0L。V mean は不変、D mean は park=none 側が 701 の −1 で 19.50→19.43。
- **一般化しないこと**: 本 14 盤セットで 0L でも park=far が全局面で優位という意味ではない。Issue #333 の 32x32 大盤評価（`docs/evidence/campaign-311/tour-planning/evaluation.json`）では park=far に mountain-heavy での axis-1 V LOSS や more-agents での D 155→125 など既知の劣化カウンターエグザンプルがある。

## マップサイズ gate の確認（代表小盤＋gate 両方向）

| case | map | park=none (V/D/B) | park=far (V/D/B) | 結果 |
|---|---|---|---|---|
| fixture:small-8x8 | 8×8 | 2/2/7 | 2/2/7 | gate 閉（edge<24）で同一点数 |
| fixture:medium-16x16 | 16×16 | 4/4/9 | 4/4/9 | gate 閉で同一点数 |
| fixture:large-32x32 | 32×32 | 4/4/14 | 4/4/14 | 単日ケースで `tomorrow_steps=0` → day gate 閉 |

ユニットテスト `park_far_matches_park_none_on_small_maps`（小盤で byte-identical）と `park_far_spreads_on_maps_of_edge_24_or_more`（edge=24 で plan が分岐）が gate の両方向を固定している。

## live 互換（提出が通るか）

- 公式バイナリ smoke（`scripts/campaign/official_cli_smoke.py`）は**未実行**: 本マシンに公式サーバーバイナリ（sha256 `cd552d93…`）が配置されていない（前セッションの `/tmp` が消滅）。バイナリ配置後に `--candidate 'coverage:budget=official:park=far'` での再実行が残タスク。
- 実施した代替検証:
  - `cargo test -p cli --lib campaign_candidate`（live bridge と offline の日次一致）**PASS**。
  - `cargo test -p cli --test campaign_live`（mock HTTP 4 endpoint で実 CLI が official 形式計画を受理）**PASS**。
  - `campaign_diagnose`（701 / P,S,P,S,P / 両 park）の `normalized_action_changes_total = 0` — `decide()` 出力は `normalize_day_plan` の不動点であり、`2c5800d` が治した `revision = -1`（予算超過拒否）の failure mode は発生しない。
  - `coverage_official_normalization_preserves_every_action` が正規化不動点を回帰固定。
- 未完: 外部 arena 未接続。スコア・順位検証ではない。

## 再現

```bash
CARGO_INCREMENTAL=0 cargo build --release -p bench --bin procon-eval --offline --locked
PROCON_SOURCE_COMMIT_SHA=$(git rev-parse HEAD) ./target/release/procon-eval \
  --cases all \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --generated 'open:seed=17:days=3,open:seed=29:days=3,many-series:seed=3:days=3,open:seed=1:days=3:fuel=tight' \
  --kinds 'P,S,P,S,P' \
  --candidates 'coverage:budget=official:park=none,coverage:budget=official:park=far' \
  --baseline 'coverage:budget=official:park=none' \
  --output-dir target/eval-331-remeasure
```

数値の一次データは `runs.jsonl` / `aggregate.json`、環境メタは `environment.json`、計測 SHA は `source-sha.txt`。
