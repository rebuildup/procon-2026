# 収束判断と打ち切り — 本選直前（2026-10-11 00:23 JST 時点）

本番候補の比較計測（seed 42/7 holdout 16 盤）の収束判断と、実験パイプラインの打ち切り・継続の最終整理。
**本ファイルは観察と既存成果の変換のみで、新規計測は起動していない。**

---

## 1. 実行状態の観察（00:23 JST）

| PID | 起動時刻 | 内容 | 所有 | 本報告の扱い |
|---|---|---|---|---|
| 187464 | 00:12:17 | coordinator `--arena --next` fleet 59s（~123% CPU） | Arena セッション | 触らない（観察のみ） |
| 186637 | 00:10:27 | procon-eval batch2（seed 7・holdout 16 盤 × 5 候補・jobs 2） | 比較計測セッション | 完了待ち（ETA ~01:05、batch1 実績 ~55 分から推定） |
| 190144 | 00:16:45 | procon-eval bail-435-verify（convoy 20s vs convoy 20s+bail=on、4 盤面・kinds 8 台） | shell snapshot `…9542742-6s5ebi.sh` が `results-405/summary` への commit を伴う → cluster-report 系と同一セッションの高確度 | 所有者確定のため kill 禁止・観察のみ |

batch2 の 2 つの worker（fleet s7 / convoy s7）は 00:20 時点で稼働中。出力先 `target/eval-p1-s7/`（batch1 `eval-p1-s42/` と同形式、run 終了時に書かれる）。

---

## 2. 実験台帳の照合（報告 15〜20 × 既存成果）

PR #407 報告 20（13:15Z）が規定した実行順序 **P0 → E3 → F1 → F2 → F3 → E4** の各項目と、確定済み成果の対応。

**注意（台帳の欠損）**: `P0 → E3 → F1 → F2 → F3 → E4` の正式定義は報告 20 の言及のみで、PR #407 コメント群・Issue #423/#425/#317/#432/#435（本体+コメント）・`docs/` のいずれにも恒久記録が見つからなかった（2026-10-11 00:20 時点の検索）。P0/E3 は報告 15〜20 の文脈から特定可能、F2/F3/E4 は推定。再開が必要になった場合は定義の再文書化から行うこと。

| ID | 定義（出典） | 状態 | 暫定結論 | 判断 |
|---|---|---|---|---|
| P0 | both モード未完日の pool[0] 齟齬（実測 369 vs 期待 ≈416）の日別 chosen/pool tracing 観測（報告 20。tracing は実装済み・コード変更なし） | 未完（0/1。measurer2 に挿入済みと記録あり） | 診断対象が **convoy@10s s9012 の救済**（S2+S3 再設計）。S2 単独は既に取り下げ、convoy@10s hard 運用は報告 16 で棄却済み → **本番候補（fleet@59s）に無関係** | **打ち切り提案** |
| E3 改訂版 | chains12/16 の s9008 dV 追跡 + chains12 clocked corpus + chains12 の 20k/40k/59k ラダー（報告 19・20） | 未完 | offline では chains12 がピーク（B 11863・D-loss なし）だが **clocked は wall-clock 分割で別物**（報告 19 の注意書き）。既存の clocked 証跡は chains8 @10s で収束（B+23・@20s 追加利得なし）。本番候補は 59s で、chains12 @59s の検証は未確認 | **打ち切り提案**（chains8 の seed 依存性判定のみ batch2 で完了予定） |
| F1 | fuel_need 系（本選型盤面での利得検証） | **完了（既存証跡で判定可能）** | eval-428（Arena official 9 盤面・@10s）: V/D 差 0・B −1〜−8 一貫マイナス。batch1（holdout 16 盤・seed42）: dB −10・W/T/L 5/4/7・D−1 セル 2 | **打ち切り確定**（不採用） |
| F2 | fuel 系後続バッチ（報告 20 の順序参照。定義の恒久記録なし・推定） | 未定義 | F1 の結果が一貫マイナスのため後続に価値なし | **打ち切り提案** |
| F3 | 同上（推定: fuel 系の multi-seed 確認） | 未定義 | 同上 | **打ち切り提案** |
| E4 | 同上（推定: R1 条件付き選択の統合評価。報告 16 で R1 は「beam 配分再設計が成功した場合は不要になる可能性」付きで保持） | 未定義 | R1 の適用精度 4/6・oracle regret −21 は負荷依存。本番運用は盤面選択 handoff で 32×32 は fleet に帰着済み → 追加設計の価値が見えない | **打ち切り提案** |

### 既存成果との照合で「完了済み・再実行不要」と判定できるもの

