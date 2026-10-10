"""Hard boards with a slot-symmetric filter: whichever agent is the supply, the
other three reach at most 9 series on day 0 without meeting it. Records, per
supply slot, the day-0 caps (no meeting / one patrol escorted from a meeting)."""
import sys, os, json, heapq, importlib.util
spec = importlib.util.spec_from_file_location("g", sys.argv[1]); g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
SIZE = g.SIZE
def caps(case, slot):
    grid = [v for row in case["map"]["cells"] for v in row]
    spots = {s["pos"]: s["brand"] for s in case["spots"]}
    def adj(p):
        r, c = divmod(p, SIZE)
        return [rr * SIZE + cc for rr, cc in g.nbrs(r, c) if grid[rr * SIZE + cc] != 3]
    sd = {case["agents"][slot]: 0}; h = [(0, case["agents"][slot])]
    while h:
        t, c = heapq.heappop(h)
        if t > sd[c]: continue
        for n in adj(c):
            nt = t + g.STEP[grid[c]]
            if nt < sd.get(n, 999): sd[n] = nt; heapq.heappush(h, (nt, n))
    per = []
    for i, start in enumerate(case["agents"]):
        if i == slot: continue
        front = {}; tank = set(); met = set(); heap = [(0, 0, start, 0, 0)]
        while heap:
            t, f, c, m, e = heapq.heappop(heap)
            fr = front.setdefault((c, m, e), [])
            if any(t2 <= t and f2 <= f for t2, f2 in fr): continue
            fr.append((t, f)); (met if e else tank).add(m)
            if not e:
                arrive = max(sd.get(c, 999), t, 1)
                if arrive <= g.DAY_STEPS[0]: heapq.heappush(heap, (arrive, 0, c, m, 1))
            cost = g.STEP[grid[c]]; burn = 0 if e else g.BURN[grid[c]]
            if t + cost > g.DAY_STEPS[0] or f + burn > g.FUEL: continue
            for n in adj(c): heapq.heappush(heap, (t + cost, f + burn, n, m | (1 << spots[n]) if n in spots else m, e))
        mx = lambda s: [a for a in s if not any(a != b and a & b == a for b in s)]
        per.append((mx(tank), mx(tank | met)))
    best = lambda l: max(bin(a | b | c).count("1") for a in l[0] for b in l[1] for c in l[2])
    free = best([p[0] for p in per])
    one = max(best([per[q][1] if q == p else per[q][0] for q in range(3)]) for p in range(3))
    return free, one
out, count, seed, last = sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
kept = 0
while kept < count and seed < last:
    case = g.generate(seed); seed += 1
    if case is None: continue
    # cheap reject first: slot 0
    c0 = caps(case, 0)
    if c0[0] > 9: continue
    cs = [c0] + [caps(case, s) for s in (1, 2, 3)]
    if max(c[0] for c in cs) > 9: continue
    name = case["source"]["name"].replace("round1-like", "sym")
    case["case_id"] = "fixture:" + name; case["source"]["name"] = name
    json.dump(case, open(os.path.join(out, name + ".json"), "w"), separators=(",", ":"))
    json.dump(cs, open(os.path.join(out, name + ".caps"), "w"))
    kept += 1; print(name, cs, flush=True)
