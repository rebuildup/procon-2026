# 予行演習マップ（2026-10-10）での戦略・車種・探索予算の実測（Issue #398）

2026-10-10 12 時台に MacBook Air（Apple M5、10 コア、macOS 27.0.1）で計測した。
盤面は [`data/rehearsal/2026-10-10-105938.setting.json`](../../../data/rehearsal/2026-10-10-105938.setting.json)（SHA256 `0878fa9f…db107b`）。16×16、5 日、4 台、燃料 40、10 種類 16 スポット。上限は V=10、D=50。

## 結論

| 位置づけ | Candidate | Kinds | V | D | B | T（5 日合計） |
|---|---|---|---:|---:|---:|---:|
| この盤面の最高点 | `fleet:budget=official:iters=60000:chains=4:seed=3` | `S,P,P,P` | 10 | 49 | 102 | 2.0 s |
| 次点・未知盤面にも使える設定 | `fleet:budget=official:budget_ms=10000:adapt=off` | `--kinds-auto`（この盤面では `S,P,P,P` を選ぶ） | 10 | 49 | 94〜104（中央 100） | 50 s |
| 現行の本番設定 | `fleet:budget=official:budget_ms=10000` | `--kinds-auto` | 10 | 48〜49 | 86〜104（中央 100） | 3.9 s |

- 戦略は `fleet`、車種は補給 1 台（`S,P,P,P`）。どちらも他の候補と V か D で差がつき、反復しても入れ替わらない。
- 最高点の設定は簡易サーバー経由で 3 回、offline で 2 回、すべて 10/49/102 になった。ただし seed をこの盤面に合わせた結果で、他の盤面や他チームが動く試合で同じ点になる根拠はない（下の「seed への依存」）。
- 現行の本番設定は 1 日目に 1 種類を落とすことがあり、D が 48 になる。offline で 45 回中 9 回、簡易サーバーでは 2 回中 2 回。探索時間を 2 秒以上に固定した設定は offline 35 回、簡易サーバー 3 回のすべてで D=49 だった。
- D=50 は 259 回の offline 計測と 11 回のサーバー試合のどれにも出ていない。1 日目（32 ステップ）は最高でも 9 種類。到達不能の証明はしていない。Issue #382 が扱っている現象と同じ。

本番設定を `adapt=off` に切り替えるかどうかは、この記録を見て運用側が決める。この PR では `~/.procon-honsen` の設定を変えていない。

## 計測条件

- ソース: `54a0bb9a1af1252d6df83ec201574e18bf4023c0`（branch `398`）。offline の全 259 run の `source_commit_sha` がこの値。
- `procon-eval`: `cargo build --release --locked -p bench --bin procon-eval`（rustc 1.99.0）、sha256 `31bc2f1a…99483`。
- サーバー試合の `procon`: 本選用に配置済みの `v2-71a9c0f`（sha256 `9b626753…4a5ec`）。`54a0bb9` との差は `crates/cli/src/tournament.rs` とそのテストだけで、探索（`crates/bench`）は同一。
- 簡易サーバー: 運営配布の `procon-server-darwin-arm64`（sha256 `cd552d93…a5e041`）。
- offline は 1 run ずつ `--jobs 1` で順に実行した。`--cases small-8x8` は評価器の仕様で同時に走るが、集計したのは `fixture:rehearsal-20261010-105938` だけ。
- 同じ Mac で他のセッションがビルドと評価を回しており、1 分平均の負荷は 2〜57 の間で動いた。run ごとの値を `results.csv` の `load1` に残してある。時間で打ち切る設定はこの負荷の影響を受ける。
- 除外した run は 0（invalid / timeout / panic / rejected / fuel-short / day-budget 違反 / 車種不一致のいずれもなし）。

T は 5 日分の探索時間の合計。順位は V → D → B の順で決め、T は V/D/B が同じときだけ見ている。

## Stage A: 戦略

| Candidate | Kinds | V | D | B | T | 判定 |
|---|---|---:|---:|---:|---:|---|
| `fleet:budget=official:budget_ms=10000` | S,P,P,P | 10 | 48〜49 | 86〜104 | 3.84 s | 採用（45 run） |
| `fleet:budget=official:iters=60000:chains=4` | S,P,P,P | 10 | 49 | 100 | 1.6〜2.9 s | 採用（5 run 同値） |
| `fleet:budget=official:iters=20000:chains=4` | S,P,P,P | 10 | 48 | 98 | 0.55 s | D で劣る |
| `fleet:budget=official:iters=8000` | S,P,P,P | 10 | 48 | 96 | 0.06 s | D で劣る |
| `anytime:budget_ms=5000:seed=42:budget=official` | P,S,P,S | 10 | 42 | 57 | 1.70 s | 除外（D −7） |
| `anytime:budget_ms=5000:seed=42:budget=official` | S,P,P,P | 9 | 40 | 58 | 1.20 s | 除外（V −1） |
| `anytime:budget_ms=5000:seed=42:budget=official` | P,P,P,P | 10 | 25 | 35 | 0.37 s | 除外 |
| `coverage:budget=official` | S,P,P,P | 10 | 33 | 46 | 0.003 s | 除外（D −16） |
| `coverage:budget=official` | P,S,P,S | 9 | 38 | 54 | 0.004 s | 除外（V −1） |
| `coverage:budget=official` | P,P,P,P | 9 | 22 | 37 | 0.001 s | 除外 |

