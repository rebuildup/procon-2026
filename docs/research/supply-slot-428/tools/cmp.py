#!/usr/bin/env python3
"""cmp.py RUNS.jsonl BASE_SUBSTR NEW_SUBSTR : paired official-order comparison."""
import json, sys, collections
rows = [json.loads(l) for l in open(sys.argv[1])]
base, new = sys.argv[2], sys.argv[3]
by = collections.defaultdict(dict)
for r in rows:
    if "error" in r: continue
    by[r["case"]][r["cand"]] = r
w = t = l = 0; dV = dD = dB = 0; n = 0
for case, d in sorted(by.items()):
    a = [v for k, v in d.items() if k == base] or [v for k, v in d.items() if base in k]
    b = [v for k, v in d.items() if k == new] or [v for k, v in d.items() if new in k]
    if not a or not b: continue
    a, b = a[-1], b[-1]; n += 1
    ka, kb = (a["V"], a["D"], a["B"]), (b["V"], b["D"], b["B"])
    w += kb > ka; t += kb == ka; l += kb < ka
    dV += kb[0] - ka[0]; dD += kb[1] - ka[1]; dB += kb[2] - ka[2]
    if "-v" in sys.argv: print(f"{case:24s} {ka} -> {kb} {'W' if kb>ka else 'L' if kb<ka else '='}")
print(f"n={n} W/T/L {w}/{t}/{l}  dV {dV:+d} dD {dD:+d} dB {dB:+d}")
