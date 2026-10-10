"""Boards with the frame of the organisers' 24x24 / 32x32 sample maps.

The 16x16 sample shares its frame (agents, daySteps, fuel, spot count) with
both real 2026-10-10 boards, so the later rounds are expected to share the
frame of the larger samples:

    24x24: 5 agents, daySteps 48/56/68/68/72, fuel 80, 24 spots
    32x32: 7 agents, daySteps 64/76/88/88/100, fuel 120, 32 spots

Terrain, spot layout, series multiplicities, stocks and starts are drawn at
random in two styles: `u` (uniform placement, sample-like stocks) and `r`
(round-one-like: spots towards the border, starts towards the centre, mostly
single stocks, fewer series).

    python3 gen_final_like.py OUT_DIR SIZE COUNT FIRST_SEED [tight] [mirror]

`mirror` writes `players: 1`: the simulator then counts our own road use as
every team's, which reproduces the practice arena's live ball counts within
about 3% (`players: 10` leaves the roads empty and overstates them by 5-25%).
"""
import collections, json, math, os, random, sys

FRAME = {
    16: (4, [32, 36, 40, 40, 48], 40),
    24: (5, [48, 56, 68, 68, 72], 80),
    32: (7, [64, 76, 88, 88, 100], 120),
}
# `tight`: the round-one frame scaled by the board side (fuel = 1.25 x day 1),
# which is also what the practice arena's `terrain: official` boards use
# (24x24: fuel 60, 48..72 steps).
TIGHT = {
    16: (4, [32, 36, 40, 40, 48], 40),
    24: (5, [48, 54, 60, 60, 72], 60),
    32: (7, [64, 72, 80, 80, 96], 80),
}


def nbrs(size, r, c):
    even = r % 2 == 0
    for dr, dc in ((-1, 0 if even else -1), (-1, 1 if even else 0), (0, 1),
                   (1, 1 if even else 0), (1, 0 if even else -1), (0, -1)):
        rr, cc = r + dr, c + dc
        if 0 <= rr < size and 0 <= cc < size:
            yield rr, cc


def generate(size, seed, tight=False):
    agents_n, day_steps, fuel = (TIGHT if tight else FRAME)[size]
    rng = random.Random(seed * 1000 + size)
    style = "ur"[seed % 2]
    spots_n = size
    # plain, road, mountain, pond: the hull of the samples and the real boards.
    share = [rng.uniform(.20, .37), rng.uniform(.13, .30), rng.uniform(.09, .37), rng.uniform(.14, .41)]
    grid = [[rng.choices(range(4), share)[0] for _ in range(size)] for _ in range(size)]
    for _ in range(rng.choice((0, 1, 1))):
        nxt = [[0] * size for _ in range(size)]
        for r in range(size):
            for c in range(size):
                count = collections.Counter({grid[r][c]: 2.5 + rng.random() * 2})
                for rr, cc in nbrs(size, r, c):
                    count[grid[rr][cc]] += 1
                nxt[r][c] = max(count, key=lambda k: (count[k], rng.random()))
        grid = nxt
    seen, best = set(), []
    for r in range(size):
        for c in range(size):
            if grid[r][c] == 3 or (r, c) in seen:
                continue
            queue = [(r, c)]
            seen.add((r, c))
            i = 0
            while i < len(queue):
                for nb in nbrs(size, *queue[i]):
                    if grid[nb[0]][nb[1]] != 3 and nb not in seen:
                        seen.add(nb)
                        queue.append(nb)
                i += 1
            if len(queue) > len(best):
                best = queue
    keep = set(best)
    for r in range(size):
        for c in range(size):
            if (r, c) not in keep:
                grid[r][c] = 3
    cells = sorted(best)
    if len(cells) < size * size * 0.45:
        return None
    edge = lambda rc: min(rc[0], rc[1], size - 1 - rc[0], size - 1 - rc[1])

    def draw(pool, weight, k):
        out, pool = [], list(pool)
        for _ in range(k):
            x = rng.choices(pool, [weight(x) for x in pool])[0]
            out.append(x)
            pool.remove(x)
        return out, pool

    if style == "u":
        spots_at, rest = draw(cells, lambda rc: 1.0, spots_n)
        agents_at, _ = draw(rest, lambda rc: 1.0, agents_n)
        series_n = round(spots_n * rng.uniform(.70, .80))
        stocks = [rng.randint(1, agents_n) for _ in range(spots_n)]
    else:
        scale = size / 16
        spots_at, rest = draw(cells, lambda rc: math.exp(-edge(rc) / (1.3 * scale)), spots_n)
        agents_at, _ = draw(rest, lambda rc: math.exp(-abs(edge(rc) - 6 * scale) / (1.5 * scale)), agents_n)
        series_n = round(spots_n * rng.uniform(.60, .70))
        stocks = [rng.choice((1, 1, 1, 1, 2, 2, 3, agents_n)) for _ in range(spots_n)]
    if tight:
        # Round one and the arena's official boards: about two balls a spot.
        stocks = [rng.choice((1, 1, 1, 2, 2, 3, 3, agents_n - 1)) for _ in range(spots_n)]
        series_n = round(spots_n * rng.uniform(.62, .75))
    brands = list(range(series_n)) + [rng.randrange(series_n) for _ in range(spots_n - series_n)]
    rng.shuffle(brands)
    spots = []
    for (r, c), brand, stock in zip(spots_at, brands, stocks):
        grid[r][c] = 0
        spots.append({"brand": brand, "pos": r * size + c, "stocks": stock})
    spots.sort(key=lambda s: (s["brand"], s["pos"]))
    for r, c in agents_at:
        grid[r][c] = 0
    name = f"{'tight' if tight else 'final'}-like-{size}{style}-s{seed}"
    return {
        "case_id": f"fixture:{name}", "seed": None, "generation_parameters": None,
        "source": {"kind": "fixture", "name": name},
        "startsAt": 1791608168, "daySeconds": [60] * 5, "daySteps": day_steps,
        "map": {"height": size, "width": size, "cells": grid},
        "spots": spots, "agents": sorted(r * size + c for r, c in agents_at), "fuelLimits": fuel,
        "players": PLAYERS, "busyThreshold": 1, "jammedThreshold": 2 if tight else rng.choice((2, 3)),
    }


if __name__ == "__main__":
    out, size, count, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
    tight = "tight" in sys.argv[5:]
    PLAYERS = 1 if "mirror" in sys.argv[5:] else 10
    os.makedirs(out, exist_ok=True)
    kept = 0
    while kept < count:
        case = generate(size, seed, tight)
        seed += 1
        if case is None:
            continue
        with open(os.path.join(out, case["source"]["name"] + ".json"), "w", encoding="utf-8") as fh:
            json.dump(case, fh, separators=(",", ":"))
        kept += 1