`anytime` と `coverage` はどの車種でも D が 42 以下で、`fleet` の最悪値 48 に届かない。

## Stage B: 車種

`fleet:budget=official:budget_ms=10000` で比べた。`iters` 系の候補でも順序は同じ（[`full/report.md`](full/report.md)）。

| Kinds | V | D | B | T | 判定 |
|---|---:|---:|---:|---:|---|
| S,P,P,P（補給 1 台、スロット 0） | 10 | 48〜49 | 86〜104 | 3.84 s | 採用 |
| P,S,P,P / P,P,S,P / P,P,P,S（補給 1 台、別スロット） | 10 | 49 | 91〜101 | 3.84 s | S,P,P,P と区別できない（各 3 run） |
| P,S,P,S（従来の交互） | 10 | 44 | 66 | 2.00 s | 除外（D −4 以上） |
| S,S,P,P（補給 2 台） | 10 | 43〜44 | 66〜68 | 2.00 s | 除外 |
| P,P,P,P（全車巡回） | 9 | 43 | 58 | 6.83 s | 除外（V −1） |

全車巡回は探索時間を 0.25〜10 秒のどこに置いても 9/43/58 で動かない（24 run）。燃料 40 で 5 日は補給なしでは足りない。

`--kinds-auto` はサーバー試合 3 回とも `S,P,P,P` を選んだ。事前試行の値は `S,P,P,P` が 10/48/96、`P,P,P,P` が 9/43/57 で、選択までに 0.1 秒かからず、1.5 秒の枠に収まる。固定の `--kinds S,P,P,P` と結果は変わらない。

## Stage C: 探索時間

`fleet`、`S,P,P,P`、既定 seed。`adapt=on`（既定）は探索時間を (巡回車数 × スポット数)² / 3 ミリ秒で頭打ちにするので、この盤面では `budget_ms` を 1000 以上にしても 768 ms で止まる。それより長く探索させるには `adapt=off` が要る。表は実際に使った探索時間でまとめた。

| 1 日の探索時間 | 該当する設定 | run | D=49 の回数 | うち負荷 10 未満 | うち負荷 10 以上 | B 最小 / 中央 / 最大 | T |
|---:|---|---:|---:|---:|---:|---|---:|
| 250 ms | `budget_ms=250` | 16 | 15 | 6/6 | 9/10 | 89 / 97.5 / 103 | 1.25 s |
| 500 ms | `budget_ms=500` | 16 | 13 | 5/6 | 8/10 | 92 / 98 / 103 | 2.50 s |
| 768 ms | `budget_ms=1000` 以上で `adapt=on`（現行の本番設定を含む） | 45 | 36 | 16/17 | 20/28 | 86 / 100 / 104 | 3.84 s |
| 1000 ms | `budget_ms=1000:adapt=off` | 33 | 29 | 13/14 | 16/19 | 84 / 100 / 106 | 5.00 s |
| 2000 ms | `budget_ms=2000:adapt=off` | 13 | 13 | 11/11 | 2/2 | 93 / 100 / 102 | 10.0 s |
| 5000 ms | `budget_ms=5000:adapt=off` | 11 | 11 | 5/5 | 6/6 | 94 / 100 / 100 | 25.0 s |
| 10000 ms | `budget_ms=10000:adapt=off` | 11 | 11 | 4/4 | 7/7 | 95 / 100 / 104 | 50.0 s |

V は 145 run すべて 10。

- D=48 になった run は 1 日目が 8 種類で、代わりに B が 97〜106 と高め（17 run の中央 102）。1 日目に 9 種類を取る計画と、8 種類で玉を多く取る計画のどちらを選ぶかが揺れている。
- 1 秒以下では 110 run 中 17 回 D=48、2 秒以上では 35 run 中 0 回。この差が偶然で出る確率は 1% 未満。負荷が高いほど 1 秒以下の設定は崩れやすい（負荷 10 以上で 67 run 中 14 回）。
- B の中央値はどの探索時間でも 97.5〜100 で、探索を伸ばしても玉数は増えない。
- V/D/B の最大値（10/49、B 中央 100）を保てた最短の探索時間は 2000 ms。ただし高負荷での計測は 2 run しかない。5000 ms と 10000 ms は高負荷でも 13 run すべて D=49。

