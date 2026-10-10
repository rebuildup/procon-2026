# rarity=2 の固定反復比較（#417）

`fleet:rarity=2` は45盤面のV/D/B比較で **15勝13分17敗、V合計差0 / D合計差−4 / B合計差−4**。日別種類数Dが3盤面で減っており、この条件では採用しない。raw recordsと盤面別差分は同じディレクトリに保存した。

| 評価集合 | 盤面数 | V/D/B 勝・分・敗 | 回答時間込み 勝・分・敗 | ΔV / ΔD / ΔB |
|---|---:|---|---|---|
| hard | 12 | 5 / 0 / 7 | 5 / 0 / 7 | 0 / −4 / +8 |
| 既存holdout | 24 | 10 / 4 / 10 | 11 / 0 / 13 | 0 / 0 / −12 |
| bundled fixtures | 9 | 0 / 9 / 0 | 5 / 0 / 4 | 0 / 0 / 0 |
| 全体（重複除外） | 45 | 15 / 13 / 17 | 21 / 0 / 24 | 0 / −4 / −4 |

勝敗はbaseline `rarity=0`に対する辞書順。第4軸はcanonical evaluatorと同じく回答時間を整数ミリ秒に切り捨てて比較した。共有Macで一度だけ測った時間なので、V/D/B同点の13盤面を安定した優劣の証明には使わない。`official/` は実行時のstage名で、公式サンプルを含むbundled fixtures9件を指す。9件すべてが公式提供盤面という意味ではない。

## 比較条件

- candidate source: `d87c66d169087e917881fb4eb676a3b2cac8b06c`（#413 / PR #414）。証跡PRのbaseは `413`、target releaseは `release-0-5-1`。
- candidate: `fleet:budget=official:iters=20000:chains=4:seed=42:rarity=0` と `rarity=2`。同一実行ファイル内で比較。
- 車種: `S,P,P,P,P,P,P,P`（先頭1台だけ補給、残り巡回）。`--jobs 1`、時計による打ち切りなし。
- canonical `procon-eval` / `MatchSim`、入力は `docs/evidence/campaign-311/fleet-371/corpus-hard/` の12件、`corpus-holdout/` の24件、`crates/bench/fixtures/maps/` の9件。CLIの必須fixtureとしてhard/holdoutへ追加された `openapi-sample` は各集合の集計から除外し、bundled fixturesで一度だけ数える。
- source/binary SHA、入力hash、実行コマンドは `manifest.json`。計測機は共有macOS。時間をWindowsネイティブ本番の測定とは扱わない。

## Dが悪化した盤面

| case suffix | baseline V/D/B | rarity=2 V/D/B |
|---|---|---|
| hard-16-a4-d10-short-s9001 | 13 / 130 / 166 | 13 / 129 / 166 |
| hard-24-a4-d7-short-s9007 | 20 / 138 / 167 | 20 / 136 / 171 |
| hard-24-a6-d9-short-s9008 | 20 / 180 / 245 | 20 / 179 / 223 |

玉数の増加は、上位の評価軸であるDの低下を相殺しない。45盤面を等重みとした経験平均でも、第1軸同点・第2軸低下となる。競技盤面の真の分布における期待値の証明ではない。

raw recordsはhard26、holdout50、fixtures18の計94 run（重複を除くと90 run）。全件 `scored=true`、illegal、fuel-short、rejected、timeout、日次回答期限違反は0。1日あたり実測最大は4.337秒。入力の日次期限はmanifestの入力hashに紐づく値で評価されており、60秒へ一律に置き換えていない。

holdoutではDの変化がなく、Bで10勝10敗4分。最大のB低下は `hold-32-a7-d10-mixed-s5017` の−10。hardの最大低下は `hard-16-a5-d10-short-s9004` の−24。全45盤面の差分は `paired-summary.json`。

## 実験の作用範囲

#413の説明は「初期経路だけを変更、探索スコアは不変」だが、評価したsourceではそうなっていない。`fleet_solver.rs` の `World::greedy` は、`initial`（816行）と `evaluate` の翌日種類数推定（673行）の両方から呼ばれる。`self.rarity` による順序変更（758行）はどちらにも作用し、翌日推定のDはSAの探索スコアへ加算される。

この比較が測るのは **初期経路と翌日推定を同時に変える効果**。初期経路だけの仮説を棄却したことにはならない。次の候補を作る場合は初期経路への適用を独立させ、同じ入力・seedで比較する。

PR #414の差分を実際のbase `release-0-5-1@32636db` と照合すると、変更はfleet実装・テスト・監査ノートの3ファイル。Windows運用ファイルはbase由来であり、このcandidateのdeltaではない。

## 検証範囲

hard12、既存holdout24、bundled fixtures9の計測を同じ固定sourceで完了した。既存holdoutは過去候補にも使われており、新規の未使用最終評価データとは呼ばない。

Arena、観測交通の実戦盤面、別seed、時計予算、Windows実機はこの証跡に含めない。本番設定は変更していない。

## 再現と再集計

評価sourceを新しい隔離worktreeへcheckoutし、`manifest.json` のbuild commandで `procon-eval` をビルドする。そのsource checkoutを作業ディレクトリにして、この証跡の `reproduce.py` を実行する。スクリプトはsource SHAを検査し、既存の出力があれば停止する。出力先はsource checkout内の `target/rarity-413-evaluation/`。各stageはmanifestに列挙したコマンドと同一条件で実行する。

保存済みデータは `python3 docs/evidence/rarity-417/summarize.py` で再集計できる。スクリプトはsource SHA、45件の一意性、対になるcandidateと車種、scored/違反数を検査し、`paired-summary.json` を生成する。原データのSHA-256はmanifestに固定した。
