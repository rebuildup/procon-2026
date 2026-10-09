"""Unused-seed holdout #2 generator (Issue #377).

Same synthetic-board distribution as fleet-371's `gen_corpus.py holdout`
mode — identical size / agent / day / short-day / fuel-mix logic — but the
seed family is one that has NEVER been used for any development or tuning
decision in this repo.

Seed families already consumed (do not reuse for holdouts):

  fuzz-0..239                gen_fuzz.py (robustness runs)
  1..8, 101..108             --generated axis / longrun_eval.py seeds
  1, 2, 3, 4, 5, 7, 17, 29, 42  fixture / candidate seeds
  1001..1024                 gen_corpus.py tune (adjustment-24)
  5001..5024                 gen_corpus.py holdout (holdout-24 — spent)
  9001..9012                 gen_corpus.py hard (hard-12 — partially spent)

This script draws from a fresh family: **61001..61024** for `hold2`
(24 boards) and **62001..62012** for `hard2` (12 worst-case boards:
every day at the step floor `W+H` and fuel at the 1.0x floor).

The generated boards are committed byte-for-byte under
`corpus-holdout2/` and `corpus-hard-extreme/`; the seed list below is the
durable contract. Re-running this script reproduces them exactly.

RULE — "never tune on the holdout": boards under `corpus-holdout2/` and
`corpus-hard-extreme/` must not drive any candidate parameter change.
They are scored after a candidate is frozen, and a failed score is
reported as a loss, not fed back into tuning. First-touch after
generation counts as tuning; keep every sweep on these boards in this
directory so the audit trail shows they were never touched earlier.

Usage: gen_holdout2.py <output-dir> hold2|hard2
"""

import collections
import json
import random
import sys

TAG = ""
SEED_BASE = {"hold2": 61000, "hard2": 62000}
SEED_COUNT = {"hold2": 24, "hard2": 12}


def nbrs(r, c, H, W):
    # 4-neighbour subset for connectivity safety (same as fleet-371).
    for dr, dc in ((0, 1), (0, -1), (1, 0), (-1, 0)):
        rr, cc = r + dr, c + dc
        if 0 <= rr < H and 0 <= cc < W:
            yield rr, cc


def gen(seed, size, agents, days, short, fuel_mult):
    rng = random.Random(seed)
    H = W = size
    frac = [rng.uniform(.2, .35), rng.uniform(.18, .3), rng.uniform(.2, .3), rng.uniform(.08, .2)]
    tot = sum(frac)
    frac = [f / tot for f in frac]
    g = [[rng.choices(range(4), frac)[0] for _ in range(W)] for _ in range(H)]
    for _ in range(4):  # majority smoothing -> blobs
        n = [[0] * W for _ in range(H)]
        for r in range(H):
            for c in range(W):
                cnt = collections.Counter()
                for dr in (-1, 0, 1):
                    for dc in (-1, 0, 1):
                        rr, cc = r + dr, c + dc
                        if 0 <= rr < H and 0 <= cc < W:
                            cnt[g[rr][cc]] += 1 + (dr == 0 and dc == 0) * rng.random() * 2
                n[r][c] = max(cnt, key=lambda k: (cnt[k], rng.random()))
        g = n
    # connectivity: largest non-pond 4-connected component
    comp = {}
    best = []
    for r in range(H):
        for c in range(W):
            if g[r][c] == 3 or (r, c) in comp:
                continue
            q = [(r, c)]
            comp[(r, c)] = 1
            i = 0
            while i < len(q):
                a, b = q[i]
                i += 1
                for x in nbrs(a, b, H, W):
                    if g[x[0]][x[1]] != 3 and x not in comp:
                        comp[x] = 1
                        q.append(x)
            if len(q) > len(best):
                best = q
    cells = best[:]
    rng.shuffle(cells)
    nser = {16: 13, 24: 20, 32: 27}[size]
    nspots = nser + rng.randint(1, max(2, size // 6))
    picked = cells[:nspots + agents]
    spots = []
    brands = list(range(nser)) + [rng.randrange(nser) for _ in range(nspots - nser)]
    for (r, c), b in zip(picked[:nspots], brands):
        g[r][c] = 0
        spots.append({"brand": b, "pos": r * W + c, "stocks": rng.randint(1, agents)})
    ag = []
    for r, c in picked[nspots:]:
        g[r][c] = 0
        ag.append(r * W + c)
    lo, hi = W + H, (W + H) * 4
    if short:
        hi = int((W + H) * (1.6 if short is True else short))
    ds = [rng.randint(lo, hi) for _ in range(days)]
    return {
        "case_id": f"fixture:{TAG}-{size}-a{agents}-d{days}-"
                   f"{'short' if short else 'mixed'}-s{seed}",
        "seed": None, "generation_parameters": None,
        "source": {"kind": "fixture",
                   "name": f"{TAG}-{size}-a{agents}-d{days}-"
                           f"{'short' if short else 'mixed'}-s{seed}"},
        "startsAt": 1778227200, "daySeconds": [60] * days, "daySteps": ds,
        "map": {"height": H, "width": W, "cells": g}, "spots": spots, "agents": ag,
        "fuelLimits": int(ds[0] * fuel_mult), "players": 10,
        "busyThreshold": 3, "jammedThreshold": 8,
    }


def main():
    out = sys.argv[1]
    mode = sys.argv[2] if len(sys.argv) > 2 else "hold2"
    assert mode in ("hold2", "hard2"), mode
    globals()["TAG"] = {"hold2": "hold2", "hard2": "hard2"}[mode]
    hard = mode == "hard2"
    seed = SEED_BASE[mode]
    names = []
    for size, agopts in ((16, (4, 4, 3, 5)), (24, (5, 5, 4, 6)), (32, (7, 7, 6, 8))):
        n = SEED_COUNT[mode] // 3 if hard else 8
        for i in range(n):
            seed += 1
            rng = random.Random(seed * 7)
            if hard:
                # Worst case: every day at the W+H step floor, fuel at the
                # 1.0x floor — strictly harder than fleet-371's hard-12
                # (which allows up to 1.15x steps).
                c = gen(seed, size, agopts[i % 4], rng.choice([7, 9, 10]),
                        short=1.0, fuel_mult=1.0)
            else:
                c = gen(seed, size, agopts[i % 4], rng.choice([4, 6, 7, 9, 10]),
                        short=(i % 2 == 1),
                        fuel_mult=[1.0, 2.4, 1.5, 3.0][i % 4] if i < 4 else rng.uniform(1, 3))
            p = f"{out}/{c['source']['name']}.json"
            with open(p, "w") as f:
                json.dump(c, f)
            names.append(p)
    print(",".join(names))


if __name__ == "__main__":
    main()
