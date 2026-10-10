# rarity=2 の固定反復比較（#417）

`fleet:rarity=2` は hard12 で **5勝0分7敗、V合計差0 / D合計差−4 / B合計差+8**。日別種類数Dが3盤面で減っているため、この条件では採用しない。勝敗は baseline `rarity=0` に対する公式辞書順。hard12ではV/D/B同点がなく、回答時間を含めても勝敗は変わらない。

## 比較条件

- candidate source: `d87c66d169087e917881fb4eb676a3b2cac8b06c`（#413 / PR #414）。証跡PRのbaseは `413`、target releaseは `release-0-5-1`。
- candidate: `fleet:budget=official:iters=20000:chains=4:seed=42:rarity=0` と `rarity=2`。同一実行ファイル内で比較。
- 車種: `S,P,P,P,P,P,P,P`（先頭1台だけ補給、残り巡回）。`--jobs 1`、時計による打ち切りなし。
- canonical `procon-eval` / `MatchSim`、入力は `docs/evidence/campaign-311/fleet-371/corpus-hard/` の12件。CLIの必須fixtureとして追加された `openapi-sample` はhard12集計から除外。
- source/binary SHA、入力hash、実行コマンドは `manifest.json`。計測機は共有macOS。時間をWindowsネイティブ本番の測定とは扱わない。

## Dが悪化した盤面

| case suffix | baseline V/D/B | rarity=2 V/D/B |
|---|---|---|
| hard-16-a4-d10-short-s9001 | 13 / 130 / 166 | 13 / 129 / 166 |
| hard-24-a4-d7-short-s9007 | 20 / 138 / 167 | 20 / 136 / 171 |
| hard-24-a6-d9-short-s9008 | 20 / 180 / 245 | 20 / 179 / 223 |

玉数の増加は、上位の評価軸であるDの低下を相殺しない。12盤面を等重みとした経験平均でも、第1軸同点・第2軸低下となる。競技盤面の真の分布における期待値の証明ではない。

`hard/runs.jsonl` の26 run（12盤面と追加fixture、各2候補）は全件 `scored=true`。illegal、fuel-short、rejected、timeout、日次回答期限違反は0。

## 実験の作用範囲

#413の説明は「初期経路だけを変更、探索スコアは不変」だが、評価したsourceではそうなっていない。`fleet_solver.rs` の `World::greedy` は、`initial`（816行）と `evaluate` の翌日種類数推定（673行）の両方から呼ばれる。`self.rarity` による順序変更（758行）はどちらにも作用し、翌日推定のDはSAの探索スコアへ加算される。

この比較が測るのは **初期経路と翌日推定を同時に変える効果**。初期経路だけの仮説を棄却したことにはならない。次の候補を作る場合は初期経路への適用を独立させ、同じ入力・seedで比較する。

PR #414の差分を実際のbase `release-0-5-1@32636db` と照合すると、変更はfleet実装・テスト・監査ノートの3ファイル。Windows運用ファイルはbase由来であり、このcandidateのdeltaではない。

## 検証範囲

この時点で完了しているのはhard12。既存holdout24とbundled fixtures9の計測は同じ固定sourceで進行中。既存holdoutは過去候補にも使われており、新規の未使用最終評価データとは呼ばない。

Arena、観測交通の実戦盤面、別seed、時計予算、Windows実機はこの証跡に含めない。本番設定は変更していない。
