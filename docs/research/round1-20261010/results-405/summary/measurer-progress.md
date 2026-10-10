# 追加測量進捗 (measurer: addenda-415)

作業 worktree: `.worktrees/eval-wsl-405` (branch eval-convoy-415)
binary sha256: `a6737883ae651d799bc7106fef0703c1e3a0b34ab1ad6ce8366f1c5eec14a6c7` (`target/release/procon-eval`)
開始: 2026-10-10 19:5x JST

## 測定対象 (チーム指示)

1. `b3-20s-seed42/` — convoy@20000 を 9 corpus 盤面へ拡張 (seed42, kinds S,P×7) + fleet@10s / chains=8 アンカー
2. `c2-20s-s2/` — convoy@20000 × 補給 2 台 (`S,S,P,P,P,P,P,P`) 9 盤面 + fleet@10s アンカー (seed42)
3. round1 match3 solo chains8 @20s (seed42/7) → `wsl-10s/round1-solo.jsonl` 追記
4. (余裕があれば) `b3-20s-s9012-s7/` — convoy@20000 seed7 s9012

先行情報: setE (hard convoy/fleet iters=120000, holdout fleet iters=60000) が並行稼働中。
重負荷 eval は setE 完了後に 1 件ずつ実行する。

## ログ

- [開始] setE 稼働中 (`hard-convoy-iters120k` 実行中) を確認。待機の間に軽量タスク 3 (round1_rollout chains8@20s) を開始。
- [19:44] **タスク 3 完了**: round1 match3 solo / observed / chains8 @20s を seed42・seed7 の 2 run 実施し
  `wsl-10s/round1-solo.jsonl` に追記 (48 → 60 行、各 run 5 day + summary)。label は候補 ID
  (`fleet:budget=official:budget_ms=20000:adapt=off:chains=8:seed={42,7}`)。

  | run | B | D | V | decide_ms | illegal/fuel_short/rejected |
  |---|---|---|---|---|---|
  | chains8@20s seed42 | 55 | 43 | 10 | 20012–20055 (全日 20s 使用) | 0/0/0 |
  | chains8@20s seed7 | 55 | 43 | 10 | 20012–20056 (全日 20s 使用) | 0/0/0 |

  比較 (同盤面 round1-solo.jsonl 既存): chains8@10s seed42/7 は B=55/D=43 → **20s にしても chains8 の対局盤面スコアは不変**。
  convoy@20s seed42/7 (B=54/D=46) に対し chains8@20s は B+1 / D-3 (辞書順で chains8 側が勝ち)。
  deadline 裕度: 無し — harness は remaining_ms=u32::MAX のため daily deadline 制約は eval 側で別途確認 (下記タスク 5)。