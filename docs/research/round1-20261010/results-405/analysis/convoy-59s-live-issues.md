# convoy@59s live 問題の行レベル分析（#317 台账 match 1999/2000）

対象: `crates/bench/src/adapters/convoy_solver.rs`（worktree eval-wsl-405, read-only 分析）。
実測: match 1999（10日・24x24 相当・5 agent）day0 pool24/depth8/reached5/56.1s、day1 pool58/reached1、day4-9 pool=1。match 2000（8日）day0 pool=1、day2 pool42/reached0/55.3s。

## 1. 「残天数=depth」と予算の非連結（day 0 全日先読みが 55s に収まらない）

構造（行番号は上記ファイル）:

- `decide` L1123-1124: `remaining = num_days - day - 1`、`depth = self.depth.min(remaining)`。depth は「先読み可能日数」でしかなく、wall-clock 予算と完全に非連結。10日マッチ day0 では remaining=9 → depth 8 が確定し、55s で間に合うかは一切考慮されない。
- 予算分割 L1119 / L1129-1133: root の `day_plans` に `started + w/3`（≈19.7s）、残り ≈39.3s を 2 scenario（forecast=both）で均等分割 → 各 scenario ≈19.6s。
- beam 1 レベルの展開コスト（`ahead` L1034-1091）: ノード数は width=24 に切られ（L1052）、各ノードで非 root `day_plans`（L1066）を 1 回呼ぶ。1 回あたり:
  - 内蔵 fleet `self.ahead.decide`（L802, 808）: `iters=20000, budget_ms=0`（L135）。budget 0 なので fleet 側 L1192 の window が None → **deadline を無視して必ず 20000 iter 走る**。root fleet の実測（iters=60000 / budget 4000ms、L128, L130）から 1 iter ≈ 15-66µs → 1 ノード ≈ 0.3-1.3s。ここが hard floor で、deadline で切り詰められない。
  - tank label search × np（L820-829）、escort/touch × 2np（L867-868）、riders（L883-900）、(b,a)×2 order × team() 組合せ（L902-988）。
  - `keep` → `play`（L795-800, L717-749）: plan ごとに `state.clone()` + 全 step シミュレータ replay。
- 実測裏付け: day0 で reached 5 = レベル5完了 ≈ 5x24x2 = 240 回の非 root day_plans が ≈39s → **1 ノード ≈0.16s**。depth 8 を完走するには 384 回 ≈ 60-115s が必要で、39.3s に対して構造的に不可能（チューニング余地なし）。
- deadline チェック粒度: `ahead` はレベル末でのみ時刻確認（L1082）。レベル途中は `patrol_day` 内の 8192 label ごとの確認（L461-464）と `late()`（L817）に頼る。レベルを開始したら 24 ノード分の day_plans が（deadline で各搜索が中断され pool=1 化しつつ）最後まで回る。

結論: day0 の truncated（reached<depth）は設計上必至。depth を予算から導出する変更（下記 §4）が必須。

## 2. pool=1 崩壊の原因判定

`decide` L1119 の root `day_plans` が返す計画は L808 の fleet plan が常に最初に入る。その後の全 emission は tank 搜索成功が前提。崩壊経路は 3 本:

1. L811: `Board::new` が None（steps 異常など）→ out は fleet のみ。
2. L813: `np == 0 || np > 8 || agent id 不一致` → 同上。
3. **L825-827: どれか 1 台でも patrol の tank label search が `None`（label cap 1.5M 超過 L461 または root deadline ≈19.7s 超過 L462）→ `return out` で即打ち切り、pool = fleet のみ。**

判定: match 1999 day4-9 / match 2000 day0,1,4-7 の pool=1 は **（3）が主因、「fleet 計画のみ」の状態**。 escort/touch 搜索の失敗（L867-868）は pool を崩さない（L881 で b を skip するだけ）。

- 残天数は無関係（variants は root 限定 L793、`last_day` は搜索終端条件にのみ影響 L818）。match 2000 は day0 で pool=1 → 盤面・位置依存。
- 連日再発の自己増幅: pool=1 → chosen=0（L1156）→ fleet 計画を採用 → fleet の day 終了状態がまた tank search を cap/deadline 越えさせる → 翌日も pool=1。day4 で一度落ちると day9 まで固定されるのはこの負ループ。
- 根本原因の二つ書き: (a) tank 搜索 1 台の失敗で全 pool を放棄する all-or-nothing 設計（L825-827）、(b) 失敗時に理由が記録されず（tracing に pool=1 のみ）、cap 縮小再試行などの縮退もない。

