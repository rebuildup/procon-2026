#!/usr/bin/env python3
"""slots.py BIN OUT.jsonl JOBS CAND CASE... : whole match for every single-supply slot."""
import json, subprocess, sys, concurrent.futures as cf, os
bin_, out, jobs, cand = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
def run(job):
    case, slot = job
    c = json.load(open(case)); n = len(c["agents"])
    kinds = ",".join("S" if i == slot else "P" for i in range(n))
    p = subprocess.run([bin_, case, "/dev/null", "mode=solo", "traffic=own", f"kinds={kinds}", f"candidate={cand}"], capture_output=True, text=True)
    for line in p.stdout.splitlines():
        r = json.loads(line)
        if r["kind"] == "summary":
            return {"case": os.path.basename(case)[:-5], "slot": slot, "cand": cand, "V": r["V"], "D": r["D"], "B": r["B"], "dD": r["daily_D"], "dB": r["daily_B"]}
    return {"case": case, "slot": slot, "error": p.stderr[-200:]}
jobsl = []
for case in sys.argv[5:]:
    n = len(json.load(open(case))["agents"])
    jobsl += [(case, s) for s in range(n)]
with cf.ThreadPoolExecutor(jobs) as ex, open(out, "a") as fh:
    for r in ex.map(run, jobsl):
        fh.write(json.dumps(r) + "\n"); fh.flush()