| 成果 | 証跡 | 再実行不要の根拠 |
|---|---|---|
| chains8 @10s の corpus 評価 | #415（9 盤面 10W19T5L・B+46）＋ #407 報告 16/18（full 36 盤面・clocked 比較は B+16 だが s9008 dV−1 の seed 依存セル）＋ batch1 seed42（B+23・D−1 セル 2） | 3 つの独立系で同方向。残る論点は seed 依存性のみ → batch2 で判定 |
| chains16 @10s | batch1 seed42（B−3・7/2/7）＋ E1 offline（B 11858 < chains12 11863・4 敗 B-only） | **10s では chains8 に明確に劣。打ち切り条件成立** |
| fuel_mix 本選型評価 | eval-428 ＋ batch1 seed42 | 両系で利得なし → 不採用確定 |
| convoy @20s vs fleet@10s | #407 報告 20（9 盤面 dV 同点・B 差最大 13 = ±20 ノイズ域）＋ Track A 単一 run 曲線（20s 飽和）＋ batch1（4/0/12・D−1 セル 3） | 本番候補変更の見込みなし → 打ち切り |
| cluster=on 大盤面 | #432/#435（s5020 部分リフト B+2 ノイズ・s5017 リフトなし・s5014 全面リフトだが 24×24 は beam 構造上の利得なし）＋ rep1（B−4..−12 劣化） | 本番採用根拠にならない → 打ち切り |
| S2（pool[0] 予算強化）単独 | 報告 20（取り下げ推奨済み） | 完了済み |
| P3（persist = beam 2 倍化） | 報告 15（+18〜38・ただし persist 既定化は match3 D41 スパイクで非推奨） | 機序確定（律速は beam）。実装線としては報告 16 で棄却済み |
| Track A 予算曲線 | #425（s5014/s5017/s5020 × 10/20/30/40s。fleet 40k s5020 B678 > convoy 665..672） | 完了（#317 反映済み・14:38Z Close） |
| bail=on | PR #435（rt 100s→46s・B 揺れ帯・pool==1 盤面でのみ発火）＋ batch 計測実行中（PID 190144） | 方向性確定。完了計測の回収のみ |
| F 系列（tight-32 合成盤） | branch 428 README（合成盤 +3.9%・実盤面効果なし） | 完了済み（eval-428 と整合） |

---

## 3. 打ち切り判断（ユーザー指定の飽和基準を適用）

| 対象 | 判定 | 根拠（強度） |
|---|---|---|
| chains16 @ 10s | **打ち切り** | batch1 seed42 で B−3 = chains8（+23）より明確に劣（実測済み）。batch2 seed7 で最終確認後、確定 |
| fuel_need 系（F1〜F3） | **打ち切り** | 本選型盤面で利得なし（eval-428: B −1〜−8・batch1: dB−10）（実測済み） |
| convoy 大盤面の追加計測 | **打ち切り** | batch1 で D−1 セル 3（辞書順で劣）・@20s は fleet@10s 同水準（ノイズ域）・cluster=on の +2 はノイズ（実測済み）。本番候補変更の見込み乏しい |
| E3 残り（chains12/16 s9008 dV 追跡・40/59s の限界利益） | **打ち切り提案** | clocked @20s で収束済み（Track A・報告 19/20）→ 低期待価値。**例外: chains8 vs chains4 の D−1 が seed 依存かどうかは batch2（実行中・コスト追加なし）で判定** |
| P0（tracing 観測） | **打ち切り提案** | 診断対象が本番候補と無関係の convoy@10s 救済。本選後の convoy 再設計ラインを続けない限り価値なし |
| F2 / F3 / E4 | **打ち切り提案** | F1 の不採用確定で後続の前提が消失・E4 は定義未発見 |

---

## 4. 総合表（指定形式）

| 実験 | 状態 | 完了数 | 経過時間 | 暫定結論 | 次の判断 |
|---|---|---|---|---|---|
| batch1（seed 42・holdout 16 盤・10s・source `63a7e5e`） | **完了** | 16 盤 × 4 候補 | ~55 分 | chains8 B+23（D−1 セル 2）/ chains16 B−3 / fuel B−10 / convoy@20s 4/0/12・D−1 セル 3。V 損失 0 | 完了済み・再実行不要 |
| batch2（seed 7・同条件） | 実行中 | 0/16 出力（worker 2 並列稼働・00:10 開始） | ~13 分+（ETA ~01:05） | — | **完了時に chains8 の D−1 seed 依存性を判定** → chains16/fuel/convoy の seed7 確認も同時確定 |
| eval-428（Arena official 9 盤面・fuel_mix @10s） | 完了 | 9 盤面 | — | V/D 差 0・B 一貫マイナス | fuel 不採用確定（#428 報告済み） |
| cluster probes（source `b7e2160`・20s・seed 42） | 完了 | 3 盤面 | — | s5020 部分 pool リフト（B+2 ノイズ）/ s5017 リフトなし / s5014 全面リフト | 本番不採用。#432/#435 報告済み（cluster-report 系） |
| F 系列（tight-32 合成盤） | 完了 | — | — | 合成 +3.9%・実盤面効果なし | 完了済み |
| Track A 予算曲線 | 完了 | 3 盤面 × 4 予算 | — | convoy 20s 飽和・fleet 10s 飽和 | 完了済み（#317 反映） |
| E1 chains ラダー（offline） | 完了 | 45 runs × 3 | — | chains12 ピーク・chains8 は s9008 dV−4（決定論）・chains16 B 減少 | offline のみの知見。clocked E3 は打ち切り提案 |
| chains8 full corpus（36 盤面） | 完了 | 45 runs | — | D 非劣化（対決定論 baseline）・B+196 | 完了済み |
| P0（tracing 観測） | 未完 | 0/1 | — | 本番候補に無関係 | **打ち切り提案** |
| E3 改訂版 | 未完 | 0/〜3 | — | 低期待価値（@20s 収束済み） | **打ち切り提案**（batch2 完了で chains8 seed 依存性のみ判定） |
| F1（fuel 本選型） | 完了相当 | 9+16 盤面 | — | 利得なし | **打ち切り確定** |
| F2 / F3 / E4 | 未定義 | 0 | — | 前提消失 | **打ち切り提案** |
| bail-435-verify（PID 190144） | 実行中（他セッション所有） | — | 00:16 開始 | bail=on は rt 半減・B 揺れ帯（rep1 済） | 観察のみ・完了で #435 採否材料 |
| Arena coordinator（PID 187464・fleet 59s） | 実行中（Arena セッション所有） | — | 00:12 開始 | 本番候補の arena 実績継続 | **絶対に触らない** |

