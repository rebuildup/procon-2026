#!/bin/bash
# 2026-10-11 残バッチ逐次ドライバ（静穏ギャップ捕捉・nice 19・各バッチ間で静穏ゲート）
# 判定は人間（メインエージェント）が行う。スクリプトは実行と証跡出力のみ。
set -u
R=/home/basic/work/procon-2026/.worktrees/eval-wsl-405
E=$R/target/release/procon-eval
BAIL=/home/basic/work/procon-2026/.worktrees/bail-435-verify/target/release/procon-eval
C=docs/evidence/campaign-311/fleet-371
LOG=$R/docs/research/round1-20261010/results-405/summary/queue-20261011.log

say() { echo "[$(date +%H:%M:%S)] $*" >> "$LOG"; }

quiet_ok() {
  # load < 4 かつ 自分（eval-wsl-405）以外の procon-eval プロセスなし
  local l
  l=$(awk '{print int($1)}' /proc/loadavg)
  if [ "$l" -ge 4 ]; then return 1; fi
  local p c
  for p in $(pgrep -f procon-eval 2>/dev/null); do
    c=$(readlink /proc/$p/cwd 2>/dev/null)
    case "$c" in
      $R/*|*/eval-wsl-405) ;;  # 自分は許可（自前 worker はこのスクリプトしか出さない想定）
      */bail-435-verify*) return 1 ;;
      "") return 1 ;;
      *) return 1 ;;
    esac
  done
  return 0
}

wait_quiet() {
  say "waiting for quiet (batch: $1)"
  while ! quiet_ok; do sleep 45; done
  # 3 連続確認（60 秒間隔）で安定を取る
  for i in 1 2 3; do
    sleep 60
    if ! quiet_ok; then
      say "quiet lost during confirmation, retrying (batch: $1)"
      wait_quiet "$1"
      return
    fi
  done
  say "quiet confirmed, starting batch: $1"
}

cd "$R"

# Batch D: convoy depth ラダー（24×24 × depth 1/2/8）
wait_quiet D
nice -n 19 $E --quiet \
  --cases all \
  --case-files "$C/corpus-hard/hard-24-a6-d9-short-s9008.json,$C/corpus-hard/hard-24-a5-d7-short-s9005.json" \
  --kinds 'S,P,P,P,P,P,P,P' \
  --candidates 'convoy:budget=official:budget_ms=10000:seed=42:depth=1,convoy:budget=official:budget_ms=10000:seed=42:depth=2,convoy:budget=official:budget_ms=10000:seed=42:depth=8' \
  --jobs 2 \
  --output-dir docs/research/round1-20261010/results-405/wsl-10s/d-depth-ladder 2>&1 | tail -2 >> "$LOG"
say "batch D done"

# bail s9012 静穏再実行（parent vs bail・1 盤面）
wait_quiet BAIL-S9012
nice -n 19 $BAIL --quiet \
  --cases all \
  --case-files "$C/corpus-hard/hard-32-a8-d7-short-s9012.json" \
  --kinds 'S,P,P,P,P,P,P,P' \
  --candidates 'convoy:budget=official:budget_ms=20000:seed=42,convoy:budget=official:budget_ms=20000:seed=42:bail=on' \
  --jobs 1 \
  --output-dir /home/basic/work/procon-2026/.worktrees/bail-435-verify/eval-bail-s9012-quiet 2>&1 | tail -2 >> "$LOG"
say "bail s9012 quiet rerun done"

# Batch C: E3 予算カーブ（budget_ms ラダー × chains 4/8・4 盤面）
wait_quiet C
nice -n 19 $E --quiet \
  --cases all \
  --case-files "$C/corpus-hard/hard-32-a8-d7-short-s9012.json,$C/corpus-hard/hard-24-a6-d9-short-s9008.json,$C/corpus-holdout/hold-32-a8-d10-short-s5020.json,$C/corpus-holdout/hold-32-a6-d6-mixed-s5023.json" \
  --kinds 'S,P,P,P,P,P,P,P' \
  --candidates 'fleet:budget=official:budget_ms=2500:adapt=off:seed=42:chains=4,fleet:budget=official:budget_ms=5000:adapt=off:seed=42:chains=4,fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=4,fleet:budget=official:budget_ms=20000:adapt=off:seed=42:chains=4,fleet:budget=official:budget_ms=2500:adapt=off:seed=42:chains=8,fleet:budget=official:budget_ms=5000:adapt=off:seed=42:chains=8,fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=8,fleet:budget=official:budget_ms=20000:adapt=off:seed=42:chains=8' \
  --jobs 2 \
  --output-dir docs/research/round1-20261010/results-405/wsl-10s/e3-budget-ladder 2>&1 | tail -2 >> "$LOG"
say "batch C done"

# Batch A-slim: s5020 c8 fuel/mix offline 欠落 2 セル（+アンカー c8 で同実行比較）
wait_quiet A-SLIM
nice -n 19 $E --quiet \
  --cases all \
  --case-files "$C/corpus-holdout/hold-32-a8-d10-short-s5020.json" \
  --kinds 'S,P,P,P,P,P,P,P' \
  --candidates 'fleet:budget=official:iters=60000:seed=42:chains=8,fleet:budget=official:iters=60000:seed=42:chains=8:fuel_need=50,fleet:budget=official:iters=60000:seed=42:chains=8:fuel_need=50:fuel_mix=on' \
  --jobs 1 \
  --output-dir docs/research/round1-20261010/results-405/wsl-corpus/f3-a-slim 2>&1 | tail -2 >> "$LOG"
say "batch A-slim done"

say "ALL BATCHES COMPLETE"