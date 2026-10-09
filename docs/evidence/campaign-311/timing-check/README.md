# 公式簡易サーバー移動タイミング一致実験 — Issue #342 triage（判定 A/B）

source branch: `342-timingcheck`（`origin/release-0-4-0` @ `6431b25` から切る。
parity 修正 PR #355 済みの tree）。

**結論（verdict）: (A) evaluator-correct。post-parity `MatchSim` の移動タイミングは
公式簡易サーバーと 7 ケース全て完全一致した。sim bug は検出されなかった。**
701 のスコア崩壊は「旧 MatchSim の移動過大評価の除去（公式 Q6 タイムラインへの一致）+
planner 側の加重クロック未適応」で説明され、エンジン追加修正は不要。

## 背景と問い

post-parity MatchSim（公式 Q6 タイムライン: Move は action phase で予約され
`tick + terrain_step_cost(origin)` の反映で着地・課金）で再計測した 701 は
`park=none 20/194/377 → 12/26/26` と崩壊した。二つの説明:

- **(A) evaluator-correct**: 旧 MatchSim が移動を要素レート（1 要素 = 1 セル）で
  実行し移動量を過大評価していた。崩壊は過大評価の除去 + planner 未適応。
- **(B) sim bug**: 新タイムラインは公式より移動を遅くしている。

本実験は公式配布簡易サーバーの day 境界 `GET /`（pos / fuel が観測可能）を正とし、
同一 wire plan を post-parity `MatchSim` に通して day1 開始時点の pos/fuel を突き合わせた。

## 環境

- macOS (Darwin arm64)。公式簡易サーバー `procon-server-darwin-arm64`
  （`docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip` 同梱、改変なし、
  sha256 `cd552d93980c6b2a503c859dedc0357666823451662381cfa886260f5a81e041`）
- MatchSim 側: `crates/bench/examples/timing_parity.rs`（診断用 example。
  `engine::from_wire` 経由 = evaluator と同一の変換経路。プローブと同一
  `config.json` + `plans.json` を入力に、day 境界ごとの pos/fuel を JSON Lines 出力）
- 盤: 12×8 平地（ケースごとに pos 0 を山地/道路に変更）。スポット pos 14
  （本実験では未使用）。巡回車 1 台（V7 は 2 台）。`daySteps=[16,16]`、
  `daySeconds=[6,6]`、`fuelLimits=20`（V7 のみ 4）
- engine のチーム構成バリデータは 3..=8 台を要求するため、example 側は
  3 台にダミー運搬車（遠方平地で全待ち、スポットにも道路にも接触しない）を追加して
  再現。サーバー側は実際の台数（1〜2 台）で走った。この差異は被検証エージェントの
  タイムラインに影響しない（待ち・採取なし・道路滞留なし）

## ケースと plan（重み付きステップ合計 = 16 を厳守、`revision: 1` で全件受理）

| ケース | day 0 plan（wire コード） | 検証点 |
|---|---|---|
| V1_single_move | `[[2,-14]]` | 単発平地移動: t0 予約 → t2 着地・課金 |
| V2_seven_moves | `[[2,2,2,2,2,2,2,-2]]` | 連続移動 7 本（平地 2 tick 刻み） |
| V3_boundary_move | `[[-14,2]]` | t14 予約、due t16 == 日末反映で着地するか |
| V4_mountain_control | `[[2,-13]]` | 山地出発: 3 tick・燃料 2 |
| V5_mountain_boundary | `[[-13,2]]` | 山地出発を t13 予約、due t16 |
| V6_road_cadence | `[[2,2,2,2,2,2,2,2,-1]]` | 道路(滑)出発 1 tick・燃料 2 + 平地 7 本 |
| V7_supply_refuel | `[[2,2,-12],[-16]]` | 燃料 4 の巡回車が 2 セル走って運搬車と同セル（Q22 同セル補給） |

## 結果（day1 開始時点 `GET /` vs `timing_parity` 予測 — 全件一致）