本番設定（768 ms）と固定時間を 1 run ずつ交互に回した A/B は [`paired/report.md`](paired/report.md)。高負荷の時間帯に重なり、本番設定は 20 run 中 5 回 D=48、`budget_ms=1000:adapt=off` は 20 run 中 4 回、5000 ms と 10000 ms は各 8 run すべて D=49 だった。

## seed への依存

`iters` 指定の候補は時計を見ないので、同じ seed なら何度回しても同じ点になる（seed 42 を 5 回、ほか 5 seed を 2 回ずつ確認）。点を動かすのは seed だけ。

| `iters`（`chains=4`） | D=49 になった seed（8 個中） | B の範囲 | T 中央 |
|---:|---:|---|---:|
| 20000 | 1 | 96〜103 | 0.59 s |
| 60000 | 5 | 94〜103 | 1.72 s |
| 120000 | 5 | 94〜103 | 3.93 s |
| 240000 | 5 | 94〜103 | 7.39 s |

seed は 1、2、3、7、11、12、42、2026。`iters=60000` で D=49 かつ B が最大なのは seed 3（102）、次が seed 42（100）。反復数を 4 倍にしても D=49 になる seed の数は増えないので、seed 3 の 102 は探索の質ではなく当たり seed とみるのが妥当で、別の盤面に持ち越せる性質ではない。

`iters` には時間の上限がない。この盤面では 1 日 0.24〜0.53 秒（高負荷時の最大 1.7 秒）だったが、24×24 や 32×32 での所要時間は測っていない。

## Stage D: 簡易サーバーでの受理

[`ops/honsen/e2e.py`](../../../ops/honsen/e2e.py) で簡易サーバーを起動し、10 チームを登録して自チームだけが回答した。受理は `POST /` に `revision >= 0` が返った日だけを数えている。V/D/B は coordinator が出す公式リプレイの値（`server/*.plan.log`）で、サーバーが翌日に返した位置と燃料は全試合・全日で予測と一致した。

| Candidate | Kinds | 1 日 | 受理 | V | D | B | 回答時間の合計 | 締切までの余裕（最小） |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `fleet:budget=official:iters=60000:chains=4:seed=3` | `S,P,P,P` | 60 s | 5/5 | 10 | 49 | 102 | 2.04 s | 58.8 s |
| 同上（2 回） | `S,P,P,P` | 10 s | 5/5 | 10 | 49 | 102 | 3.4 s | 7.6 s |
| `fleet:budget=official:iters=60000:chains=4` | `S,P,P,P` | 60 s | 5/5 | 10 | 49 | 96 | 1.98 s | 58.8 s |
| 同上（2 回） | `S,P,P,P` | 10 s | 5/5 | 10 | 49 | 96 | 3.3 s | 7.6 s |
| `fleet:budget=official:budget_ms=10000:adapt=off` | `--kinds-auto` → S,P,P,P | 60 s | 5/5 | 10 | 49 | 100 | 50.0 s | 49.5 s |
| `fleet:budget=official:budget_ms=10000:adapt=off` | `S,P,P,P` | 60 s | 5/5 | 10 | 49 | 94 | 50.1 s | 49.5 s |
| `fleet:budget=official:budget_ms=5000:adapt=off` | `S,P,P,P` | 60 s | 5/5 | 10 | 49 | 100 | 25.0 s | 54.6 s |
| `fleet:budget=official:budget_ms=10000`（現行） | `--kinds-auto` → S,P,P,P | 60 s | 5/5 | 10 | 48 | 103 | 3.86 s | 58.3 s |
| 同上（2 回目） | `--kinds-auto` → S,P,P,P | 60 s | 5/5 | 10 | 48 | 102 | 3.86 s | 58.4 s |

60 秒の試合は 3〜4 本を同時に走らせた。全試合で未回答の日は 0、終了コードは 0。

seed 42 の `iters=60000` は offline では 10/49/100、サーバー経由では 3 回とも 10/49/96 だった。どちらも再現するので乱れではなく入力の差だが、原因は特定していない（サーバーの状態には他 9 チームの車両が載る）。seed 3 は両方で 10/49/102。offline の B はサーバー経由の値と数玉ずれることがある、という前提で読む。

## 本番で使うコマンド

Windows（PowerShell）。`procon.exe` は `cargo build --release --locked -p cli --bin procon` で `target\release\` にできる。

```powershell
$env:PROCON_TOKEN = "<配布されたトークン>"
$env:PROCON_RESUME_RUNNING_MATCH = "1"

