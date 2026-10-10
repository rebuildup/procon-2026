"""Boards shaped like the round-one match-3 board (2026-10-10): its terrain shares,
its stock and series multiplicities, spots pushed to the border and agent starts
to the centre. A board is kept when its first day is as tight as the real one:
no team plan without a supply meeting reaches more than `MAX_FREE` series.

    python3 gen_round1_like.py OUT_DIR COUNT FIRST_SEED
"""
import collections, heapq, json, math, os, random, sys

SIZE, AGENTS, SPOTS, SERIES = 16, 4, 16, 10
DAY_STEPS = [32, 36, 40, 40, 48]
STOCKS = [1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 3, 4, 4, 4]
MULT = [1, 1, 1, 1, 1, 1, 2, 2, 2, 4]
FUEL = 40
MAX_FREE = 8
STEP = {0: 2, 1: 1, 2: 3}
BURN = {0: 1, 1: 2, 2: 2}

def nbrs(r, c):
    even = r % 2 == 0
    for dr, dc in ((-1, 0 if even else -1), (-1, 1 if even else 0), (0, 1),
                   (1, 1 if even else 0), (1, 0 if even else -1), (0, -1)):
        rr, cc = r + dr, c + dc
        if 0 <= rr < SIZE and 0 <= cc < SIZE:
            yield rr, cc

def generate(seed):
    rng = random.Random(seed)
    share = [rng.uniform(.19, .27), rng.uniform(.21, .29), rng.uniform(.29, .37), rng.uniform(.14, .22)]
    grid = [[rng.choices(range(4), share)[0] for _ in range(SIZE)] for _ in range(SIZE)]
    nxt = [[0] * SIZE for _ in range(SIZE)]
    for r in range(SIZE):
        for c in range(SIZE):
            count = collections.Counter({grid[r][c]: 2.5 + rng.random() * 2})
            for rr, cc in nbrs(r, c):
                count[grid[rr][cc]] += 1
            nxt[r][c] = max(count, key=lambda k: (count[k], rng.random()))
    grid = nxt
    seen = set(); best = []
    for r in range(SIZE):
        for c in range(SIZE):
            if grid[r][c] == 3 or (r, c) in seen:
                continue
            queue = [(r, c)]; seen.add((r, c)); i = 0
            while i < len(queue):
                for nb in nbrs(*queue[i]):
                    if grid[nb[0]][nb[1]] != 3 and nb not in seen:
                        seen.add(nb); queue.append(nb)
                i += 1
            if len(queue) > len(best):
                best = queue
    keep = set(best)
    for r in range(SIZE):
        for c in range(SIZE):
            if (r, c) not in keep:
                grid[r][c] = 3
    cells = sorted(best)
    if len(cells) < SPOTS + AGENTS + 20:
        return None
    edge = lambda rc: min(rc[0], rc[1], SIZE - 1 - rc[0], SIZE - 1 - rc[1])
    def draw(pool, weight, k):
        out = []
        pool = list(pool)
        for _ in range(k):
            w = [weight(x) for x in pool]
            x = rng.choices(pool, w)[0]
            out.append(x); pool.remove(x)
        return out, pool
    spots_at, rest = draw(cells, lambda rc: math.exp(-edge(rc) / 1.3), SPOTS)
    agents_at, _ = draw(rest, lambda rc: math.exp(-abs(edge(rc) - 6) / 1.5), AGENTS)
    brands = [b for b, m in enumerate(MULT) for _ in range(m)]
    rng.shuffle(brands)
    stocks = STOCKS[:]; rng.shuffle(stocks)
    spots = []
    for (r, c), brand, stock in zip(spots_at, brands, stocks):
        grid[r][c] = 0
        spots.append({"brand": brand, "pos": r * SIZE + c, "stocks": stock})
    spots.sort(key=lambda s: (s["brand"], s["pos"]))
    for r, c in agents_at:
        grid[r][c] = 0
    name = f"round1-like-s{seed}"
    return {
        "case_id": f"fixture:{name}", "seed": None, "generation_parameters": None,
        "source": {"kind": "fixture", "name": name},
        "startsAt": 1791608168, "daySeconds": [60] * 5, "daySteps": DAY_STEPS,
        "map": {"height": SIZE, "width": SIZE, "cells": grid},
        "spots": spots, "agents": sorted(r * SIZE + c for r, c in agents_at), "fuelLimits": FUEL,
        "players": 10, "busyThreshold": 1, "jammedThreshold": 2,
    }

def free_cap(case):
    """Most series the three patrols reach on day 0 with no supply meeting."""
    grid = [v for row in case["map"]["cells"] for v in row]
    spots = {s["pos"]: s["brand"] for s in case["spots"]}
    def adj(p):
        r, c = divmod(p, SIZE)
        return [rr * SIZE + cc for rr, cc in nbrs(r, c) if grid[rr * SIZE + cc] != 3]
    per = []
    for start in case["agents"][1:]:
        front = {}; got = set(); heap = [(0, 0, start, 0)]
        while heap:
            t, f, c, m = heapq.heappop(heap)
            fr = front.setdefault((c, m), [])
            if any(t2 <= t and f2 <= f for t2, f2 in fr):
                continue
            fr.append((t, f)); got.add(m)
            cost, burn = STEP[grid[c]], BURN[grid[c]]
            if t + cost > DAY_STEPS[0] or f + burn > FUEL:
                continue
            for n in adj(c):
                heapq.heappush(heap, (t + cost, f + burn, n, m | (1 << spots[n]) if n in spots else m))
        per.append([a for a in got if not any(a != b and a & b == a for b in got)])
    return max(bin(a | b | c).count("1") for a in per[0] for b in per[1] for c in per[2])

if __name__ == "__main__":
    out, count, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    os.makedirs(out, exist_ok=True)
    kept = 0; caps = collections.Counter()
    while kept < count:
        case = generate(seed); seed += 1
        if case is None:
            continue
        cap = free_cap(case); caps[cap] += 1
        if cap > MAX_FREE:
            continue
        with open(os.path.join(out, case["source"]["name"] + ".json"), "w", encoding="utf-8") as fh:
            json.dump(case, fh, separators=(",", ":"))
        kept += 1
        print("kept", case["source"]["name"], "day-0 cap without supply", cap, flush=True)
    print("caps drawn", sorted(caps.items()), "next seed", seed)
