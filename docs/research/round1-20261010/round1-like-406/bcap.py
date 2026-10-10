"""Last-day ball caps per patrol: most distinct spots one patrol can stand on,
(a) within its tank, (b) escorted from a supply meeting on. Team bound ignores
stock sharing: best over 'who is escorted' of the sum. Compared with the day's B."""
import sys, json, heapq
case = json.load(open(sys.argv[1])); traffic_file = sys.argv[2]; run = sys.argv[3]
W = case["map"]["width"]; H = case["map"]["height"]; cells = [v for row in case["map"]["cells"] for v in row]
spot_ids = {s["pos"]: i for i, s in enumerate(case["spots"])}; steps = case["daySteps"]
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
def most(start, fuel0, limit, tr, sd):
    front = {}; m0 = (1 << spot_ids[start]) if start in spot_ids else 0
    heap = [(0, 0, start, m0, 0)]; best = [0, 0]
    while heap:
        t, f, c, m, e = heapq.heappop(heap)
        fr = front.setdefault((c, m, e), [])
        if any(t2 <= t and f2 <= f for t2, f2 in fr): continue
        fr.append((t, f)); n1 = bin(m).count("1"); best[e] = max(best[e], n1)
        if not e:
            arrive = max(sd.get(c, 999), t, 1)
            if arrive <= limit: heapq.heappush(heap, (arrive, 0, c, m, 1))
        k = cost(c, tr); b = 0 if e else burn(c)
        if t + k > limit or f + b > fuel0: continue
        for n in nb(c): heapq.heappush(heap, (t + k, f + b, n, m | (1 << spot_ids[n]) if n in spot_ids else m, e))
    return best[0], max(best)
days = [json.loads(l) for l in open(run)]
d = [x for x in days if x.get("kind") == "day"][-1]
day = d["day"]; tr = roads.get(day, {}); ag = d["agents"]
sup = [a for a in ag if a["kind"] == "Supply"][0]; pats = [a for a in ag if a["kind"] == "Patrol"]
sd = sdist(sup["start"], tr)
per = [most(a["start"], a["fuel_start"], steps[day], tr, sd) for a in pats]
free = sum(p[0] for p in per); one = max(sum(per[q][1] if q == p else per[q][0] for q in range(3)) for p in range(3))
print(json.dumps({"label": sys.argv[4], "B_last": d["B"], "per_patrol_balls": [a["balls"] for a in pats], "free": [p[0] for p in per], "escorted": [p[1] for p in per], "bound_free": free, "bound_one": one, "fuel": [a["fuel_start"] for a in pats]}))