# 未知の盤面（本番の既定候補）
.\procon.exe coordinator --server http://<サーバー>:<ポート> --exit-after-match `
  --candidate "fleet:budget=official:budget_ms=10000:adapt=off" --kinds-auto

# 予行演習と同じ盤面だと分かっている場合だけ
.\procon.exe coordinator --server http://<サーバー>:<ポート> --exit-after-match `
  --candidate "fleet:budget=official:iters=60000:chains=4:seed=3" --kinds S,P,P,P
```

`release-0-5-0` の起動スクリプト [`ops/honsen/honsen.ps1`](../../../ops/honsen/honsen.ps1)（記録と再起動つき、手順は `docs/ops/honsen-windows.md`）を使う場合は、既定の candidate を引数で差し替える。`--kinds-auto` は既定のまま。

```powershell
powershell -ExecutionPolicy Bypass -File .\honsen.ps1 start -Server http://<IP>:<ポート> `
  -Candidate "fleet:budget=official:budget_ms=10000:adapt=off"
```

車種の使い分け:

- 盤面を事前に知らない試合は `--kinds-auto`。補給 1 台と全車巡回を試合開始時に試して選ぶ。この盤面では固定の `S,P,P,P` と同じ結果になる。
- `--kinds S,P,P,P` の固定は、盤面が予行演習と同じだと確認できたときだけ使う。

フォールバック（上から順に）:

1. `fleet:budget=official:budget_ms=10000` + `--kinds-auto`。現行の本番設定で、Arena と #385 の障害試験を通っている。この盤面では D が 1 落ちることがある。
2. `anytime:budget_ms=5000:seed=42:budget=official`、車種指定なし。この盤面では 10/42/57。

`adapt=off` から現行設定へ戻す条件: 公式サーバーで回答から締切までの余裕が 20 秒を切る、または 1 日でも未回答が出る。

## 確認できていないこと

- **Windows での実行**。ビルドも計測もこの Mac（arm64）だけ。上のコマンドは引数と環境変数が同じというだけで、Windows 実機での動作と所要時間は未確認。`iters` 候補は浮動小数点の実装差で点が変わる可能性があり、Windows で同じ 10/49/102 になるかは `py -3 scripts/rehearsal_20261010_eval.py --confirm --only det60k-seed3` で確かめられる。
- **10 チームが動く試合**。簡易サーバーの相手 9 チームは動かない。この盤面は渋滞のしきい値が 1 / 2 と低く、実戦では道路の所要時間が変わる。ここでの点はすべて単独走行の値。
- **本選サーバーでの受理**。確認したのは簡易サーバーだけ。
- **`adapt=off` の他盤面での効果**。D が安定するのを確かめたのはこの 1 盤面。24×24 以上では `adapt=on` でも探索時間が 2 秒以上になるので差は出にくいはずだが、測っていない。32×32 は `adapt=on` でも 10 秒を使い切るので、時間の余裕は #385 の記録（余裕 49 秒）と同じになる。
- **回答時間が同点決着に効く場面**。`adapt=off` は T が 3.9 秒から 50 秒に伸びる。V/D/B が上位と完全に並んだときだけ不利になる。
- **D=50 の可否**。出ていないが、不可能とは示していない。

## 再現

```bash
git checkout 398
cargo build --release --locked -p bench --bin procon-eval
python3 scripts/rehearsal_20261010_eval.py --full    --out target/rehearsal-20261010/full
python3 scripts/rehearsal_20261010_eval.py --confirm --out target/rehearsal-20261010/confirm
python3 scripts/rehearsal_20261010_eval.py --paired  --out target/rehearsal-20261010/paired

python3 ops/honsen/e2e.py --server-binary <procon-server> --cli <procon> \
  --case-file data/rehearsal/2026-10-10-105938.case.json \
  --candidate 'fleet:budget=official:budget_ms=10000:adapt=off' --kinds auto \
  --output-dir <新しいディレクトリ>
```

## ファイル

| パス | 内容 |
|---|---|
| [`full/`](full/) | Stage A〜C。戦略 × 車種 × 探索時間 × seed、118 run |
| [`confirm/`](confirm/) | 上位設定の反復と `iters` × seed、85 run |
| [`paired/`](paired/) | 本番設定と固定時間の交互 A/B、56 run |
| [`server/`](server/) | 簡易サーバー 11 試合の要約（`*.json`）と日ごとの計画ログ（`*.plan.log`） |

各ディレクトリの `report.md` が順位表、`results.csv` が run ごとの値、`runs.jsonl` が評価器の `RunRecord`（この盤面の分だけ）。
