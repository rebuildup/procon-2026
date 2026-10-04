#!/usr/bin/env bash
# Issue #243 ablation matrix — every condition on the same imported board.
#
# Prereqs (Step 1, run once the corrected /setting JSON is in place):
#   gh issue view 243 --json body -q .body > target/abl/issue-body.md
#   python3 scripts/ablation/extract_setting.py target/abl/issue-body.md target/abl/match489-setting.json
#   cargo run -q -p cli --bin procon -- arena import-case \
#       --input target/abl/match489-setting.json \
#       --output target/abl/match489-case.json --name match489
#   python3 scripts/ablation/make_variants.py target/abl/match489-case.json target/abl/match489-fuelinf.json
#
# Four candidates (owner's region-cause axis + the briefed STAR/anchor):
#   R1 region STAR        multiloop:kmeans=4:seed=42:walk=astar
#   R2 region-less, same walk            loop:iterations=32:initial=greedy:walk=astar
#   R3 region-less, extended ops + walk  loop:iterations=32:initial=greedy:operators=extended:walk=astar
#   R4 anchor (briefed)   loop:iterations=32:initial=greedy
#
# `--cases` always includes a tiny fixture so the loader itself is exercised;
# the aggregator filters records to case ids containing `match489`.
set -euo pipefail

cd "$(dirname "$0")/../.."

STAR='multiloop:kmeans=4:seed=42:walk=astar'
LOOP_WALK='loop:iterations=32:initial=greedy:walk=astar'
LOOP_EXT='loop:iterations=32:initial=greedy:operators=extended:walk=astar'
ANCHOR='loop:iterations=32:initial=greedy'
CANDS="$STAR,$LOOP_WALK,$LOOP_EXT,$ANCHOR"
ABL=target/abl

run() { # run <dir> <case-file> <baseline> [extra flags...]
  local dir="$1" case_file="$2" baseline="$3"
  local rc=0
  shift 3
  # procon-eval exits non-zero when a run is not rankable. On this board the
  # fuel-blind anchor legitimately is (illegal / fuel_short) — that IS the
  # evidence, so a non-zero exit must not abort the matrix. Records are
  # written before the exit, which is what we aggregate.
  CARGO_INCREMENTAL=0 cargo run -q -p bench --bin procon-eval --locked -- \
    --cases small-8x8 \
    --case-files "$case_file" \
    --candidates "$CANDS" \
    --baseline "$baseline" \
    --output-dir "$ABL/$dir" \
    --quiet \
    "$@" || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "  !! procon-eval exit=$rc for $dir (unrankable run(s) recorded — continuing)"
  fi
  echo "  -> $ABL/$dir"
}

# A — current: default kinds (from_wire alternation), fuel as imported.
echo "[A] current"
run A-current "$ABL/match489-case.json" "$ANCHOR"

# B — fuel-infinity: identical board with fuelLimits raised (counterfactual).
echo "[B] fuel-inf"
run B-fuelinf "$ABL/match489-fuelinf.json" "$ANCHOR"

# C — oracle-refill: default fuel, day-boundary Patrol refill (counterfactual).
echo "[C] oracle-refill"
run C-oracle "$ABL/match489-case.json" "$ANCHOR" --oracle-refill

# D — kind mix. Slot cycling (#238): `P,S` == the default for a 5-agent team,
# `P,P,S` -> P,P,S,P,P = 4P1S, `P,P,P` -> 5P0S.
echo "[D1] kinds P,S"
run D1-kinds-ps "$ABL/match489-case.json" "$ANCHOR" --kinds 'P,S'
echo "[D2] kinds P,P,S"
run D2-kinds-ppsp "$ABL/match489-case.json" "$ANCHOR" --kinds 'P,P,S'
echo "[D3] kinds P,P,P"
run D3-kinds-ppp "$ABL/match489-case.json" "$ANCHOR" --kinds 'P,P,P'

# E — assign=patrol on all candidates (Supply gets no tour).
echo "[E] assign=patrol"
CANDS="$STAR:assign=patrol,$LOOP_WALK:assign=patrol,$LOOP_EXT:assign=patrol,$ANCHOR:assign=patrol"
run E-assign "$ABL/match489-case.json" "$ANCHOR:assign=patrol"

echo
echo "== aggregate =="
python3 "$(dirname "$0")/aggregate.py" "$ABL"
