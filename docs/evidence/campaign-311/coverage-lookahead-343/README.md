# search 翌日公式評価（Issue #343）

## 出典と仮説

- Parent Issue / Draft PR: #340 / #341
- Current Issue / Draft PR: #343 / #344（PR base は `340`）
- Parent source SHA: `0d913f2651729fbf270390e255ae664a92901fc6`
- Evaluated solver source SHA: `081aa070364489011328dbade65e94bb2d27c0e7`
- Day-trace harness SHA: `b0c0723437f169065ef082eb1dfd960466ebef35`
- Post-evaluation validation guard SHA: `e42b9d0be39470e1ad3c1a79844501a5c7ad9e83`
- Baseline: `coverage:budget=official:park=far`
- Agent kinds: `P,S,P,S,P`
- Ranking: 公式辞書順 `(variety, daily_variety_sum, total_balls, response_time_us lower)`
- Arena: 不使用。すべてcanonical offline `procon-eval`。

#340 の search は当日種類数とballsを保持基準にし、当日の終端位置を次日に評価していません。HTML戦略ラボから終端位置rolloutの考え方を取り入れ、候補当日をcase固有 `SimConfig` で再生後、その状態から翌日をbaseline coverage方針で1日だけ評価しました。採点は独自の重み付き値でなく公式スコアの先行3軸です。

## 結果

| candidate | 15ケース W/D/L vs baseline | variety 合計 | daily variety 合計 | balls 合計 | 平均 response | 平均 elapsed |
|---|---:|---:|---:|---:|---:|---:|
| `coverage:budget=official:park=far` | 0/15/0 | 73 | 305 | 740 | 39.6 ms | 42.8 ms |
| `search:trials=8:seed=42:budget=official:park=far` | 5/0/10 | 73 | 298 | 743 | 379.9 ms | 383.3 ms |
| `search:trials=4:seed=42:budget=official:park=far:lookahead=next` | 7/1/7 | 73 | 305 | 762 | 330.6 ms | 333.9 ms |
| `search:trials=8:seed=42:budget=official:park=far:lookahead=next` | 7/0/8 | 73 | 305 | 765 | 504.2 ms | 507.2 ms |

全60 run が `success`。invalid 0、timeout 0、panic 0、illegal action 0、fuel short 0、day-budget 超過0。confirmationの `elapsed_us` 合計は19.008秒。最大個別 response/elapsed は lookahead 4試行で4.051/4.058秒、8試行で5.753/5.758秒（fixture:701）。この負荷はcaseのday window内だが、同点の公式指標ではbaselineより遅くなる。

fixture:701 は次のとおり。

| candidate | variety / daily / balls | response / elapsed |
|---|---:|---:|
| baseline | 20 / 198 / 358 | 399.4 / 405.1 ms |
| search trials=8、lookaheadなし | 20 / 191 / 352 | 4.217 / 4.229 s |
| search trials=4、next | 20 / 198 / 367 | 4.051 / 4.058 s |
| search trials=8、next | 20 / 198 / 370 | 5.753 / 5.758 s |

lookahead候補は15ケースで上位3軸を落とさず、ballsを合計22/25増やしました。一方、応答時間を含む公式tupleのcase別W/D/Lは4試行で7/1/7、8試行で7/0/8で、全体としてbaselineを優越しません。候補2試行の小規模確認はfixture:701で `20/188/356`、baseline `20/198/358` となり、1 variationではday regressionを止められませんでした。したがって `lookahead=next` は実験候補に留め、採用・既定化しません。

10ケースscreen（9 bundled + fixture:701）でもlookahead 4/8はbaselineと同じ `44/228`、balls `469/472` vs `448` でした。記録は `screening/` にあります。15ケースconfirmationは9 bundled + fixture:701 + 5 generated cases（既存search比較と同じseed set）です。

## 原因の再現証拠

`diagnostics/day8-trace-343.log` は `b0c0723437f169065ef082eb1dfd960466ebef35` のignored integration traceです。0-based day 8（試合9日目）に、baselineは20種類、searchは18種類でした。同じsearch状態からbaseline方針をshadow replayすると15種類に留まり、searchは18種類を取得します。よって当日のbaseline solverがsearchより有効だったという説明ではなく、前日までのactionで作られた位置・燃料状態が根因です。

