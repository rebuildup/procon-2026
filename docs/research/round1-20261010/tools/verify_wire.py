"""Re-score a `round1_rollout` run with the independent Python replay (#403).

Reads the JSON Lines a rollout printed (its `day` records carry the wire
plan), plays the wire plans day after day under the recorded road state with
`r1sim.py`, and checks V/D/B and every day-end position and fuel against
what the Rust `MatchSim` reported.

    round1_rollout CASE EVENTS mode=solo candidate=... > run.jsonl
    python3 docs/research/round1-20261010/tools/verify_wire.py run.jsonl [--traffic smooth]
"""

from __future__ import annotations

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from r1sim import SUPPLY, Board, load_events, observed_days, simulate_day, traffic_of  # noqa: E402

RUN = "data/round1/20261010T044644Z-p28796"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("rollout")
    ap.add_argument("--run", default=RUN)
    ap.add_argument("--traffic", default="observed", choices=("observed", "smooth"))
    args = ap.parse_args()
    board = Board.load(os.path.join(args.run, "setting.json"))
    days = observed_days(load_events(os.path.join(args.run, "events.jsonl")))
    with open(args.rollout, encoding="utf-8") as fh:
        records = [json.loads(line) for line in fh if line.strip()]
    summary = records[-1]
    kinds = [1 if k == "S" else 0 for k in summary["kinds"].split(",")]
    pos, fuel = list(board.start), [board.fuel_cap] * len(kinds)
    brands: set[int] = set()
    total_d = total_b = 0
    ok = True
    for rec in records[:-1]:
        day = rec["day"]
        traffic = traffic_of(days[day]["state"]) if args.traffic == "observed" else {}
        res = simulate_day(board, day, kinds, pos, fuel, traffic, rec["wire"])
        got = res.brands()
        brands |= got
        total_d += len(got)
        total_b += res.balls()
        rust_end = [a["end"] for a in rec["agents"]]
        rust_fuel = [a["fuel_end"] for a, k in zip(rec["agents"], kinds) if k != SUPPLY]
        mine_fuel = [f for f, k in zip(res.fuel, kinds) if k != SUPPLY]
        same = (rust_end == res.pos and rust_fuel == mine_fuel
                and rec["D"] == len(got) and rec["B"] == res.balls() and not res.errors)
        ok &= same
        print(f"day {day}: D={len(got)} B={res.balls()} end={res.pos} fuel={mine_fuel} "
              f"matches_rust={same} errors={res.errors}")
        pos, fuel = res.pos, res.fuel
    same = (len(brands), total_d, total_b) == (summary["V"], summary["D"], summary["B"])
    print(f"TOTAL V={len(brands)} D={total_d} B={total_b} matches_rust={same and ok}")
    return 0 if same and ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
