"""Driver for the round-one counterfactual rollouts (#403).

Runs `crates/bench/examples/round1_rollout.rs` one configuration at a time
(never in parallel: several candidates stop on the wall clock) and appends
each run's `summary` record to `results/<experiment>.jsonl`.

    cargo build --release --locked -p bench --example round1_rollout
    python3 docs/research/round1-20261010/tools/rollouts.py <experiment> [--case PATH --events PATH]
    python3 docs/research/round1-20261010/tools/rollouts.py --list
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
BIN = os.path.join(ROOT, "target", "release", "examples", "round1_rollout")
CASE = "data/round1/derived/round1-match3.case.json"
EVENTS = "data/round1/20261010T044644Z-p28796/events.jsonl"
SEEDS = [1, 2, 3, 7, 11, 12, 42, 2026]
MORE_SEEDS = SEEDS + [5, 8, 13, 21, 34, 55, 89, 144]
LIVE = "fleet:budget=official:budget_ms=10000:adapt=off"


def fleet(iters: int, seed: int, chains: int = 4) -> str:
    return f"fleet:budget=official:iters={iters}:chains={chains}:seed={seed}"


def portfolio(seeds: list[int], iters: int = 60000) -> str:
    return "|".join(fleet(iters, s) for s in seeds)


def experiments() -> dict[str, list[dict]]:
    exp: dict[str, list[dict]] = {}
    exp["replay"] = [{"label": "submitted", "mode": "submitted"}]
    # The live configuration, repeated: it stops on the wall clock.
    exp["live-config"] = [
        {"label": f"live-{tr}-{i}", "mode": "solo", "traffic": tr, "candidate": LIVE}
        for tr in ("observed", "smooth")
        for i in range(3)
    ]
    exp["seeds"] = [
        {"label": f"seed{s}-{tr}", "mode": "solo", "traffic": tr, "candidate": fleet(60000, s)}
        for tr in ("observed", "smooth")
        for s in MORE_SEEDS
    ]
    exp["kinds"] = [
        {"label": f"{k}-{tr}-seed{s}", "mode": "solo", "traffic": tr, "kinds": k,
         "candidate": fleet(60000, s)}
        for k in ("S,P,P,P", "P,S,P,P", "P,P,S,P", "P,P,P,S", "P,P,P,P", "S,S,P,P", "S,P,S,P")
        for tr in ("observed", "smooth")
        for s in SEEDS
    ]
    exp["budget"] = [
        {"label": f"{ms}ms-{i}", "mode": "solo", "traffic": "observed",
         "candidate": f"fleet:budget=official:budget_ms={ms}:adapt=off"}
        for ms in (250, 1000, 2000, 5000, 10000, 20000, 40000)
        for i in range(3)
    ]
    # Issue #409: chain count above four under the live budget. The harness
    # has no clock (remaining_ms = u32::MAX), so the live path runs only
    # because budget_ms is set; `chains=<N>` now widens the live cap to N.
    exp["issue409"] = [
        {"label": f"c{c}-s{s}", "mode": "solo", "traffic": "observed",
         "candidate": f"fleet:budget=official:budget_ms=10000:adapt=off:chains={c}:seed={s}"}
        for c in (4, 8, 12, 16)
        for s in (1, 2, 3, 7)
    ]
    exp["iters"] = [
        {"label": f"iters{it}-seed{s}", "mode": "solo", "traffic": "observed",
         "candidate": fleet(it, s)}
        for it in (8000, 20000, 60000, 240000, 960000)
        for s in SEEDS
    ]
    cont = "fleet:budget=official:iters=20000"
    exp["portfolio"] = [
        {"label": f"p{n}-h{h}-{fc}-{tr}", "mode": "portfolio", "traffic": tr, "forecast": fc,
         "horizon": h, "options": portfolio(MORE_SEEDS[:n]), "cont": cont}
        for tr in ("observed", "smooth")
        for n in (4, 8, 16)
        for h in (0, 1, 2, 99)
        for fc in ("persist", "smooth")
        if not (tr == "smooth" and fc == "persist" and h > 0 and False)
    ]
    exp["portfolio-cont"] = [
        {"label": f"p8-cont-{name}-{tr}", "mode": "portfolio", "traffic": tr, "horizon": 99,
         "options": portfolio(SEEDS), "cont": c}
        for tr in ("observed", "smooth")
        for name, c in (("8k", "fleet:budget=official:iters=8000"),
                        ("20k", "fleet:budget=official:iters=20000"),
                        ("60k4", "fleet:budget=official:iters=60000:chains=4"),
                        ("240k4", "fleet:budget=official:iters=240000:chains=4"))
    ]
    return exp


def run(cfg: dict, case: str, events: str) -> dict:
    cmd = [BIN, case, events] + [f"{k}={v}" for k, v in cfg.items()]
    started = time.time()
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        return {"kind": "error", "label": cfg.get("label"), "stderr": proc.stderr[-400:]}
    days = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
    summary = days.pop()
    summary["wall_s"] = round(time.time() - started, 2)
    summary["load1"] = os.getloadavg()[0]
    summary["command"] = " ".join(os.path.relpath(c, ROOT) if os.path.isabs(c) else c for c in cmd)
    summary["end"] = [[a["end"], a["fuel_end"]] for a in days[-1]["agents"]]
    summary["chosen"] = [
        next((k for k, o in enumerate(d["options"]) if o["chosen"]), None) for d in days
    ]
    return summary


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("experiment", nargs="?")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--case", default=CASE)
    ap.add_argument("--events", default=EVENTS)
    ap.add_argument("--out", default=os.path.join(HERE, "..", "results"))
    ap.add_argument("--only", default="")
    args = ap.parse_args()
    exp = experiments()
    if args.list or not args.experiment:
        for name, cfgs in exp.items():
            print(f"{name}: {len(cfgs)} runs")
        return 0
    os.makedirs(args.out, exist_ok=True)
    path = os.path.join(args.out, f"{args.experiment}.jsonl")
    done = set()
    if os.path.exists(path):
        with open(path, encoding="utf-8") as fh:
            done = {json.loads(line).get("label") for line in fh if line.strip()}
    with open(path, "a", encoding="utf-8") as fh:
        for cfg in exp[args.experiment]:
            if cfg["label"] in done or args.only not in cfg["label"]:
                continue
            rec = run(cfg, args.case, args.events)
            fh.write(json.dumps(rec, ensure_ascii=False) + "\n")
            fh.flush()
            if rec["kind"] == "error":
                print("ERROR", rec, file=sys.stderr)
                continue
            bad = rec["illegal_action_count"] + rec["fuel_short_count"] + rec["rejected_response_count"]
            print(f"{rec['label']:<28} {rec['kinds']} V{rec['V']} D{rec['D']} B{rec['B']} "
                  f"{rec['daily_D']} {rec['daily_B']} max_day_ms={max(rec['decide_ms'])}"
                  f"{' INVALID' if bad else ''}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
