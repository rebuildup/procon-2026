# チーム側新ソース監査と個人側への実験移管（2026-10-10）

## 固定した参照

- チーム `cslab-nituc/procon2026` **main**: `dae61b9f9c19d5a3aa069f93714b8107971041ae`（新Rust solverを含む。2026-10-10）。
- チームPR [#25](https://github.com/cslab-nituc/procon2026/pull/25): `YAMAKENO16-patch-1`、`visualizer/docs/strategies.md` の系列比較（PRは **base=kimura-visualizer** でありチームmain導入済みではない）。
- チームPR [#28](https://github.com/cslab-nituc/procon2026/pull/28): ABC候補統合調整サーバー（未マージ、公式本番物理PC未検証）。
- 個人repo `main@7a93ad7`: `fleet` のベースライン。新Candidate `convoy` は [#407](https://github.com/rebuildup/procon-2026/pull/407) で独立開発中。

## チーム新Rustコードの確認

| ファイル / 実装 | 実際に確認した新要素 | 個人側との比較・判断 |
|---|---|---|
| `solver/src/ref_anneal_meet.rs` `past_repair_one` | 燃料不足で完走できない区間を探して、給油の約束を直前のチェックポイントまたはそれ以前へ戻し、到達可能性を再評価 | `fleet` の固定供給順の制限に対応する有望な近傍。ただし `convoy` の移動中合流・時間展開との重複を検証してから移植 |
| `solver/src/ref_anneal_meet.rs` `time_expanded_meeting` | 巡回車の時刻別位置に補給車の到達時間が間に合うセル・tickを探す | `fleet` の `World::intercept` は停車点のみ。 `convoy` は任意セル/tickで会合可能であり、まず `convoy` 対比較 |
| `solver/src/ref_legs_meet.rs` | 会合点探索のためキーセルから各セルへの最短時刻ツリーを保持 | `convoy` のDP高速化に寄与する可能性。メモリ・前計算オーバーヘッド含め比較 |
| `solver/src/day_replan.rs` | 車両ごとに未取得種類を優先する経路ビーム、時間・燃料制約経路、最後に補給を計画 | `fleet` と部分重複し、日末の翌日効果は未比較。直接置換しない |
| `solver/src/ref_anneal.rs` `choose_segment_variants` | 給油間の区間を時間・燃料のパレートDPで組み合わせる | 個人 #382 / PR #386 の単純な `legs=pareto` は15盤面で全体非改善。別モデルにおける初期解/給油組合せとして独立比較する |
| `base/src/simulate.rs` | チーム独自シミュレーション | 公式 `engine::MatchSim` と自動パリティ検証を通すまでは個人側の採点器を置換しない |

**新Rustの勝率・V/D/Bの対個人 `fleet` 実測結果は未取得**。`run_meet.rs` などのCLIは存在するが、同一盤面・同一kinds・公式engineによる横並び結果と異なる。

## チームPR #25での定量的ヒント

`visualizer/docs/strategies.md` の 16×16、合成12枚＋公式サンプル1枚×4条件（52試合）では、チームJSの `v1-greedy`→`v8-early-topup` が日別種類達成率 76.0%→92.5% と報告されている。孤立系列優先 `v5-isolated` は特定比較で +0.45 / 17勝10敗という説明があるが、同ページの表では `v4-tuned` 90.7% 対 `v5-isolated` 90.3% で **合算平均はむしろ低い**。先方の評価説明自体が比較集合を跨いでおり「全ケースで優位」とは主張できない。

さらに比較ツールは主に D満点比を集計するため、公式の順位軸 `V → D → B → 応答時間` の完全な採用判定ではない。固定マップの同一コーパス・同一車種構成・同一clockの canonical engine が必要。

### 実際に取り込んだ実装（実験のみ）

`fleet:budget=official:iters=20000:chains=4:seed=42:rarity=2`

- チームJSの孤立系列価値付けに着想を得て **Rustで独立実装**。元JS/Rustのコードをコピーしていない。
- 種類ごとのスポット出現数 `n` で初期経路生成の未取得候補の移動時間を `floor(t * n/(n+rarity))` として順位付けする。探索スコアや公式出力は変更しない（この式は独立の実験仮説であり、チーム側の `isoW` と同一アルゴリズムではない）。
- `rarity=0` が既定値。従来の距離順に完全一致。比較候補以外の既定 `fleet` と `--kinds-auto` の事前評価は変更なし。
- 0..4 のみ指定可。オフライン固定seedの再現性／公式 `BudgetMode::Official` 正規化固定点テストを追加。

### 未検証・採否

- `rarity=2` の公式 V/D/B の勝敗は **未測定**。本番設定変更・採用提案ではない。
- 公式参考16/24/32、hard12、holdout24、新規holdout、1回戦第3試合の観測交通で、同じ `fleet` seed/chains/kinds/iters による 0 対 2 の paired evalを行う。
- `V/D` の回帰があれば、玉数が増えていても棄却。上回ったときだけ複数seed、live 10秒、Windows実機で追認。
- 新Rustの補給修復は #407 の競合／重複領域を確認し、評価コーパスが揃うまで本Issueで直接 `fleet` へ移植しない。
- 原チームPR #25/#28 の成果は参考・競合候補であり、個人本番バイナリ／チームmainへ無断統合しない。
