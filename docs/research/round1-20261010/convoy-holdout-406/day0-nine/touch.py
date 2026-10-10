"""Can patrol a1 (start 59) take brands {2,3,4,5} on day 0 with one supply touch?
State (cell, mask, met) with Pareto (tick, fuel); met=1 after the touch (tank refilled)."""
import sys, heapq
sys.path.insert(0, "docs/research/round1-20261010/tools")
from r1sim import Board, simulate_day
B = Board.load("data/round1/20261010T044644Z-p28796/setting.json")
LIMIT = B.day_steps[0]; CAP = B.fuel_cap
spots = sorted(B.spots); sidx = {p: i for i, p in enumerate(spots)}
def dist(src):
    d = {src: 0}; h = [(0, src)]; prev = {}
    while h:
        t, c = heapq.heappop(h)
        if t > d[c]: continue
        for k in range(6):
            n = B.neighbor(c, k)
            if n is None: continue
            nt = t + B.step_cost(c, {})
            if nt < d.get(n, 999): d[n] = nt; prev[n] = (c, k); heapq.heappush(h, (nt, n))
    return d, prev
sd, sprev = dist(B.start[0])
def search(start, want_brands):
    front = {}; parent = {}
    heap = [(0, 0, start, 0, 0, None)]
    goal = None
    while heap:
        t, f, c, m, met, par = heapq.heappop(heap)
        key = (c, m, met)
        fr = front.setdefault(key, [])
        if any(t2 <= t and f2 <= f for t2, f2 in fr): continue
        fr.append((t, f)); parent[(key, t, f)] = par
        me = (key, t, f)
        got = {B.spots[spots[i]][0] for i in range(len(spots)) if m >> i & 1}
        if want_brands <= got:
            return me, parent
        # touch: wait here until the supply can stand on this cell
        if not met and t >= 0:
            arrive = max(sd.get(c, 999), t, 1)
            if arrive <= LIMIT:
                heapq.heappush(heap, (arrive, 0, c, m, 1, (me, ("touch", arrive))))
        cost = B.step_cost(c, {}); fc = B.fuel_cost(c)
        if t + cost > LIMIT or f + fc > CAP: continue
        for d in range(6):
            n = B.neighbor(c, d)
            if n is None: continue
            nm = m | (1 << sidx[n]) if n in sidx else m
            heapq.heappush(heap, (t + cost, f + fc, n, nm, met, (me, d)))
    return None, parent
for want in ({2, 3, 4, 5}, {2, 3, 4}):
    g, parent = search(59, want)
    print("want", want, "->", g)
    if g:
        acts = []; cur = g; touch = None
        while parent[cur] is not None:
            prev, a = parent[cur]
            if isinstance(a, tuple):
                touch = (prev[0][0], prev[1], a[1]); acts.append(("wait", a[1] - prev[1]))
            else: acts.append(a)
            cur = prev
        acts.reverse(); print("  actions", acts, "touch (cell, patrol arrives, refuel tick)", touch, "supply earliest there", sd[touch[0]])