---

## 5. 本選推奨

### (a) 本番候補: **変更なし**

`fleet:budget=official:budget_ms=59000:adapt=off` @ `13a0106` + `--kinds-auto` を維持する。

| 変更候補 | 判断 | 根拠強度 |
|---|---|---|
| chains8 追加（fleet chains4→8） | **見送り** | **暫定**: @10s で B+23（seed42・実測済み）だが D−1 セル 2 があり seed 依存性が未判定（batch2 実行中）＋ **@59s での chains8 検証は未確認**。本選直前に辞書順リスクを負う価値がない。batch2 で D−1 なしでも、59s 未確認である限り変更しない |
| fuel_mix | **不採用** | **実測済み**: 本選型盤面で B 一貫マイナス（eval-428・batch1） |
| convoy 本番化 | **不採用** | **実測済み**: 本選型盤面で D 損失 3 セル・B 下位（batch1 4/0/12）。@20s は fleet@10s 同水準（ノイズ域）で上振れ根拠なし |
| cluster=on / bail=on（convoy param） | **作用なし（本番候補は fleet）** | 実測済み: cluster=on は大盤面 B 劣化、bail=on は convoy の rt 改善のみ |

**根拠**: arena official 実績（D 上限 6/7・rank 1×3・座礁 0・margin ~5.25s 恒常）は **実測済み**。chains8 の潜在利得（B+2〜3%）は辞書順リスクと wall-margin 検証不足（未確認）を上回らない。

### (b) fallback

1. 一次 fallback = 本番候補自体の既定無変更（chains4 現行・報告 16 の fallback と同一）
2. 最終安全網 = 決定論 offline 設定（`fleet:iters=60000:chains=4` clockless）— wall-clock 系の計時異常時に使用（報告 16 の対決定論 baseline 実績）
3. convoy 系統（20s・bail=on）は fleet が何らかの理由で失格になった場合の代替候補（辞書順リスク D−1 セルあり・本選では基本使用しない）

### (c) 打ち切り対象一覧

chains16 @10s / fuel_need 系（F1〜F3）/ convoy 大盤面追加計測 / E3 残り / P0 / F2・F3・E4（定義未発見を含む）

### (d) 継続対象と自動停止条件

| 対象 | 継続理由 | 自動停止条件 |
|---|---|---|
| batch2（seed 7・PID 186637） | chains8 の D−1 seed 依存性判定（計算コスト追加なし・既に実行中）。chains16/fuel/convoy の seed7 確認も同時に確定 | **完了時（ETA ~01:05）に打ち切り**。chains8 に D−1 が出たら本選不採用確定・実験ライン全停止。D−1 なしなら「本選後 59s 検証候補」として記録し停止 |
| bail-435-verify（PID 190144・cluster-report 系所有） | PR #435 bail=on の multi-rep 補完 | 完了回収のみ。所有セッションの判断に従う |
| Arena coordinator（PID 187464） | 本番候補の実戦継続監視 | 触らない。本選開始まで |

**停止条件（共通）**: 本選開始（2026-10-11 中）以降は新規計測を起動しない。全実験ラインは batch2 完了時点で打ち切り、以後は Arena セッションのみ。

---

## 6. 証跡ハッシュ（既存記録の再掲）

- batch1: source `63a7e5e` / binary `4ce8b1a0…`（`target/eval-p1-s42/aggregate.json`）
- cluster probes: source `b7e2160` / probe sha256 `8305f1ff…`（#435 報告）
- 本番候補: `13a0106`（Arena 標準構成・`--kinds-auto`）
- chains8 full corpus: `b4-chains8-full`（#407 報告 16・branch `eval-convoy-415` @ `03c3ada`）

本報告の数値は `target/eval-p1-s42/aggregate.json` から辞書順（V→D→B）比較を再計算し、PR #407 報告・チーム内共有値と一致することを確認済み（2026-10-11 00:22 JST）。
