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
cargo run --locked -p cli --bin procon -- match-config validate data/rehearsal/2026-10-10-105938.setting.json

# まず短い比較。出力先 target/rehearsal-20261010
python3 scripts/rehearsal_20261010_eval.py

# 広い探索予算比較・反復試行も含む
python3 scripts/rehearsal_20261010_eval.py --full
```

Windows PowerShell で動かす場合は:

```powershell
cargo build --release --locked -p bench --bin procon-eval
cargo run --locked -p cli --bin procon -- match-config validate data/rehearsal/2026-10-10-105938.setting.json
py -3 scripts/rehearsal_20261010_eval.py --bin target/release/procon-eval.exe --full
```

成果物: `target/rehearsal-20261010/report.md`, `results.csv`, `results.json`, 各 candidate/kinds 別の `runs.jsonl` と stderr/stdout。保存対象の証拠は `docs/evidence/rehearsal-20261010/` にコピーし、rawログに token がないことを点検してからPRへ追加する。**結果値や勝者は未実測のまま文書に書かない。**

注意: `procon-eval` の `--cases` は bundled fixture を最低1つ指定する仕様なので、スクリプトは `small-8x8` も起動する。**結果表では case_id=`fixture:rehearsal-20261010-105938` のみを評価する**。`--case-files` へ生の `*.setting.json` を直接指定しない。

## 判定（最優先は得点）

- 実測した4軸 `(variety, daily_variety_sum, total_balls, -response_time_us)` を **辞書順**で比較。応答速度でV/D/Bの不足を補正しない
- invalid/timeout/panic・拒否/違法行動/燃料不足・day budget超過があれば除外する
- `fleet` は `--kinds S,P,P,P`（Supply slot0）と `--kinds P,P,P,P` を比較。従来既定の `P,S,P,S` と区別
- `--kinds-auto` は coordinator 側にのみ存在する。スクリプトの判定は iters=8000 で比較した **近似**。coordinator の1.5秒締切を本番環境で確認する
- 時間依存 `budget_ms` は CPU・負荷と乱数の影響を受けるため `--full` の反復結果を確認し、V/Dが揺れるケースを「確定」と扱わない
- 探索時間は各日60秒に収め、車種構成の提出は別の30秒締切に収める。過去の予行演習は途中退場しているため、本来の競技スコアとして扱わない
- これは **単独チームの canonical offline replay**。10名の道路競合、公式HTTPによる5日受理、Windows本番実機の安定性は別ゲート。最終決定は Issue #398 に証拠を記録する

## 現行の比較基準（過去 evidence、今回の勝者ではない）

過去の研究コーパスでは `fleet:budget=official:budget_ms=10000` + `--kinds-auto` が有力。
[既存 fleet evidence](../../docs/evidence/campaign-311/fleet-371/README.md) と
[本選準備 evidence](../../docs/evidence/honsen-385/README.md) を参照。
この盤面では燃料40・日数5のため、全車巡回と補給1台のどちらがV/D/Bを改善するか **実測で決める**。
