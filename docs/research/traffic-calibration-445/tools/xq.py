#!/usr/bin/env python3
"""Experiment queue with timeouts, exit codes, logs and resume.

    xq.py add  ID TIMEOUT_S -- CMD...     append a job (ID must be new)
    xq.py run  [JOBS]                      run pending jobs, JOBS at a time (default 1)
    xq.py status                           one line per job
    xq.py unlock ID                        clear the lock of a job whose runner died

State lives next to this file's parent: queue/jobs.jsonl (append-only job
list), queue/done.jsonl (append-only results), logs/ID.log. `run` skips every
ID already in done.jsonl, so it can be restarted after any interruption; a
job that was running when the runner died is simply run again. A job never
runs longer than its timeout (it is killed and recorded as `timeout`), and a
failed job is recorded and never retried automatically.
"""
import json, os, subprocess, sys, time, concurrent.futures as cf
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
JOBS, DONE, LOGS = f"{ROOT}/queue/jobs.jsonl", f"{ROOT}/queue/done.jsonl", f"{ROOT}/logs"
def load(path):
    return [json.loads(l) for l in open(path)] if os.path.exists(path) else []
def run_one(job):
    # One runner per job: a second `run` started meanwhile skips it. A lock
    # left by a runner that died is cleared with `xq.py unlock ID`.
    lock = f"{ROOT}/queue/running-{job['id']}.lock"
    try:
        os.close(os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY))
    except FileExistsError:
        return {"id": job["id"], "status": "skipped: running elsewhere"}
    try:
        return _run(job)
    finally:
        os.remove(lock)
def _run(job):
    began = time.time()
    with open(f"{LOGS}/{job['id']}.log", "w") as log:
        try:
            code = subprocess.run(job["cmd"], shell=True, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                  timeout=job["timeout"]).returncode
            status = "ok" if code == 0 else "failed"
        except subprocess.TimeoutExpired:
            code, status = None, "timeout"
    rec = {"id": job["id"], "status": status, "exit": code, "seconds": round(time.time() - began, 1),
           "ended": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
    with open(DONE, "a") as fh:
        fh.write(json.dumps(rec) + "\n")
    return rec
cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
if cmd == "add":
    jid, timeout = sys.argv[2], int(sys.argv[3])
    assert sys.argv[4] == "--" and jid not in {j["id"] for j in load(JOBS)}, "usage / duplicate id"
    with open(JOBS, "a") as fh:
        fh.write(json.dumps({"id": jid, "timeout": timeout, "cmd": " ".join(sys.argv[5:])}) + "\n")
elif cmd == "run":
    done = {d["id"] for d in load(DONE)}
    pending = [j for j in load(JOBS) if j["id"] not in done]
    with cf.ThreadPoolExecutor(int(sys.argv[2]) if len(sys.argv) > 2 else 1) as ex:
        for rec in ex.map(run_one, pending):
            print(json.dumps(rec), flush=True)
elif cmd == "unlock":
    os.remove(f"{ROOT}/queue/running-{sys.argv[2]}.lock")
else:
    done = {d["id"]: d for d in load(DONE)}
    for j in load(JOBS):
        d = done.get(j["id"])
        running = os.path.exists(f"{ROOT}/queue/running-{j['id']}.lock")
        print(j["id"], d["status"] if d else ("running" if running else "pending"), d.get("seconds", "") if d else "", sep="\t")
