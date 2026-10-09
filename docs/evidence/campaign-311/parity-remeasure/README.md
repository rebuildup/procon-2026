# parity 再計測 — エンジン一致修正 (PR #355) 後の main 候補 BEFORE/AFTER

- **source SHA**: `59a550248c6be57ef95fc6f68793bd5141f00cef`（`origin/342-parity` @ `59a5502`、PR #355）。runs.jsonl / aggregate.json の全 12 レコードに同一 SHA をスタンプ済み。
- **harness**: `procon-eval`（`crates/bench/src/bin/procon-eval.rs`）release profile、`jobs=1`（逐次）、`--kinds P,S,P,S,P`、`--offline --locked`、`CARGO_INCREMENTAL=0`。詳細は `environment.json`。
- **計測日**: 2026-10-09。実行前に `uptime` で負荷確認（load average 2.14、上限目安 ~8 未満）。

## 重要な前提（必読）

1. ** BEFORE 数値は MatchSim-provisional**。pre-parity の旧エンジンは公式ルールを粗く模倣したシミュレータで、**移動を 1 要素 = 1 セルの要素レートで実行**していた。post-parity エンジン（`e59c213` "engine: match official move timing, traffic, and response rules"）は公式 Q6 タイムライン（移動は `tick + terrain_step_cost(origin)` に反映、道路 traffic は全チーム滞留ステップ平均、Q6 whole-response rejection）に作り直されている。したがって BEFORE/AFTER の絶対値は**シミュレータ意味論が異なるため直接比較できない**。本ドキュメントの delta は「公式ルール下での実測スコアへの変化」を読むためのもの。
2. **post-parity 数値も内部 evaluator の結果**。official_rule_parity テストスイート（`e59c213`、551 行）と公式サーバ検証シナリオ（`docs/evidence/campaign-311/official-server-parity/` A–F）で一致を固定しているが、**official server 受理 = evaluator 正しさではない**。本マシンに公式サーバーバイナリはない（`coverage-park-remeasure` README 同様、live smoke は未実施）。
3. **全体優越は主張しない**。本計測は 4 ケースのみ。ケースごとの delta のみを報告する。

## ケースとサイズラベル（誤記レビュー指摘への対応）

| case | 生 JSON 検証済みマップ | 日数 | agents |
|---|---|---:|---:|
| `fixture:701` | **24×24**（`map.width=map.height=24` を直接確認） | 10（daySteps `[146,183,76,180,135,144,59,136,53,120]`） | 5 |
| `fixture:medium-16x16` | **16×16** | 1 | 4 |
| `fixture:large-32x32` | **32×32** | 1 | 8 |
| `fixture:small-8x8` | **8×8** | 1 | 3 |

- v2 generator のサイズクラスは **8 / 16 / 32 のみ**（`crates/bench/src/generator.rs`: `MapSize::MIN=8`、medium=16、`MapSize::MAX=32`）。**24×24 は生成できない**。コーパス中で唯一の 24×24 が `fixture:701` であり、本計測では「701 = 24×24」と正しくラベル付けした。過去の「24×24 とラベルしたが実体は 32×32」というレビュー指摘は、生 JSON の map 次元を直接確認することで解決済み。
- kinds は全候補・全ケースで `P,S,P,S,P` に統一（同一ケース軸内で候補間の kinds は常に一致。harness が同一ケース軸を全候補に共有する）。

## 候補（`CandidateRegistry::builtin()` @ 59a5502）

| # | candidate id | status |
|---|---|---|
| 1 | `coverage:budget=official:park=far` | 計測済み |
| 2 | `coverage:budget=official:park=none`（W/D/L の基準 = baseline） | 計測済み |
| 3 | `search:trials=16:seed=42:budget=official`（#341 記載の search variant。canonical 順は trials, seed, time_budget_ms, budget） | 計測済み |
| 4 | #344 lookahead variant（`search:...:park=far:lookahead=next`） | **本 SHA ではビルド不可 — 事実として記録、修正はしない** |

- lookahead 不可能の根拠: parity ブランチの `search` factory は `trials / seed / time_budget_ms / budget` のみ。`park` / `lookahead` パラメータは 342-parity の search 再構築で削除された。再現コマンドとエラー文は `lookahead-unbuildable.txt`（`error: bad candidate id ...: algorithm "search" does not take parameter "park"`）。BEFORE 値のみ origin/343 証跡から引用。
- `any` / `fuelaware` アルゴリズムも本ブランチの builtin() に存在しない（登録済み: wait, loop, multiloop, greedy, nearest, astar, coverage, search）。同様に事実として記録。

## AFTER 結果（59a5502、12 run すべて `success`）

応答時間の単位を明示する: **`response_time_us` はマイクロ秒 (µs)**。ランキング軸 4 は score タプルの `total_response_time_ms`（**ミリ秒 = floor(µs/1000)**）。全レコードで µs→ms の整合を確認済み（1715150µs→1715ms、112424→112、98936→98、8906→8、299360→299、16377→16、143→0）。100 倍単位混在は無し。

