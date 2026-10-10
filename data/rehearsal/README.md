# 2026-10-10 本選予行演習盤面（Issue #398）

## 入力と身元

- **生のサーバー設定**: [2026-10-10-105938.setting.json](2026-10-10-105938.setting.json)
- **評価器用 flat CaseSpec**: [2026-10-10-105938.case.json](2026-10-10-105938.case.json)
- 生ファイル SHA256: `0878fa9f3a23d3942151a73cc3647cc7f96800fdb94134d161a76a3a58db107b`
- 開始時刻: 2026-10-10 10:59:38 JST (`startsAt=1791597578`)
- 16×16 / 5日 / 4台 / 10種類 / 16スポット / 燃料40 / players=10
- `daySteps=[32,36,40,40,48]`, `daySeconds=[60,60,60,60,60]`, traffic thresholds 1/2
- 種類数 V の上限は10、日別種類数累積 D の上限は50（各日全種類取得できる場合）
- 生JSONとCaseSpecは同じ盤面値。CaseSpec は `case_id`/`source`/`seed`/`generation_parameters` だけ付加する。既存 `--cases all` の corpus は変更しない

## 検証コマンド（完全オフライン、外部APIアクセスなし）

```bash
git checkout 398
cargo build --release --locked -p bench --bin procon-eval

# 短い比較（6 run）。出力先 target/rehearsal-20261010
python3 scripts/rehearsal_20261010_eval.py

# Stage A〜C: 戦略 × 車種 × 探索予算 × seed（118 run、約12分）
python3 scripts/rehearsal_20261010_eval.py --full --out target/rehearsal-20261010/full

# Stage D: 上位設定の反復と seed 依存の確認（85 run）
python3 scripts/rehearsal_20261010_eval.py --confirm --out target/rehearsal-20261010/confirm

# Stage D: 本番設定と固定探索時間の交互 A/B（56 run）
python3 scripts/rehearsal_20261010_eval.py --paired --out target/rehearsal-20261010/paired
```

Windows PowerShell では `python3` を `py -3` に替え、`--bin target/release/procon-eval.exe` を付ける。

スクリプトは起動時に生 JSON の SHA256、CaseSpec との完全一致、盤面寸法・台数・日数・燃料・しきい値を検査し、合わなければ何も実行しない。
`procon match-config validate` は別形式（`days` と agent オブジェクトを持つローカル試合設定）用で、サーバーの `GET /setting` をそのまま保存した生ファイルには使えない。

`--only <文字列>` はラベルで絞り込み、`--resume` は `--out` に残っている計測済み run を再利用する。
成果物は `report.md`、`results.csv`、`results.json`、`summary.json` と、run ごとの `runs.jsonl`・stdout・stderr。
順位は反復中の**最悪** V/D/B で付ける。

注意: `procon-eval` の `--cases` は bundled fixture を最低1つ指定する仕様なので、スクリプトは `small-8x8` も起動する。**結果表では case_id=`fixture:rehearsal-20261010-105938` のみを評価する**。`--case-files` へ生の `*.setting.json` を直接指定しない。

## 判定（最優先は得点）

- 実測した4軸 `(variety, daily_variety_sum, total_balls, -response_time_us)` を **辞書順**で比較。応答速度でV/D/Bの不足を補正しない
- invalid/timeout/panic・拒否/違法行動/燃料不足・day budget超過があれば除外する
- `fleet` は `--kinds S,P,P,P`（Supply slot0）と `--kinds P,P,P,P` を比較。従来既定の `P,S,P,S` と区別
- `--kinds-auto` は coordinator 側にのみ存在する。スクリプトの判定は iters=8000 で比較した **近似**。coordinator の1.5秒締切を本番環境で確認する
- 時間依存 `budget_ms` は CPU・負荷と乱数の影響を受けるため `--full` の反復結果を確認し、V/Dが揺れるケースを「確定」と扱わない
- 探索時間は各日60秒に収め、車種構成の提出は別の30秒締切に収める。過去の予行演習は途中退場しているため、本来の競技スコアとして扱わない
- これは **単独チームの canonical offline replay**。10名の道路競合、公式HTTPによる5日受理、Windows本番実機の安定性は別ゲート。最終決定は Issue #398 に証拠を記録する

## 実測結果（2026-10-10、source `54a0bb9`）

| 位置づけ | Candidate | Kinds | V / D / B | T |
|---|---|---|---|---:|
| この盤面の最高点 | `fleet:budget=official:iters=60000:chains=4:seed=3` | `S,P,P,P` | 10 / 49 / 102 | 2.0 s |
| 次点・未知盤面にも使える設定 | `fleet:budget=official:budget_ms=10000:adapt=off` | `--kinds-auto` | 10 / 49 / 94〜104 | 50 s |
| 現行の本番設定 | `fleet:budget=official:budget_ms=10000` | `--kinds-auto` | 10 / 48〜49 / 86〜104 | 3.9 s |

Windows（PowerShell）での起動:

```powershell
$env:PROCON_TOKEN = "<配布されたトークン>"
$env:PROCON_RESUME_RUNNING_MATCH = "1"
.\procon.exe coordinator --server http://<サーバー>:<ポート> --exit-after-match `
  --candidate "fleet:budget=official:budget_ms=10000:adapt=off" --kinds-auto
```

起動スクリプト `ops/honsen/honsen.ps1` を使う場合は `start -Server http://<IP>:<ポート> -Candidate "fleet:budget=official:budget_ms=10000:adapt=off"`。

最高点の設定は seed をこの盤面に合わせたもので、同じ盤面だと分かっている場合にだけ使う。Windows 実機と 10 チームが動く試合では未確認。
計測の内訳、フォールバック、未確認の項目は [docs/evidence/rehearsal-20261010/README.md](../../docs/evidence/rehearsal-20261010/README.md) にある。
