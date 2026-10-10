"""Team-day existence search on the round-one board (day 0, all roads smooth).
Per patrol: label search over (cell, spot mask) with Pareto (tick, fuel), fuel uncapped.
Then: which brand unions are reachable when k patrols may exceed the tank."""
import sys, json, itertools, heapq
sys.path.insert(0, "docs/research/round1-20261010/tools")
from r1sim import Board
B = Board.load("data/round1/20261010T044644Z-p28796/setting.json")
LIMIT = B.day_steps[0]; CAP = B.fuel_cap
spots = sorted(B.spots); sidx = {p: i for i, p in enumerate(spots)}
brand = [B.spots[p][0] for p in spots]
brands = sorted(set(brand)); bidx = {b: i for i, b in enumerate(brands)}
def bmask(m):
    out = 0
    for i in range(len(spots)):
        if m >> i & 1: out |= 1 << bidx[brand[i]]
    return out
def labels(start):
    """(cell, mask) -> Pareto list of (tick, fuel)."""
    front = {}
    m0 = 0
    heap = [(0, 0, start, m0)]
    while heap:
        t, f, c, m = heapq.heappop(heap)
        fr = front.setdefault((c, m), [])
        if any(t2 <= t and f2 <= f for t2, f2 in fr): continue
        fr.append((t, f))
        cost = B.step_cost(c, {}); fc = B.fuel_cost(c)
        if t + cost > LIMIT: continue
        for d in range(6):
            n = B.neighbor(c, d)
            if n is None: continue
            nm = m | (1 << sidx[n]) if n in sidx else m
            heapq.heappush(heap, (t + cost, f + fc, n, nm))
    return front
starts = B.start[1:]
res = []
for s in starts:
    fr = labels(s)
    best = {}  # brand mask -> min fuel
    for (c, m), lst in fr.items():
        bm = bmask(m); mf = min(f for _, f in lst)
        if mf < best.get(bm, 999): best[bm] = mf
    # keep maximal brand sets per fuel class
    res.append(best)
    print("start", s, "labels", sum(len(v) for v in fr.values()), "brand sets", len(best),
          "max brands", max(bin(b).count("1") for b in best),
          "max brands within tank", max(bin(b).count("1") for b, f in best.items() if f <= CAP), flush=True)
def top(best, capped):
    items = [(b, f) for b, f in best.items() if (f <= CAP or not capped)]
    # drop dominated subsets
    keep = [b for b, f in items if not any(b != b2 and b & b2 == b for b2, _ in items)]
    return keep
for over in range(4):
    bestn = 0; ex = []
    for who in itertools.combinations(range(3), over):
        sets = [top(res[p], p not in who) for p in range(3)]
        for a in sets[0]:
            for b in sets[1]:
                ab = a | b
                for c in sets[2]:
                    n = bin(ab | c).count("1")
                    if n > bestn: bestn = n; ex = []
                    if n == bestn and len(ex) < 6:
                        ex.append((who, [[brands[i] for i in range(10) if x >> i & 1] for x in (a, b, c)],
                                   [res[p][x] for p, x in enumerate((a, b, c))]))
    print("patrols allowed over the tank:", over, "-> best union", bestn)
    for e in ex[:4]: print("   ", e)