| case | candidate | V | D | B | response µs | response ms (軸4) |
|---|---|---:|---:|---:|---:|---:|
| 701 (24×24, 10d) | `coverage:...:park=none` | 12 | 26 | 26 | 98,936 | 98 |
| 701 | `coverage:...:park=far` | 17 | 40 | 41 | 112,424 | 112 |
| 701 | `search:trials=16:seed=42:budget=official` | 17 | 57 | 58 | 1,715,150 | 1,715 |
| medium-16x16 | park=none | 4 | 4 | 4 | 808 | 0 |
| medium-16x16 | park=far | 4 | 4 | 4 | 703 | 0 |
| medium-16x16 | search trials=16 | 4 | 4 | 4 | 8,906 | 8 |
| large-32x32 | park=none | 3 | 3 | 3 | 16,574 | 16 |
| large-32x32 | park=far | 3 | 3 | 3 | 16,377 | 16 |
| large-32x32 | search trials=16 | 3 | 3 | 3 | 299,360 | 299 |
| small-8x8 | park=none | 1 | 1 | 2 | 160 | 0 |
| small-8x8 | park=far | 1 | 1 | 2 | 143 | 0 |
| small-8x8 | search trials=16 | 1 | 1 | 3 | 1,355 | 1 |

## BEFORE（pre-parity 証跡引用 — 再実行はしていない）

| case | candidate | V | D | B | response µs | 出典 SHA |
|---|---|---:|---:|---:|---:|---|
| 701 | `coverage:...:park=none` | 20 | 194 | 377 | — | `18253a4`（coverage-park-remeasure、tree 同値 `8f4cc89`） |
| 701 | `coverage:...:park=far` | 20 | 198 | 358 | — | 同上 |
| 701 | `search:trials=16:seed=42:budget=official` | 20 | 195 | 386 | 2,583,127 | `a6dfe64`（origin/340 search-first。701 runs の kinds は P,S,P,S,P。profile 記載なし） |
| medium-16x16 | park=none / park=far | 4 | 4 | 9 | — | `18253a4` |
| medium-16x16 | search trials=16 | 4 | 4 | 11 | 14,713 | `a6dfe64` |
| large-32x32 | park=none / park=far | 4 | 4 | 14 | — | `18253a4` |
| large-32x32 | search trials=16 | 4 | 4 | 17 | 262,443 | `a6dfe64` |
| small-8x8 | park=none / park=far | 2 | 2 | 7 | — | `18253a4` |
| small-8x8 | search trials=16 | 2 | 2 | 7 | 1,847 | `a6dfe64` |
| 701 | #344 lookahead（trials=4 / 8, `park=far:lookahead=next`） | 20 | 198 | 367 / 370 | 4,050,552 / 5,752,618 | `081aa07`（origin/343 confirmation）。**AFTER 計測不能**（パラメータ削除） |

注: coverage BEFORE の `18253a4` は `a6dfe64` より後の pre-parity tree であり、701 park=none が `18253a4`=194 と `a6dfe64`=195 で 1 点違うのは既知（coverage-park-remeasure README が舊 normalizer の受理差分として切り分け済み）。本表は coverage ペアに `18253a4`（同じ kinds P,S,P,S,P・release）を主出典とし、search に `a6dfe64` を使う。

## BEFORE/AFTER headline（ケースごとの delta — 全体優越の主張ではない）

| case | candidate | V | D | B | 読み取り |
|---|---|---|---|---|---|
| 701 (24×24) | park=none | 20→12 | 194→26 | 377→26 | **大幅減（軸1 V も 20→12）** |
| 701 | park=far | 20→17 | 198→40 | 358→41 | **大幅減。ただし park=none に対する軸1/2/3 优势は維持** |
| 701 | search trials=16 | 20→17 | 195→57 | 386→58 | **大幅減。3候補中で最大値は維持** |
| medium-16x16 | 両 park | 4→4 | 4→4 | 9→4 | V/D 不変、B 9→4 |
| medium-16x16 | search | 4→4 | 4→4 | 11→4 | B 11→4、応答 14,713→8,906 µs |
| large-32x32 | 両 park | 4→3 | 4→3 | 14→3 | **軸1 V も 4→3** |
| large-32x32 | search | 4→3 | 4→3 | 17→3 | 同上、応答 262,443→299,360 µs |
| small-8x8 | 両 park | 2→1 | 2→1 | 7→2 | **軸1 V も 2→1** |
| small-8x8 | search | 2→1 | 2→1 | 7→3 | B のみ 3 を維持 |

- **701 で軸1（variety）崩壊 + D 崩壊 → タスクの STOP ルールに抵触したため、ケース追加はせず原因調査（下節）を実施して停止した。**
- 崩壊は 701 に限らず全ケースでスコアが低下（small-8x8 の B 7→2、large-32x32 の V 4→3）。つまり「solver の退化」ではなく「シミュレータ意味論の変化」が全ケースに効いている。

## W/D/L vs `park=none` baseline（公式辞書式順、同バッチ同一ケース軸）

**valid-only 集計**（両者とも score 出力のケースのみ比較。aggregate.json の wins/draws/losses と同一）:

