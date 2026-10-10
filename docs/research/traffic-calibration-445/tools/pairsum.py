#!/usr/bin/env python3
"""pairsum.py FILE.jsonl... : fleet vs sized paired summary (lexicographic V, D, B) per board and seed."""
import json, sys, collections
for path in sys.argv[1:]:
    by = collections.defaultdict(dict)
    for line in open(path):
        r = json.loads(line)
        if "error" in r:
            print("ERROR", r); continue
        seed = r["cand"].split("seed=")[1].split(":")[0]
        by[(r["case"], seed)]["sized" if r["cand"].startswith("sized") else "fleet"] = r
    w = t = l = dd = db = dworse = 0
    print("##", path)
    for (case, seed), d in sorted(by.items()):
        if len(d) < 2:
            print(case, seed, "incomplete"); continue
        f, z = d["fleet"], d["sized"]
        a, b = (z["V"], z["D"], z["B"]), (f["V"], f["D"], f["B"])
        res = "W" if a > b else "L" if a < b else "T"
        w += res == "W"; t += res == "T"; l += res == "L"
        dd += a[1] - b[1]; db += a[2] - b[2]; dworse += a[:2] < b[:2]
        print(f"{case} seed{seed} fleet {b[0]}/{b[1]}/{b[2]} {sum(f['ms'])/1000:.0f}s | sized {a[0]}/{a[1]}/{a[2]} {sum(z['ms'])/1000:.0f}s "
              f"(max {max(z['ms'])/1000:.1f}s) | {res} dD{a[1]-b[1]:+d} dB{a[2]-b[2]:+d} bad {f['bad']}/{z['bad']}")
    print(f"sized W/T/L {w}/{t}/{l}  sum dD {dd:+d}  sum dB {db:+d}  V-or-D worse pairs {dworse}")
