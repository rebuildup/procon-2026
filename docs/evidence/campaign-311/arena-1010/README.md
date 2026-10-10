# Arena evidence — 2026-10-10 (WSL2 移管後・official terrain 分布)

## 構成

- `runs-fleet.jsonl` — fleet baseline（`.worktrees/arena-wsl-13a0106` @ `13a0106`）の coordinator 実行記録。
  22:45-23:45 JST 窓（lobby match 2008/2012/2014/2016/2019/2021）+ それ以前の 1985 以降。
- `runs-convoy.jsonl` — convoy 59s 実測（`.worktrees/convoy-live-3f72a37` @ `3f72a37`）。実戦主力から外す判断の根拠。
- `run-*.log` — coordinator の実行ログ（`fleet-59sN` = 22:45 以降の fleet 連続投入）。
- `api/match-<id>.json` — 公式 `/api/matches/<id>` の収集（1985-2020）。
  **lobby id と採点 table id が 1 ずれることがある**（2016 lobby → 2017 table）。照合は rating 連鎖で行う。
- `casespec-official-*.json` — official terrain 盤面の offline CaseSpec 化（1997-2005 + 2012/2014/2015）。
  offline eval（`procon-eval --case-files`）で再現可能。eval-428 の再実行（`pr428-verify/target/eval-428/`）はこの 9 盤 + fixture を使用。

## マップ分布の確定事項

- match 1996（20:00）まで `iid`/`blob` → 1997 以降全試合 `terrain: official`・style `finals16/24/32`。
- 16×16 でも series 数が揺れる: 10 series 型（match 2012: V 上限 10/D 50）と 11 series 型（match 2014: V 11/D 53、match 2019: V 11/D 55）。
- fleet 59s baseline の実戦成績（official terrain・#317 報告ベース）: rank 3→1→2→1→3→1→6→2→5・D 上限 7/9・座礁 0・margin 恒常 ~5.24s。

## ソース SHA

`source-sha.txt` 参照。ベースラインは `13a0106`（v0.4.0 系・`fleet:budget=official:budget_ms=59000:adapt=off` + `--kinds-auto`）。