| candidate | W | D | L |
|---|---:|---:|---:|
| `coverage:budget=official:park=far` | 1 | 3 | 0 |
| `search:trials=16:seed=42:budget=official` | 2 | 0 | 2 |

（park=far の勝ちは 701。search の勝ちは small-8x8（B 3>2）と 701、負けは medium-16x16 と large-32x32 の軸 4 応答時間（ms）。）

**all-run 集計**（invalid を負け、両者 invalid は引き分けとする定義）: **12/12 run が `success`・scored で invalid 0 のため、valid-only と同一**（park=far 1W/3D/0L、search 2W/0D/2L）。

## invalid counts by class（全 12 run）

| class | 合計 |
|---|---:|
| `illegal_action_count` | 0 |
| `fuel_short_count` | 0 |
| `rejected_response_count` | 0（59a5502 の新規計上軸。whole-response rejection = 0 件） |
| `timeout` | 0 |
| `day_budget_exceeded_days` | 0（全 run 空配列） |

## 原因調査（STOP ルール発動後の簡易調査）

**現象**: 701 で V 20→12〜17、D 194→26〜57、B 377→58。かつ 701 以外の全ケースでもスコア低下。

**仮説（証拠つき）: エンジン一致修正による「オフライン過大評価の除去」であり、solver の合法性・計画実行可能性の退化ではない。**

1. **移動タイミングの意味論変更が本体**。`e59c213` は MatchSim を公式 Q6 タイムラインに作り直した: 移動は action phase で予約され `tick + terrain_step_cost(origin)` に反映（Q10）。旧 sim は移動を要素レート（1 要素 = 1 セル）で実行していたため、地形コストの重い盤で移動量を大幅に過大評価していた。
2. **計画は今も公式実行可能**。`campaign_diagnose`（fixture:701 / park=far / P,S,P,S,P）day 0: `normalized_action_changes=0`（normalizer 不動点）、`illegal=0 / fuel_short=0 / rejected=0 / day_flags=[]`。agent 0 は 26 moves + 120 waits = daySteps 146 を全消費。つまり計画は day budget を使い切っており、切詰めによる機会損失ではない。agent 2 が 2 moves + 144 waits のように待ちで満たすのは計画構造（当日の巡回なし）の問題であり、予算未使用ではない。
3. **地形加重がスループットを落とす**。701 は山地中心の 24×24。day 0 で agent 0 が 146 要素中 26 移動しかできない = 1 移動あたり重み ~5.6。旧 sim なら同じ 146 要素で ~146 セル移動できていた。24 spots/日の取得可能数が ~6 倍規模で減り、V/D/B の崩壊と整合する。
4. **サイズ横断のパターンも整合**: small-8x8（平地）B 7→2、medium B 9→4、large V 4→3・B 14→3、701（山地・10 日）B 377→26。低下幅は地形コスト密度に追従し、エージェント数の単純な比例ではない。
5. **候補間の相対順位はむしろ整合的に変化**: 701 では before（park=far D 198 ≥ search 195）に対し after は search 57 > park=far 40 > none 26。MatchSim リプレイで retention する search が実 sim 意味論に適合し、解析的 tour の coverage が適合しない、という方向であり、parity 修正の期待効果と矛盾しない。
6. **sim の正しさの固定**: `e59c213` は公式ルール一致を `official_rule_parity` スイート（551 行）で固定し、`docs/evidence/campaign-311/official-server-parity/`（A–F シナリオ）が公式サーバとの micro 一致を持つ。ただし「公式受理 = evaluator 正しさ」ではないため、701 の post-parity スコアが公式サーバで再現されるかは live 検証の残課題。

**残るリスク**: coverage 系が要素空間で計画し切詰めに頼る構造のため、加重クロック下で計画が最適でない可能性（= parity 後に solver 側の適応が未了）。これは solver 改善の課題であって evaluator の不具合ではない。ただし本計測はそれを検証していない。

## 再現

```bash
cd <342-remeasure worktree>
CARGO_INCREMENTAL=0 cargo +stable build --offline --locked -p bench --bin procon-eval --release
PROCON_SOURCE_COMMIT_SHA=59a550248c6be57ef95fc6f68793bd5141f00cef \
./target/release/procon-eval \
  --cases 'small-8x8,medium-16x16,large-32x32' \
  --case-files 'crates/bench/tests/fixtures/arena-701.json' \
  --kinds 'P,S,P,S,P' \
  --candidates 'coverage:budget=official:park=none,coverage:budget=official:park=far,search:trials=16:seed=42:budget=official' \
  --baseline 'coverage:budget=official:park=none' \
  --output-dir docs/evidence/campaign-311/parity-remeasure --quiet
```

## ファイル

- `runs.jsonl` — 12 run の生レコード（全件 `source_commit_sha=59a550248c6be57ef95fc6f68793bd5141f00cef`）
- `aggregate.json` — ランキング・W/D/L・score summary
- `environment.json` — harness/profile/units/出典・注入コマンド
- `source-sha.txt` — 測定対象 SHA
- `lookahead-unbuildable.txt` — #344 lookahead 候補が本 SHA でビルド不可である証跡
- `README.md` — 本ドキュメント
