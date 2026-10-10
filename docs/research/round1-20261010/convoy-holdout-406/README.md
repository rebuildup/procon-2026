# convoy（#405 / PR #407）の holdout 検証 — セッション procon-2026-4f による独立計測

対象は `origin/405` @ `3d30381`（各 run の `source_commit_sha`）。このブランチには `convoy` のコードは無く、計測結果だけを置く。

- 比較: `convoy:budget=official:budget_ms=20000:seed=42` 対 `fleet:budget=official:iters=60000:chains=4:seed=42`（`convoy` が内部で使う `fleet` と同じ設定・同じ seed）
- kinds: `S,P,P,P,P,P,P,P`（補給 1 台）
- 盤面: `docs/evidence/campaign-311/fleet-371/corpus-hard`（12）と `corpus-holdout`（24）。どちらも `convoy` の調整には使われていない
- `procon-eval --jobs 3`、同じ Mac で他セッションの計測が並走（load 25〜35）。回答時間は参考値

| セット | 勝 / 同 / 負（V→D→B 辞書順） | V | D（convoy 対 fleet） | B（convoy 対 fleet） | 違法・燃料不足・拒否 |
|---|---|---|---|---|---:|
| hard（12） | 9 / 1 / 2 | 同じ | 1838 対 1834 | 2915 対 2869 | 0 |
| holdout（24） | 17 / 3 / 4 | 同じ | 3452 対 3452 | 8929 対 8879 | 0 |

- D が上がった盤面は hard の 4 つ（s9003、s9007、s9008、s9010、各 +1）。D が下がった盤面は 36 盤面中 0。
- 負けの 6 盤面はすべて V・D 同点で B が 1〜6 少ない（hard: s9006 −2、s9011 −6。holdout: s5007 −1、s5004 −2、s5010 −1、s5020 −2）。
- `convoy` の 1 日の最大回答時間は 20.4〜21.0 秒。`budget_ms=20000` を最大 1.0 秒超える（高負荷下）。
- hard の s9002 と s9004 は初回 run で worker が外部から kill されたため（`hard.runs.jsonl` では `panic` と記録）、2 盤面だけ再実行した値（`hard-rerun.runs.jsonl`）を表に使っている。

再集計: `python3 compare.py <runs.jsonl>`。

未計測: 全車巡回・補給 2 台以上の kinds、他の seed、`--jobs 1` での回答時間。
