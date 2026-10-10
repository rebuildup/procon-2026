"""Per-day reachability bounds for the round-one board (#403).

For one day and one set of patrol starts, enumerate what each patrol can
collect inside the day and combine the patrols. Three fuel models:

- `free`   : fuel ignored. An upper bound for any plan with any supply use.
- `tank`   : each patrol lives on the fuel it starts the day with (no supply).
             Constructive: such a day needs no supply at all.
- `escort` : one patrol has unlimited fuel (a supply riding with it all day),
             the others live on their tanks; the best choice of escort.
             Neither a bound nor always constructive (a supply can also serve
             two patrols, or reach its patrol late) — shown as a reference.

Legs use every time/fuel trade-off between two spots (not only the fastest
path), under the road state published for that day.

    python3 docs/research/round1-20261010/tools/bounds.py
"""

from __future__ import annotations

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from r1sim import PATROL, Board, load_events, observed_days, traffic_of  # noqa: E402

RUN = "data/round1/20261010T044644Z-p28796"
INF = 10**9


def legs(board: Board, traffic: dict[int, int], src: int, limit: int) -> dict[int, list[tuple[int, int]]]:
    """Pareto (time, fuel) fronts from `src` to every spot, time <= limit."""
    n = len(board.cells)
    best = [[INF] * n for _ in range(limit + 1)]
    best[0][src] = 0
    for t in range(limit):
        row = best[t]
        for u in range(n):
            f = row[u]
            if f >= INF:
                continue
            dt = board.step_cost(u, traffic)
            if t + dt > limit:
                continue
            nf = f + board.fuel_cost(u)
            for d in range(6):
                v = board.neighbor(u, d)
                if v is not None and nf < best[t + dt][v]:
                    best[t + dt][v] = nf
    out: dict[int, list[tuple[int, int]]] = {}
    for spot in board.spots:
        front, low = [], INF
        for t in range(limit + 1):
            if best[t][spot] < low:
                low = best[t][spot]
                front.append((t, low))
        out[spot] = front
    return out


class Day:
    def __init__(self, board: Board, traffic: dict[int, int], limit: int):
        self.board, self.traffic, self.limit = board, traffic, limit
        self.cache: dict[int, dict[int, list[tuple[int, int]]]] = {}
        self.brand_bit = {b: i for i, b in enumerate(board.brands())}
        self.spot_bit = {p: i for i, p in enumerate(sorted(board.spots))}

    def legs(self, src: int) -> dict[int, list[tuple[int, int]]]:
        if src not in self.cache:
            self.cache[src] = legs(self.board, self.traffic, src, self.limit)
        return self.cache[src]

    def reach(self, start: int, fuel: int | None, by_spot: bool = False) -> set[int]:
        """Masks (brands, or spots with `by_spot`) one patrol can collect."""
        bit = (lambda p: 1 << self.spot_bit[p]) if by_spot else (
            lambda p: 1 << self.brand_bit[self.board.spots[p][0]])
        cap = INF if fuel is None else fuel
        mask0 = bit(start) if start in self.board.spots else 0
        # (node, mask) -> Pareto list of (time, fuel used)
        labels: dict[tuple[int, int], list[tuple[int, int]]] = {(start, mask0): [(0, 0)]}
        stack = [(start, mask0, 0, 0)]
        masks = {mask0}
        while stack:
            node, mask, t, f = stack.pop()
            if (t, f) not in labels.get((node, mask), ()):
                continue
            for spot, front in self.legs(node).items():
                b = bit(spot)
                if mask & b:
                    continue
                for dt, df in front:
                    nt, nf = t + dt, f + df
                    if nt > self.limit or nf > cap:
                        continue
                    key = (spot, mask | b)
                    cur = labels.setdefault(key, [])
                    if any(ct <= nt and cf <= nf for ct, cf in cur):
                        continue
                    cur[:] = [(ct, cf) for ct, cf in cur if not (nt <= ct and nf <= cf)]
                    cur.append((nt, nf))
                    masks.add(mask | b)
                    stack.append((spot, mask | b, nt, nf))
        return masks


def maximal(masks: set[int]) -> list[int]:
    ordered = sorted(masks, key=lambda m: -bin(m).count("1"))
    keep: list[int] = []
    for m in ordered:
        if not any(m & k == m for k in keep):
            keep.append(m)
    return keep


def best_union(sets: list[list[int]]) -> int:
    acc = {0}
    for masks in sets:
        acc = set(maximal({a | m for a in acc for m in masks}))
    return max(bin(m).count("1") for m in acc)


def day_bounds(board: Board, day: int, traffic: dict[int, int], starts: list[int],
               fuels: list[int]) -> dict[str, int]:
    model = Day(board, traffic, board.day_steps[day])
    free = [maximal(model.reach(s, None)) for s in starts]
    tank = [maximal(model.reach(s, f)) for s, f in zip(starts, fuels)]
    out = {"free": best_union(free), "tank": best_union(tank)}
    out["escort"] = max(
        best_union([free[i] if i == e else tank[i] for i in range(len(starts))])
        for e in range(len(starts))
    )
    # Balls: distinct spots per patrol, stock ignored except the per-spot cap.
    spots = sorted(board.spots)
    per_patrol = [model.reach(s, None, by_spot=True) for s in starts]
    out["balls_free_per_patrol"] = [max(bin(m).count("1") for m in ms) for ms in per_patrol]
    # Stock-aware team bound: a spot gives min(stock, patrols that reach it).
    out["balls_free"] = sum(
        min(board.spots[p][1],
            sum(any(m >> i & 1 for m in ms) for ms in per_patrol))
        for i, p in enumerate(spots)
    )
    out["balls_free"] = min(out["balls_free"], sum(out["balls_free_per_patrol"]))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--run", default=RUN)
    ap.add_argument("--json")
    args = ap.parse_args()
    board = Board.load(os.path.join(args.run, "setting.json"))
    days = observed_days(load_events(os.path.join(args.run, "events.jsonl")))
    rows = []
    print("day steps | live starts (pos/fuel)            | D free tank escort | B free (per patrol)")
    for day in sorted(days):
        st = days[day]["state"]
        patrols = [a for a in st["agents"] if a["kind"] == PATROL]
        starts = [a["pos"] for a in patrols]
        fuels = [a["fuel"] for a in patrols]
        b = day_bounds(board, day, traffic_of(st), starts, fuels)
        rows.append({"day": day, "starts": starts, "fuels": fuels, **b})
        print(f"{day:>3} {board.day_steps[day]:>5} | {str(list(zip(starts, fuels))):<34}| "
              f"{b['free']:>6} {b['tank']:>4} {b['escort']:>6} | {b['balls_free']} {b['balls_free_per_patrol']}")
    if args.json:
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump(rows, fh, indent=1)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
