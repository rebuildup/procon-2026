#!/usr/bin/env python3
"""Kinds composition auto-selection check (Issue #377).

Copied from fleet-371's kinds_auto_check.py, changed only in the corpus
dirs: the board list is passed as directories / files instead of the
hard-coded fleet-371 paths, so it can run over any pinned set — including
the never-tuned holdout2 family.

Simulates the live `--kinds-auto` rule offline: for every board, run the
selector candidate with (a) one supply first (`S,P,...`) and (b) all
patrol (`P,P,...`); adopt all-patrol only when it keeps V and D (the
first two official tiers) and gains B by the margin. Then score the
adopted composition with the full candidate and compare against
S-first.

Usage:
  kinds_auto_check.py <procon-eval> <selector-cand> <full-cand> <margin> <boards-dir>...
"""

import glob
import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

BIN = sys.argv[1]
SEL = sys.argv[2]
FULL = sys.argv[3]
MARGIN = float(sys.argv[4]) if len(sys.argv) > 4 else 1.05
DIRS = sys.argv[5:] or None
if DIRS:
    files = []
    for d in DIRS:
        p = os.path.abspath(d)
        files.extend(sorted(glob.glob(os.path.join(p, "*.json")))
                     if os.path.isdir(p) else [p])
else:
    S = os.path.dirname(os.path.abspath(sys.argv[0]))
    files = (sorted(glob.glob(S + "/corpus/*.json"))
             + sorted(glob.glob(S + "/corpus-holdout/*.json"))
             + sorted(glob.glob(S + "/corpus-hard/*.json")))
    files.append("crates/bench/tests/fixtures/arena-701.json")


def run(job):
    f, body, p, cand = job
    out = subprocess.run(
        [BIN, "--worker", "--candidate-id", cand, "--kinds", p, "--case-json", body],
        capture_output=True, text=True).stdout
    r = json.loads(out)
    return (r["variety"], r["daily_variety_sum"], r["total_balls"])


jobs = []
meta = []
for f in files:
    body = open(f).read()
    n = len(json.loads(body)["agents"])
    pats = [",".join("S" if j == 0 else "P" for j in range(n)),
            ",".join(["P"] * n)]
    for p in pats:
        jobs.append((f, body, p, SEL))
        meta.append((f, p))
with ThreadPoolExecutor(4) as ex:
    res = list(ex.map(run, jobs))

pick = {}
one = {}
zero = {}
for (f, p), sc in zip(meta, res):
    if "S" in p:
        if f not in one or sc > one[f][0]:
            one[f] = (sc, p)
    else:
        zero[f] = (sc, p)
for f in one:
    o = one[f]
    z = zero[f]
    # live rule: adopt all-patrol only when V/D hold and B gains the margin
    pick[f] = z if (z[0][:2] >= o[0][:2] and z[0][2] >= o[0][2] * MARGIN) else o

jobs2 = []
meta2 = []
for f in files:
    body = open(f).read()
    n = len(json.loads(body)["agents"])
    first = ",".join("S" if j == 0 else "P" for j in range(n))
    jobs2.append((f, body, first, FULL))
    jobs2.append((f, body, pick[f][1], FULL))
with ThreadPoolExecutor(4) as ex:
    res2 = list(ex.map(run, jobs2))

w = d = l = 0
tf = [0, 0, 0]
ta = [0, 0, 0]
zero_picks = 0
for i, f in enumerate(files):
    a, b = res2[2 * i], res2[2 * i + 1]
    w += b > a
    d += b == a
    l += b < a
    for k in range(3):
        tf[k] += a[k]
        ta[k] += b[k]
    z = "S" not in pick[f][1]
    zero_picks += z
    if b != a and (z or b[:2] != a[:2]):
        print(os.path.basename(f)[:-5], "first", a, "auto", b, pick[f][1])
print(f"auto vs S-first: W{w} D{d} L{l}  first {tf} auto {ta}  zero-supply picks {zero_picks}")
