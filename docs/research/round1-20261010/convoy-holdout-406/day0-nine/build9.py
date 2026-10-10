import sys, heapq, json, itertools
sys.path.insert(0, "docs/research/round1-20261010/tools")
from r1sim import Board, simulate_day
B = Board.load("data/round1/20261010T044644Z-p28796/setting.json")
LIMIT = B.day_steps[0]; CAP = B.fuel_cap
spots = sorted(B.spots); sidx = {p: i for i, p in enumerate(spots)}
def brands_of(m): return {B.spots[spots[i]][0] for i in range(len(spots)) if m >> i & 1}
def routes(start, want):
    """All Pareto (tick, fuel) labels whose brands include `want`, within the tank."""
    front = {}; parent = {}; heap = [(0, 0, start, 0, None)]; out = []
    while heap:
        t, f, c, m, par = heapq.heappop(heap)
        fr = front.setdefault((c, m), [])
        if any(t2 <= t and f2 <= f for t2, f2 in fr): continue
        fr.append((t, f)); me = (c, m, t, f); parent[me] = par
        if want <= brands_of(m): out.append(me)
        cost = B.step_cost(c, {}); fc = B.fuel_cost(c)
        if t + cost > LIMIT or f + fc > CAP: continue
        for d in range(6):
            n = B.neighbor(c, d)
            if n is None: continue
            heapq.heappush(heap, (t + cost, f + fc, n, m | (1 << sidx[n]) if n in sidx else m, (me, d)))
    def acts(me):
        a = []
        while parent[me] is not None: me, d = parent[me]; a.append(d)
        return a[::-1]
    return [(me, acts(me)) for me in out]
def pad(a, t):  # wire: moves then one negative wait for the rest
    return a + ([-(LIMIT - t)] if t < LIMIT else [])
A1 = [5, 0, 1, 0, 3, 4, 3, 3, 4, 3, 2, 3, 3, 2, 2, 2, 5, 5, 0, 0, 1, 1, 1, 1, 1, 1]
# supply: shortest path 0 -> 77, then stay
def path(src, dst):
    d = {src: 0}; prev = {}; h = [(0, src)]
    while h:
        t, c = heapq.heappop(h)
        if t > d[c]: continue
        for k in range(6):
            n = B.neighbor(c, k)
            if n is None: continue
            nt = t + B.step_cost(c, {})
            if nt < d.get(n, 999): d[n] = nt; prev[n] = (c, k); heapq.heappush(h, (nt, n))
    a = []; c = dst
    while c != src: c, k = prev[c]; a.append(k)
    return a[::-1], d[dst]
sp, st = path(B.start[0], 77)
out = []
for w2 in ({0, 1, 5}, {0, 5, 8}, {0, 1}, {0, 8}):
    r2 = routes(64, w2); r3 = routes(122, {6, 7, 9})
    for (me2, a2) in sorted(r2, key=lambda x: x[0][3])[:3]:
        for (me3, a3) in sorted(r3, key=lambda x: (x[0][3], x[0][2]))[:3]:
            plan = [pad(sp, st), pad(A1, 32), pad(a2, me2[2]), pad(a3, me3[2])]
            res = simulate_day(B, 0, [1, 0, 0, 0], list(B.start), [CAP] * 4, {}, plan)
            out.append({"want2": sorted(w2), "D": len(res.brands()), "B": res.balls(), "end": res.pos, "fuel": res.fuel,
                        "errors": res.errors, "refuels": res.refuels, "plan": plan})
            print(sorted(w2), "D", len(res.brands()), "B", res.balls(), "end", res.pos, "fuel", res.fuel, res.errors, res.refuels)
json.dump(out, open(sys.argv[1], "w"))
