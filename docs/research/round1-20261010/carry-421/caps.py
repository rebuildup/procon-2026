"""Per-day team caps along a played match: (A) nobody meets the supply, (B) one patrol
is escorted from a meeting on and the others stay within their tank (achievable),
(C) every patrol may meet the supply independently (upper bound)."""
import sys, json, heapq, itertools
sys.path.insert(0, "docs/research/round1-20261010/tools")
from r1sim import Board, load_events, observed_days, traffic_of
B = Board.load("data/round1/20261010T044644Z-p28796/setting.json")
days = observed_days(load_events("data/round1/20261010T044644Z-p28796/events.jsonl"))
spots = sorted(B.spots); sidx = {p: i for i, p in enumerate(spots)}
brand = [B.spots[p][0] for p in spots]; brands = sorted(set(brand)); bidx = {b: i for i, b in enumerate(brands)}
def bm(m):
    o = 0
    for i in range(len(spots)):
        if m >> i & 1: o |= 1 << bidx[brand[i]]
    return o
def sdist(src, tr):
    d = {src: 0}; h = [(0, src)]
    while h:
        t, c = heapq.heappop(h)
        if t > d[c]: continue
        for k in range(6):
            n = B.neighbor(c, k)
            if n is None: continue
            nt = t + B.step_cost(c, tr)
            if nt < d.get(n, 999): d[n] = nt; heapq.heappush(h, (nt, n))
    return d
def sets(start, fuel0, limit, tr, sd):
    """brand mask -> (within tank?, with one meeting?)"""
    front = {}; m0 = (1 << sidx[start]) if start in sidx else 0
    heap = [(0, 0, start, m0, 0)]; tank = set(); met = set()
    while heap:
        t, f, c, m, e = heapq.heappop(heap)
        fr = front.setdefault((c, m, e), [])
        if any(t2 <= t and f2 <= f for t2, f2 in fr): continue
        fr.append((t, f)); (met if e else tank).add(bm(m))
        if not e and sd is not None:
            arrive = max(sd.get(c, 999), t, 1)
            if arrive <= limit: heapq.heappush(heap, (arrive, 0, c, m, 1))
        cost = B.step_cost(c, tr); fc = 0 if e else B.fuel_cost(c)
        if t + cost > limit or f + fc > fuel0: continue
        for d in range(6):
            n = B.neighbor(c, d)
            if n is None: continue
            heapq.heappush(heap, (t + cost, f + fc, n, m | (1 << sidx[n]) if n in sidx else m, e))
    return tank, tank | met
def maximal(s): return [a for a in s if not any(a != b and a & b == a for b in s)]
def best(lists):
    return max(bin(a | b | c).count("1") for a in lists[0] for b in lists[1] for c in lists[2])
for name in sys.argv[1:]:
    rows = [json.loads(l) for l in open(name)]
    smooth = "smooth" in name
    print(name)
    for d in rows[:-1]:
        day = d["day"]; tr = {} if smooth else traffic_of(days[day]["state"])
        limit = B.day_steps[day]; ag = d["agents"]
        sd = sdist(ag[0]["start"], tr)
        per = [sets(a["start"], a["fuel_start"], limit, tr, sd) for a in ag[1:]]
        A = best([maximal(p[0]) for p in per])
        Bc = max(best([maximal(per[q][1] if q == p else per[q][0]) for q in range(3)]) for p in range(3))
        C = best([maximal(p[1]) for p in per])
        print(f"  day {day}: convoy D {d['D']}  cap no-supply {A}  cap one-escort {Bc}  cap all-meet {C}  fuel {[a['fuel_start'] for a in ag[1:]]}", flush=True)
