#!/bin/bash
# 2026-10-11 第 2 ラウンド（静穏ゲート・nice 19・逐次）:
# Q1 s9003 dV−1 再現確認（seed 42/7 × fuel 変種）/ Q2 chains12 clocked（R1 差し替え候補）
# Q3 depth ラダー静穏再実行（B 確定用）
set -u
R=/home/basic/work/procon-2026/.worktrees/eval-wsl-405
E=$R/target/release/procon-eval
C=docs/evidence/campaign-311/fleet-371
LOG=$R/docs/research/round1-20261010/results-405/summary/queue-round2-20261011.log
say() { echo "[$(date +%H:%M:%S)] $*" >> "$LOG"; }

quiet_ok() {
  local l p c
  l=$(awk '{print int($1)}' /proc/loadavg)
  [ "$l" -lt 4 ] || return 1
  for p in $(pgrep -f procon-eval 2>/dev/null); do
    c=$(readlink /proc/$p/cwd 2>/dev/null)
    case "$c" in $R/*|*/eval-wsl-405) ;; *) return 1 ;; esac
  done
  return 0
}
wait_quiet() {
  say "waiting for quiet (batch: $1)"
  while ! quiet_ok; do sleep 45; done
  for i in 1 2 3; do
    sleep 60
    if ! quiet_ok; then say "quiet lost, retrying ($1)"; wait_quiet "$1"; return; fi
  done
  say "quiet confirmed, starting batch: $1"
}
cd "$R"

# Q1: s9003 dV−1 再現性（clocked・seed 42 と 7・c8 anchor/fuel/mix）
wait_quiet Q1
nice -n 19 $E --quiet \
  --cases all \
  --case-files "$C/corpus-hard/hard-16-a3-d9-short-s9003.json" \
  --kinds 'S,P,P,P,P,P,P,P' \
  --candidates 'fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=8,fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=8:fuel_need=50,fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=8:fuel_need=50:fuel_mix=on,fleet:budget=official:budget_ms=10000:adapt=off:seed=7:chains=8,fleet:budget=official:budget_ms=10000:adapt=off:seed=7:chains=8:fuel_need=50,fleet:budget=official:budget_ms=10000:adapt=off:seed=7:chains=8:fuel_need=50:fuel_mix=on' \
  --jobs 1 \
  --output-dir docs/research/round1-20261010/results-405/wsl-10s/q1-s9003-seed-confirm 2>&1 | tail -1 >> "$LOG"
say "Q1 done"

# Q3: depth ラダー静穏再実行（s9008/s9005 × depth 2/8 のみ・B 確定用）
wait_quiet Q3
nice -n 19 $E --quiet \
  --cases all \
  --case-files "$C/corpus-hard/hard-24-a6-d9-short-s9008.json,$C/corpus-hard/hard-24-a5-d7-short-s9005.json" \
  --kinds 'S,P,P,P,P,P,P,P' \
  --candidates 'convoy:budget=official:budget_ms=10000:seed=42:depth=2,convoy:budget=official:budget_ms=10000:seed=42:depth=8' \
  --jobs 2 \
  --output-dir docs/research/round1-20261010/results-405/wsl-10s/d-depth-ladder-quiet 2>&1 | tail -1 >> "$LOG"
say "Q3 done"

# Q2: chains12 clocked（R1 差し替え候補・32×32 12 盤面 + s9008 dV 追跡・chains8 同時アンカー）
wait_quiet Q2
H32="$C/corpus-hard/hard-32-a6-d7-short-s9011.json,$C/corpus-hard/hard-32-a7-d7-short-s9009.json,$C/corpus-hard/hard-32-a7-d7-short-s9010.json,$C/corpus-hard/hard-32-a8-d7-short-s9012.json,$C/corpus-holdout/hold-32-a6-d4-mixed-s5019.json,$C/corpus-holdout/hold-32-a6-d6-mixed-s5023.json,$C/corpus-holdout/hold-32-a7-d10-mixed-s5017.json,$C/corpus-holdout/hold-32-a7-d10-short-s5022.json,$C/corpus-holdout/hold-32-a7-d4-short-s5018.json,$C/corpus-holdout/hold-32-a7-d7-mixed-s5021.json,$C/corpus-holdout/hold-32-a8-d10-short-s5020.json,$C/corpus-holdout/hold-32-a8-d7-short-s5024.json,$C/corpus-hard/hard-24-a6-d9-short-s9008.json"
nice -n 19 $E --quiet \
  --cases all \
  --case-files "$H32" \
  --kinds 'S,P,P,P,P,P,P,P' \
  --candidates 'fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=8,fleet:budget=official:budget_ms=10000:adapt=off:seed=42:chains=12' \
  --jobs 2 \
  --output-dir docs/research/round1-20261010/results-405/wsl-10s/q2-chains12-clocked 2>&1 | tail -1 >> "$LOG"
say "Q2 done"

say "ROUND2 COMPLETE"