| ケース | サーバー実測 pos/fuel | MatchSim 予測 pos/fuel | 一致 |
|---|---|---|---|
| V1_single_move | pos=1, fuel=19 | pos=1, fuel=19 | **一致** |
| V2_seven_moves | pos=7, fuel=13 | pos=7, fuel=13 | **一致** |
| V3_boundary_move | pos=1, fuel=19 | pos=1, fuel=19 | **一致** |
| V4_mountain_control | pos=1, fuel=18 | pos=1, fuel=18 | **一致** |
| V5_mountain_boundary | pos=1, fuel=18 | pos=1, fuel=18 | **一致** |
| V6_road_cadence | pos=8, fuel=11 | pos=8, fuel=11 | **一致** |
| V7_supply_refuel | pos=2, fuel=4（補給で満タン） | pos=2, fuel=4 | **一致** |

（ダミー運搬車は pos=95 / fuel=4294967295（無制限）。サーバーは運搬車の fuel を
4 と返すが移動タイミングの検証に無関係。）

読み取れる公式一致点:

1. **平地移動 = 2 tick・燃料 1**（V1, V2）。16 ステップの日に平地連続移動は
   7 本着地（t0,2,4,6,8,10,12 予約 → 2,4,...,14 着地）。
2. **日末反映で due == dayLimit の移動は着地する**（V3: t14 予約 → t16 着地。
   V5 の山地版も同様）。MatchSim の `due_step > day_limit` のみ切詰めの実装は正しい。
3. **山地出発 = 3 tick・燃料 2**（V4, V5）。
4. **道路(滑)出発 = 1 tick・燃料 2**（V6）。8 本の移動が 16 ステップに収まり、
   燃料は出発セル課金（2+7×1=9 → 残 11）。
5. **燃料は日をまたいで持続**し、**同セル運搬車接触で満タン補給**（V7, Q22）。
6. MatchSim 側の `illegal / fuel_short / rejected` カウンタは全ケース 0
   （サーバーの `revision: 1` と一致）。

## 701 崩壊への含意（triage 判定）

- 移動タイミングの sim bug（説明 B）は**不成立**。公式サーバーは
  post-parity MatchSim と同一の移動スループットを返す。
- 旧 sim（要素レート）は同じ plan を最大 ~2 倍以上の移動として評価していた
  （平地 1 要素 = 1 セル vs 公式 2 tick = 1 セル）。701 が山地中心の盤である点、
  崩壊幅が地形コスト密度に追従する点（parity-remeasure README の観察）と整合。
- よって崩壊は (A)「公式実行可能性の下での実スコア」への補正 + planner の
  加重クロック未適応（plan 構造が当日の待ちで埋まる構造）であり、
  **次のワークアイテムは solver 側の再調整**（加重ステップ予算を前提にした
  巡回構築）。エンジン追加修正は本実験の範囲では不要。

## 未観測・限界

- 簡易サーバーの `GET /` は `pos/fuel/kind` のみで玉数・スコアを公開しないため、
  採取タイミング（Q6 反映フェーズの取得）は本実験では未検証（従来どおり Q6 を正本とする）。
- 観測点は day1 開始時点のみ（2 日構成）。tick 単位の中間状態は簡易サーバーでは観測不能で、
  day 境界の pos/fuel のみで判定した。
- 簡易サーバーは配布バイナリだが本選当日の運用差は保証されない（fuel probe と同じ留保）。

## 再現

```bash
cargo build -p bench --example timing_parity --offline --locked
python3 scripts/campaign/timing_probe.py \
    --server-binary <公式簡易サーバーバイナリ> \
    --sim-binary target/debug/examples/timing_parity \
    --output-dir <空のディレクトリ>
```

## ファイル

- `summary.json` — 全ケースの要約（バイナリ digest 付き）
- `cases/<case>/config.json` — プローブがサーバーに渡した設定（sim へも同一入力）
- `cases/<case>/plans.json` — wire plan（sim へも同一入力）
- `cases/<case>/result.json` — HTTP 往復全記録 + day 境界状態 + sim 予測
- `cases/<case>/server.log` — サーバー出力（ローカルパスは `<home>` / `<out>` に置換済み）
- `cases/<case>/sim_stdout.jsonl` — `timing_parity` の JSON Lines 出力
