# fleet candidate — 評価証跡（Issue #371 / PR #372）

`fleet` を、現行の live 候補（`anytime` / `coverage`、kinds `P,S`）と同じ盤面で比べた記録。計測バイナリの source は `76e7684`（`runs/*/runs.jsonl` の `source_commit_sha`）。

## 盤面

| セット | 盤面数 | 内容 |
|---|---:|---|
| `corpus/` ＋ 701 | 25 | 調整に使った 24 盤面（16/24/32、3〜8 台、4〜10 日、短日/混在、燃料 1.0〜3.0 倍）＋ 実戦盤面 701 |
| `corpus-holdout/` | 24 | 同じ分布・別 seed。実装と調整が終わってから一度だけ流した |
| `corpus-hard/` | 12 | 全日が下限付近（`W+H`〜`1.15×(W+H)`）、燃料 1.0 倍、7〜10 日 |

盤面は `gen_corpus.py <出力先> tune|holdout|hard` で再生成できる（seed 固定。`tune` の出力は `corpus/` とバイト一致）。地形はランダム配置を多数決平滑化した合成盤面で、実戦盤面そのものではない。

## 結果

`procon-eval --jobs 8`。比較は V → D → B の辞書順。並列実行なので応答時間はここでは比べない。

| セット | 相手（kinds `P,S`） | `fleet` 補給 1 台 の W/D/L | V | D | B |
|---|---|---|---:|---:|---:|
| corpus＋701 | `anytime:trials=64:seed=42` | 25/0/0 | 500 vs 500 | 3880 vs 3843 | 9090 vs 6078 |
| corpus＋701 | `coverage:park=none` | 25/0/0 | 500 vs 500 | 3880 vs 3751 | 9090 vs 5716 |
| holdout | `anytime:trials=64:seed=42` | 24/0/0 | 480 vs 480 | 3451 vs 3366 | 8253 vs 5397 |
| holdout | `coverage:park=none` | 24/0/0 | 480 vs 480 | 3451 vs 3216 | 8253 vs 4913 |
| hard | `anytime:trials=64:seed=42` | 12/0/0 | 240 vs 239 | 1833 vs 1696 | 2722 vs 1870 |
| hard | `coverage:park=none` | 12/0/0 | 240 vs 237 | 1833 vs 1537 | 2722 vs 1721 |

- `fleet` は `fleet:budget=official:iters=60000`、kinds `S,P,P,P,P,P,P,P`（補給 1 台）。
- D の上限（系列数×日数）は corpus＋701 で 3882、holdout で 3452、hard で 1850。hard の取りこぼし 17 は 3〜4 台盤面（巡回車 2〜3 台）に集中している。
- 全 61 盤面・全日で illegal / fuel_short / rejected は 0。`coverage` への fallback も 0。
- 同じ kinds 同士（`P,S`）でも `fleet` が `coverage` に 24 勝 1 敗（D 3859 vs 3751、B 6281 vs 5716、初版 `03a8541`）。伸びの大半は巡回車を増やせることから来る。

再集計: `python3 compare.py runs/<set>-baseline-PS/runs.jsonl anytime runs/<set>-fleet-1S/runs.jsonl fleet`

## live 設定での逐次計測

`runs/sequential-live-2s/`: `fleet:budget=official:budget_ms=2000`、`--jobs 1`。1 日あたりの応答は 2.00〜2.01 秒で、予算どおりに止まる。

## 公式簡易サーバーでの受理

`procon-server-linux-amd64`（sha256 `769e101c…`）に実 CLI で提出。回答時間は 4 秒に短縮。

| 盤面 | kinds | 結果 |
|---|---|---|
| 701（10 日） | `S,P,P,P,P,P,P,P` | 全日受理、revisions 1..10。サーバーが返す燃料で給油を確認 |
| 701（10 日） | `--kinds-auto` → `P,P,P,S,P` | 全日受理 |
| real-16-a4-d4-short-s1002（4 日） | `--kinds-auto` → `P,P,P,P` | 全日受理。最終日開始時の残燃料 4/34/40/7 |

## 既知の限界

- 盤面は合成。他チームの交通量（道路の混雑・渋滞）は offline では再現していない。live では毎日その日の道路状態で計画し直す。
- 補給 0 台の構成は、燃料が試合全体に足りる盤面でだけ `--kinds-auto` が選ぶ（V・D を落とさず B が 8% 以上増える場合のみ）。73 盤面中 5 盤面で選ばれ、うち 3 盤面で B が 18〜25% 増、2 盤面で 3% 減だった（しきい値導入前の計測）。
- 外部 Arena には未投入。