0-based day 7終了時、baselineのpatrol位置は `(23,19),(12,19),(15,9)`、searchは `(5,10),(13,11),(23,19)`。day 8終了時はそれぞれ `(5,10),(3,23),(21,13)` と `(12,19),(9,20),(6,0)` で、day 8当日の日別seriesは双方20のままでもday 9開始状態が異なります。親candidateのstate traceでは残stockをmaxへ戻したreplayでもスコアが一致しており、在庫残ではなく持続する位置・燃料差が次日の到達可能集合を変えています。追跡ログも同じ `diagnostics/` に保存しました。新しいtrace harnessはcase固有 `SimConfig` で同じscoreとdaily vectorを再現します。

再現:

```bash
cargo test --release --locked -p bench --test day8_trace -- --ignored --nocapture
```

## 再現コマンド

評価器はRelease build済み。すべて `--jobs 1` でresponse timeの比較を計測しました。

15-case confirmation（240秒deadline、60 records）:

```bash
timeout --kill-after=5s 240s env PROCON_SOURCE_COMMIT_SHA=081aa070364489011328dbade65e94bb2d27c0e7 \
  /workspace/procon-2026/target/release/procon-eval \
  --cases all \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --generated 'open:seed=31:days=3,road-heavy:seed=19,mountain-heavy:seed=23:size=small,many-series:seed=19,pond-obstacle:seed=11:fuel=tight' \
  --kinds 'P,S,P,S,P' \
  --candidates 'coverage:budget=official:park=far,search:trials=8:seed=42:budget=official:park=far,search:trials=4:seed=42:budget=official:park=far:lookahead=next,search:trials=8:seed=42:budget=official:park=far:lookahead=next' \
  --baseline 'coverage:budget=official:park=far' \
  --jobs 1 \
  --output-dir target/eval-search-lookahead-343-confirmation \
  --markdown-out target/eval-search-lookahead-343-confirmation/report.md \
  --quiet
```

Trial=2 low-trial comparison (90秒deadline、small-8x8 + fixture:701、8 records):

```bash
timeout --kill-after=5s 90s env PROCON_SOURCE_COMMIT_SHA=081aa070364489011328dbade65e94bb2d27c0e7 \
  /workspace/procon-2026/target/release/procon-eval \
  --cases small-8x8 \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --kinds 'P,S,P,S,P' \
  --candidates 'coverage:budget=official:park=far,search:trials=2:seed=42:budget=official:park=far:lookahead=next,search:trials=4:seed=42:budget=official:park=far:lookahead=next,search:trials=8:seed=42:budget=official:park=far:lookahead=next' \
  --baseline 'coverage:budget=official:park=far' \
  --jobs 1 \
  --output-dir target/eval-search-lookahead-343-lowtrial \
  --markdown-out target/eval-search-lookahead-343-lowtrial/report.md \
  --quiet
```

## 検証

- `cargo fmt --all -- --check`: pass
- `cargo test --release --locked -p bench`: pass（164 unit tests、CLI / integration / doc tests）
- `cargo test --release --locked -p engine`: pass（66 unit + 5 integration + 1 doc test）
- `cargo clippy --locked -p bench -p engine --all-targets -- -D warnings`: pass
- `cargo build --release --locked -p bench --bin procon-eval`: pass
- `procon-eval --list-candidates`: `lookahead=none|next` を確認
- ignored day8 trace: pass（試合全10日、18.04秒）

評価後のguard commit `e42b9d0` は `lookahead=next` と `time_budget_ms>0` の組み合わせをrejectします。翌日replayの追加処理にwall-clock時間予算を適用しない誤用を防ぐための候補設定validationです。上記評価candidateはすべて `time_budget_ms=0`（省略）なので、記録済みscore / timingには影響しません。guard commit後にbench test・clippy・Release evaluator buildを再実行しました。

10-case screen（180秒deadline、40 records）:

```bash
timeout --kill-after=5s 180s env PROCON_SOURCE_COMMIT_SHA=081aa070364489011328dbade65e94bb2d27c0e7 \
  /workspace/procon-2026/target/release/procon-eval \
  --cases all \
  --case-files crates/bench/tests/fixtures/arena-701.json \
  --kinds 'P,S,P,S,P' \
  --candidates 'coverage:budget=official:park=far,search:trials=8:seed=42:budget=official:park=far,search:trials=4:seed=42:budget=official:park=far:lookahead=next,search:trials=8:seed=42:budget=official:park=far:lookahead=next' \
  --baseline 'coverage:budget=official:park=far' \
  --jobs 1 \
  --output-dir target/eval-search-lookahead-343-screen \
  --markdown-out target/eval-search-lookahead-343-screen/report.md \
  --quiet
```
