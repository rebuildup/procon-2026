# 改善の記録形式 — 「どの日・どの経路が・なぜ」(Issue #377)

候補の作者が比較結果に添付する記録の標準形式。`eval_compare.py` の
`compare.json` に出るデータを参照番として使い、主張を検証可能にする。

## ファイルの置き場所

`docs/evidence/campaign-311/eval-377/runs/<バッチ名>/records/<case_id>-<候補ラベル>.md`
に 1 盤面 1 ファイル。バッチ名は `eval_compare.py --name` とそろえる。

## テンプレート

```markdown
# <case_id> — <候補ラベル> vs <参照ラベル>

- 比較バッチ: `runs/<バッチ名>/compare.json`（source_commit_sha は各レコード参照）
- 判定: win | draw | loss（V ?, D ?, B ?／応答 ?us vs ?us）

## 日別差分（compare.json boards[].boards[*].daily_ball_counts / daily_varieties より）

| day | Δballs | Δvariety | 受理 | 一言 |
|----:|-------:|---------:|------|------|
| 3 | +4 | +1 | ok | 補給合流が 6 歩短くなり第 3 系列を取得 |
| 7 | -2 | 0 | rejected | 渋滞で 1 歩超過（day_budget_exceeded_days=[7]） |

## 経路レベルの根拠（差が出た日のみ）

`cargo run -q -p bench --example campaign_trace -- <case.json> <candidate-id> <kinds>`
のステップ列（`d7s12 [c,f12,b3] -> M{NorthEast},…`）を 2 候補分並べ、
分岐点を 1〜2 行で指す。全ステップの貼り付けはしない。

## 原因の仮説

1〜3 行。次のどの層かを明示する: 割当 / 巡回順 / 経路 / 燃料配分 /
連鎖選択 / kinds 選択。対策は候補側の Issue にリンクする。
```

## 規則

1. **数値は compare.json からの参照**。手打ちの集計を貼らない。
   日別玉数 (`daily_ball_counts`)・日別受理 (`rejected_days`)・
   予算超過 (`day_budget_exceeded_days`) は `procon-eval` が公式
   リプレイから記録したもので、候補自身の主張ではない。
2. **応答時間を比較に入れる場合は `--jobs 1` の専用バッチ**。
   並列実行下の応答時間は軸 4 の証拠に使えない。
3. 勝ち筋と負け筋を**両方**書く。部分勝利から「全般的優越」とは
   書かない（セッション運用規則に同じ）。
4. 参照バッチはハッシュ (source_commit_sha) で固定する。候補の
   コードが変わったら新しいバッチを切る。
