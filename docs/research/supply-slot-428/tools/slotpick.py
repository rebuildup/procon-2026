#!/usr/bin/env python3
"""slotpick.py PROBE ROLLOUT OUT.jsonl WINDOW_MS JOBS CAND CASE... : probe picks a slot; play slot 0 and the pick."""
import json, subprocess, sys, concurrent.futures as cf, os
probe, rollout, out, window, jobs, cand = sys.argv[1:7]
cases = sys.argv[7:]
def play(case, slot):
    c = json.load(open(case)); n = len(c["agents"])
    kinds = ",".join("S" if i == slot else "P" for i in range(n))
    p = subprocess.run([rollout, case, "/dev/null", "mode=solo", "traffic=own", f"kinds={kinds}", f"candidate={cand}"], capture_output=True, text=True)
    for line in p.stdout.splitlines():
        r = json.loads(line)
        if r["kind"] == "summary": return [r["V"], r["D"], r["B"]]
picks = {}
for case in cases:  # probes run alone: they use every core
    r = json.loads(subprocess.run([probe, window, case], capture_output=True, text=True).stdout.splitlines()[-1])
    picks[case] = (r["slot"] or 0, r["matches"])
def run(case):
    slot, matches = picks[case]
    base = play(case, 0)
    new = base if slot == 0 else play(case, slot)
    return {"case": os.path.basename(case)[:-5], "slot": slot, "matches": matches, "slot0": base, "pick": new}
w = t = l = 0; d = [0, 0, 0]
with cf.ThreadPoolExecutor(int(jobs)) as ex, open(out, "a") as fh:
    for r in ex.map(run, cases):
        fh.write(json.dumps(r) + "\n"); fh.flush()
        a, b = tuple(r["slot0"]), tuple(r["pick"])
        w += b > a; t += b == a; l += b < a
        for i in range(3): d[i] += b[i] - a[i]
        print(r["case"], "slot", r["slot"], "rounds", r["matches"], a, "->", b, flush=True)
print(f"n={w+t+l} W/T/L {w}/{t}/{l} dV {d[0]:+d} dD {d[1]:+d} dB {d[2]:+d}")
