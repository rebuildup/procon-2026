"""Per-day caps along a played match on any case + traffic file (see caps.py)."""
import sys, json, heapq
case = json.load(open(sys.argv[1])); traffic_file = sys.argv[2]; run = sys.argv[3]
W = case["map"]["width"]; H = case["map"]["height"]; cells = [v for row in case["map"]["cells"] for v in row]
spots = {s["pos"]: s["brand"] for s in case["spots"]}; steps = case["daySteps"]
roads = {}
for l in open(traffic_file):
    e = json.loads(l)
    if e.get("type") == "day_state": roads[e["observed"]["day"]] = {t["pos"]: t["status"] for t in e["observed"]["traffics"]}
def nb(p):
    r, c = divmod(p, W); even = r % 2 == 0
    for dr, dc in ((-1, 0 if even else -1), (-1, 1 if even else 0), (0, 1), (1, 1 if even else 0), (1, 0 if even else -1), (0, -1)):
        rr, cc = r + dr, c + dc
        if 0 <= rr < H and 0 <= cc < W and cells[rr * W + cc] != 3: yield rr * W + cc
def cost(p, tr): return 2 if cells[p] == 0 else 3 if cells[p] == 2 else {0: 1, 1: 2, 2: 4}[tr.get(p, 0)]
def burn(p): return 1 if cells[p] == 0 else 2
def sdist(src, tr):
    d = {src: 0}; h = [(0, src)]
    while h:
        t, c = heapq.heappop(h)
        if t > d[c]: continue
        for n in nb(c):
            nt = t + cost(c, tr)
            if nt < d.get(n, 999): d[n] = nt; heapq.heappush(h, (nt, n))
    return d
def sets(start, fuel0, limit, tr, sd):
    front = {}; m0 = (1 << spots[start]) if start in spots else 0
    heap = [(0, 0, start, m0, 0)]; tank = set(); met = set()
    while heap:
        t, f, c, m, e = heapq.heappop(heap)
        fr = front.setdefault((c, m, e), [])
        if any(t2 <= t and f2 <= f for t2, f2 in fr): continue
        fr.append((t, f)); (met if e else tank).add(m)
        if not e:
            arrive = max(sd.get(c, 999), t, 1)
            if arrive <= limit: heapq.heappush(heap, (arrive, 0, c, m, 1))
        k = cost(c, tr); b = 0 if e else burn(c)
        if t + k > limit or f + b > fuel0: continue
        for n in nb(c): heapq.heappush(heap, (t + k, f + b, n, m | (1 << spots[n]) if n in spots else m, e))
    mx = lambda s: [a for a in s if not any(a != b and a & b == a for b in s)]
    return mx(tank), mx(tank | met)
def best(l): return max(bin(a | b | c).count("1") for a in l[0] for b in l[1] for c in l[2])
out = []
for line in open(run):
    d = json.loads(line)
    if d.get("kind") != "day": continue
    day = d["day"]; tr = roads.get(day, {}); ag = d["agents"]
    sup = [a for a in ag if a["kind"] == "Supply"][0]; pats = [a for a in ag if a["kind"] == "Patrol"]
    sd = sdist(sup["start"], tr)
    per = [sets(a["start"], a["fuel_start"], steps[day], tr, sd) for a in pats]
    A = best([p[0] for p in per]); Bc = max(best([per[q][1] if q == p else per[q][0] for q in range(3)]) for p in range(3)); C = best([p[1] for p in per])
    out.append({"day": day, "D": d["D"], "free": A, "one": Bc, "all": C})
print(json.dumps({"label": sys.argv[4], "days": out}))
