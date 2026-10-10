"""Re-play the five submitted day plans of the round-one live run (#403).

Each day starts from the observed `GET /state` (positions, fuel, road state)
and plays the plan we actually submitted. The end-of-day position and fuel
are compared with the next day's observation, and the summed V/D/B with the
event-display scoreboard (V10 / D44 / B51).

    python3 docs/research/round1-20261010/tools/replay_live.py [--json OUT]
"""

from __future__ import annotations

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from r1sim import Board, load_events, observed_days, simulate_day, traffic_of  # noqa: E402

RUN = "data/round1/20261010T044644Z-p28796"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--run", default=RUN)
    ap.add_argument("--json")
    args = ap.parse_args()

    board = Board.load(os.path.join(args.run, "setting.json"))
    days = observed_days(load_events(os.path.join(args.run, "events.jsonl")))
    brands = board.brands()
    all_brands: set[int] = set()
    total_d = total_b = 0
    out = []
    ok = True
    for day in sorted(days):
        st = days[day]["state"]
        kinds = [a["kind"] for a in st["agents"]]
        pos = [a["pos"] for a in st["agents"]]
        fuel = [a["fuel"] for a in st["agents"]]
        res = simulate_day(board, day, kinds, pos, fuel, traffic_of(st), days[day]["plan"])
        got = res.brands()
        all_brands |= got
        total_d += len(got)
        total_b += res.balls()
        nxt = days.get(day + 1, {}).get("state")
        match = None
        if nxt is not None:
            match = [a["pos"] for a in nxt["agents"]] == res.pos and [
                a["fuel"] for a in nxt["agents"]
            ] == res.fuel
            ok &= match
        ok &= not res.errors
        missed = [b for b in brands if b not in got]
        print(f"== day {day} steps={board.day_steps[day]} D={len(got)} B={res.balls()} missed={missed}"
              f" next-day pos/fuel match={match} errors={res.errors}")
        for i in range(len(kinds)):
            picks = [(t, p, b) for t, a, p, b in res.picks if a == i]
            refuels = [(t, p, before) for t, a, p, before in res.refuels if a == i]
            role = "S" if kinds[i] == 1 else "P"
            print(f"   a{i}{role} {pos[i]:>3}->{res.pos[i]:>3} fuel {fuel[i]:>2}->{res.fuel[i]:>2}"
                  f" spent={res.fuel_spent[i]:>2} idle_tail={res.idle_tail[i]:>2}"
                  f" picks(tick,pos,brand)={picks} refuels(tick,pos,before)={refuels}")
        left = {p: s for p, s in sorted(res.stock_left.items()) if s > 0}
        print(f"   stock left (pos:balls) {left}")
        out.append({
            "day": day, "D": len(got), "B": res.balls(), "missed_brands": missed,
            "end_pos": res.pos, "end_fuel": res.fuel, "next_day_match": match,
            "picks": res.picks, "refuels": res.refuels, "idle_tail": res.idle_tail,
            "fuel_spent": res.fuel_spent, "stock_left": left, "errors": res.errors,
            "arrivals": res.arrivals,
        })
    print(f"TOTAL V={len(all_brands)} D={total_d} B={total_b} consistent={ok}")
    if args.json:
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump({"V": len(all_brands), "D": total_d, "B": total_b, "days": out}, fh, indent=1)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
