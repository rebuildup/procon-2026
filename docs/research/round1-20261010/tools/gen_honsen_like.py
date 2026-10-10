"""Hold-out boards shaped like the two real 2026-10-10 boards (#403 / #405).

Both real boards (rehearsal, round one) share the frame: 16x16, 4 agents,
5 days, daySteps 32/36/40/40/48, fuel 40, 10 series on 16 spots, traffic
thresholds 1/2, 10 players. Only the terrain, the spot layout and the agent
starts differ. This draws those three at random — terrain shares between the
two real boards' (plain .23-.37, road .13-.25, mountain .09-.33, pond
.18-.41), lightly smoothed into blobs — and keeps everything else fixed.

None of these boards was used to design or tune a candidate.

    python3 docs/research/round1-20261010/tools/gen_honsen_like.py OUT_DIR [COUNT] [FIRST_SEED]
"""

from __future__ import annotations

import collections
import json
import os
import random
import sys

SIZE, AGENTS, SPOTS, SERIES = 16, 4, 16, 10
DAY_STEPS = [32, 36, 40, 40, 48]
STOCKS = [1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 3, 3, 4, 4]


def hex_neighbors(r: int, c: int):
    even = r % 2 == 0
    for dr, dc in ((0, -1), (0, 1), (-1, 0 if even else -1), (-1, 1 if even else 0),
                   (1, 0 if even else -1), (1, 1 if even else 0)):
        rr, cc = r + dr, c + dc
        if 0 <= rr < SIZE and 0 <= cc < SIZE:
            yield rr, cc


def generate(seed: int) -> dict:
    rng = random.Random(seed)
    share = [rng.uniform(.23, .37), rng.uniform(.13, .25), rng.uniform(.09, .33), rng.uniform(.18, .41)]
    grid = [[rng.choices(range(4), share)[0] for _ in range(SIZE)] for _ in range(SIZE)]
    for _ in range(1):
        nxt = [[0] * SIZE for _ in range(SIZE)]
        for r in range(SIZE):
            for c in range(SIZE):
                count = collections.Counter({grid[r][c]: 2.5 + rng.random() * 2})
                for rr, cc in hex_neighbors(r, c):
                    count[grid[rr][cc]] += 1
                nxt[r][c] = max(count, key=lambda k: (count[k], rng.random()))
        grid = nxt
    # Largest pond-free connected component; everything else becomes pond so
    # every traversable cell is reachable (guideline).
    seen: dict[tuple[int, int], int] = {}
    best: list[tuple[int, int]] = []
    for r in range(SIZE):
        for c in range(SIZE):
            if grid[r][c] == 3 or (r, c) in seen:
                continue
            queue = [(r, c)]
            seen[(r, c)] = 1
            i = 0
            while i < len(queue):
                for nb in hex_neighbors(*queue[i]):
                    if grid[nb[0]][nb[1]] != 3 and nb not in seen:
                        seen[nb] = 1
                        queue.append(nb)
                i += 1
            if len(queue) > len(best):
                best = queue
    keep = set(best)
    for r in range(SIZE):
        for c in range(SIZE):
            if (r, c) not in keep:
                grid[r][c] = 3
    cells = sorted(best)
    rng.shuffle(cells)
    picked = cells[:SPOTS + AGENTS]
    brands = list(range(SERIES)) + [rng.randrange(SERIES) for _ in range(SPOTS - SERIES)]
    stocks = STOCKS[:]
    rng.shuffle(stocks)
    spots = []
    for (r, c), brand, stock in zip(picked[:SPOTS], brands, stocks):
        grid[r][c] = 0
        spots.append({"brand": brand, "pos": r * SIZE + c, "stocks": stock})
    spots.sort(key=lambda s: (s["brand"], s["pos"]))
    agents = []
    for r, c in picked[SPOTS:]:
        grid[r][c] = 0
        agents.append(r * SIZE + c)
    name = f"honsen-like-s{seed}"
    return {
        "case_id": f"fixture:{name}", "seed": None, "generation_parameters": None,
        "source": {"kind": "fixture", "name": name},
        "startsAt": 1791608168, "daySeconds": [60] * 5, "daySteps": DAY_STEPS,
        "map": {"height": SIZE, "width": SIZE, "cells": grid},
        "spots": spots, "agents": sorted(agents), "fuelLimits": 40, "players": 10,
        "busyThreshold": 1, "jammedThreshold": 2,
    }


def main() -> int:
    out = sys.argv[1]
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 24
    first = int(sys.argv[3]) if len(sys.argv) > 3 else 7001
    os.makedirs(out, exist_ok=True)
    for seed in range(first, first + count):
        case = generate(seed)
        if len(case["agents"]) < AGENTS or len(case["spots"]) < SPOTS:
            print(f"seed {seed}: component too small, skipped", file=sys.stderr)
            continue
        with open(os.path.join(out, f"{case['source']['name']}.json"), "w", encoding="utf-8") as fh:
            json.dump(case, fh, separators=(",", ":"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