## 3. best-so-far の質: 55s が truncated 時に買うもの

- L1145-1146: `complete &= reached == depth`。1 scenario でも depth 未達なら complete=false。
- L1153-1156: 採用 filter は `complete && sums[k].is_some() && sums[k] > sums[0]`。complete=false なら **非 fleet root は全て棄却、chosen=0 = fleet 計画**。
- つまり truncated 日に 55s で買える質向上は **ゼロ**（fleet 自身の ≈4s budget 分 L130 と同一の計画を返す）。`pool[k].today()`（今日の replay 値: variety/series/balls/fuel、L698-700）による tie-break（L1155）も complete 前提なので機能しない。sums に一部入っている best-so-far（L1093-1099 のレベル別集計）も捨てられる。
- pool=1 日はさらに悪い: `1..pool.len()` が空なので best は常に 0。それでも scenario 分割時間 ≈39s を `ahead` は depth レベル分回し続ける（各レベル、ノード1の day_plans = 内蔵 fleet 20000 iter + 失敗する tank 搜索を deadline まで）。「探索自明化・予算未活用」の実体はこれ。
- 保守側で既にある救済材料: (i) sums を depth ごとに段階更新すれば「到達した深さまでの比較」が可能、(ii) `pool[k].today()` 単独の比較は常に計算済み、(iii) 根 pool 自体は今日の質で並べ替え可能。

## 4. 最小検証条件の提案（実装はしない）

### 4.1 offline 再現条件
- 参数: `budget=official budget_ms=59000 depth=8 width=24 forecast=both labels=1500000 seed=42`（live 既定と同一）。
- 盤面: match 1999/2000 の case JSON + 対戦 transcript からの day ごと状態再構成（bench の offline 対局再生は決定的なので、live の day 状態を正確に再現できるはず。MatchState スナップショット保存が難しければ transcript replay で足りる）。
- 観測点: 既存 `tracing::info!`（L1157-1165: day/pool/depth/complete/chosen/elapsed_ms）に加え、(a) scenario ごとの `reached`、(b) `np`/`steps`/spot 数、(c) **pool==1 の理由**（L811/L813/L825 のどれか、cap か deadline か）。最小差分なら pool==1 のときだけ reason を 1 フィールド追加。
- 成功判定: day0 で reached<depth、day4+（1999）・day0（2000）で pool==1 が決定的に再現すること。内蔵 fleet は seed 固定、`ahead` の children は k ソート（L1085）で決定的。
- 検証の分割: (A) day0 truncation 再現（予算非連結の確認）、(B) pool=1 day の状態スナップショット 1 個での再現（tank 搜索が cap/deadline 越えすることの確認、L825 直前に一時 tracing を入れるのが最短）。

### 4.2 depth 限制設計の提案式
- 案1（最小差分・推奨）: `ahead` 内でレベル開始前に「直前レベルの実測時間 x1.2 > 残り時間なら break」（reached は完了レベル数のまま）。decide 側は無変更で complete の意味が「予算内で完走できた深さまで比較した」に変わる。ただし scenario 間で reached が不一致になり得るので、complete 判定は「全 scenario が同じ reached」に置き換えるのが安全。
- 案2（事前計算式）: `depth_cap = clamp((w - w/3) / (scenarios.len() * est_level_ms), 1, min(self.depth, remaining))`、`est_level_ms` は root day_plans の実測（≈pool/24 × 0.16s × 1.2）から初期推定しレベルごとに EMA 更新。
- 案3（コスト低減）: 非 root の内蔵 fleet に budget を与える（L135 の budget_ms: 0 → until 由来の remaining_ms を fleet 側で使う）。これ単独で 1 ノード 0.3-1.3s の hard floor が外れ、depth 8 完走可能性が変わる。案1/2 と併用推奨。

### 4.3 pool 崩壊の回帰テスト
- 単体: 合成 24x24・5 patrol 状態で `day_plans(root, 十分な deadline)` の `len() > 1` を assert。加えて「tank 搜索 1 台を cap 強制失敗させても残り pool が生きる」縮退挙動のテスト（縮退を実装する場合）。
- corpus: match 1999 day4 / match 2000 day0 の状態で `pool >= 2` かつ `reached >= 1` をゴール条件に。現状コードではこの assert が落ちる（= 再現テストとして機能）。
- 守備変更の候補（実装者へ）: L825-827 で `return out` する前に、失敗した patrol のみ cap を 1/8 にして再試行、それでも失敗ならその patrol を tank のまま team に参加させる簡易計画で代用。pool=1 負ループ（§2）を断ち切るのが目的。