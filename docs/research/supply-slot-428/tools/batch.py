#!/usr/bin/env python3
"""batch.py BIN OUT.jsonl JOBS CAND[,,CAND...] CASE... : whole-match solo runs, one summary per line."""
import json, subprocess, sys, concurrent.futures as cf, os, time
bin_, out, jobs = sys.argv[1], sys.argv[2], int(sys.argv[3])
cands = sys.argv[4].split(",,")
cases = sys.argv[5:]
traffic = os.environ.get("TRAFFIC", "own")
def run(job):
    case, cand = job
    c = json.load(open(case))
    n = len(c["agents"])
    kinds = os.environ.get("KINDS") or ",".join(["S"] + ["P"] * (n - 1))
    t = time.time()
    if cand.startswith("@"):
        # "@label mode=portfolio options=a|b cont=c": raw harness arguments.
        extra = cand.split(" ")[1:]
    else:
        extra = ["mode=solo", f"candidate={cand}"]
    p = subprocess.run([bin_, case, "/dev/null", f"traffic={traffic}", f"kinds={kinds}"] + extra,
                       capture_output=True, text=True)
    rec = None; idle = []
    for line in p.stdout.splitlines():
        r = json.loads(line)
        if r["kind"] == "summary":
            rec = r
        elif r["kind"] == "day":
            w = 0
            for a, wire in zip(r["agents"], r["wire"]):
                if a["kind"] == "Patrol":
                    w += sum(-x for x in wire if x < 0)
            idle.append(w)
    if rec is None:
        return {"case": os.path.basename(case), "cand": cand.split(" ")[0], "error": p.stderr[-300:]}
    series = len(set(s["brand"] for s in c["spots"]))
    return {"case": os.path.basename(case)[:-5], "cand": cand.split(" ")[0], "V": rec["V"], "D": rec["D"], "B": rec["B"],
            "series": series, "days": len(c["daySteps"]), "dD": rec["daily_D"], "dB": rec["daily_B"],
            "ms": rec["decide_ms"], "bad": rec["illegal_action_count"] + rec["fuel_short_count"] + rec["rejected_response_count"],
            "wall": round(time.time() - t, 1), "idle": idle, "steps": c["daySteps"], "np": n - 1}
with cf.ThreadPoolExecutor(jobs) as ex, open(out, "a") as fh:
    for r in ex.map(run, [(c, k) for c in cases for k in cands]):
        fh.write(json.dumps(r) + "\n"); fh.flush()
        if "error" in r: print(r); continue
        print(f"{r['case']:24s} {r['cand'].replace('budget=official:',''):44s} V{r['V']}/{r['series']} D{r['D']}/{r['series']*r['days']} B{r['B']} dD{r['dD']} dB{r['dB']} maxms{max(r['ms'])} bad{r['bad']} idle%{[round(100*i/(s*r['np'])) for i,s in zip(r['idle'],r['steps'])]}", flush=True)
