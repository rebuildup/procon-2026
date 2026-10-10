"""Independent single-team day simulator for the round-one analysis (#403).

Re-implements the official tick timeline (guideline + Q&A Q6/Q7/Q10/Q22/Q27)
in plain Python so the live run can be re-played without the Rust engine and
cross-checked against it. Spot stock is per team (guideline 「在庫」), so the
only thing other teams change for us is the road state, which the server
publishes at day start and which stays fixed for the day. Given the observed
`traffics`, a day of our own team is therefore fully determined by our plan.

Wire encodings: cell 0 plain / 1 road / 2 mountain / 3 pond; action 0..5 =
upper-left, upper-right, right, lower-right, lower-left, left; a negative
action is a wait of that many steps; kind 0 patrol / 1 supply.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field

PLAIN, ROAD, MOUNTAIN, POND = 0, 1, 2, 3
PATROL, SUPPLY = 0, 1
ROAD_STEPS = {0: 1, 1: 2, 2: 4}


@dataclass
class Board:
    width: int
    height: int
    cells: list[int]
    spots: dict[int, tuple[int, int]]  # pos -> (brand, max stock)
    day_steps: list[int]
    fuel_cap: int
    start: list[int]
    players: int
    busy: int
    jammed: int

    @classmethod
    def load(cls, path: str) -> "Board":
        with open(path, encoding="utf-8") as fh:
            s = json.load(fh)
        m = s["map"]
        return cls(
            width=m["width"],
            height=m["height"],
            cells=[c for row in m["cells"] for c in row],
            spots={sp["pos"]: (sp["brand"], sp["stocks"]) for sp in s["spots"]},
            day_steps=list(s["daySteps"]),
            fuel_cap=s["fuelLimits"],
            start=list(s["agents"]),
            players=s["players"],
            busy=s["busyThreshold"],
            jammed=s["jammedThreshold"],
        )

    def neighbor(self, pos: int, d: int) -> int | None:
        row, col = divmod(pos, self.width)
        even = row % 2 == 0
        dc, dr = [
            (0, -1) if even else (-1, -1),
            (1, -1) if even else (0, -1),
            (1, 0),
            (1, 1) if even else (0, 1),
            (0, 1) if even else (-1, 1),
            (-1, 0),
        ][d]
        r, c = row + dr, col + dc
        if not (0 <= r < self.height and 0 <= c < self.width):
            return None
        p = r * self.width + c
        return None if self.cells[p] == POND else p

    def step_cost(self, pos: int, traffic: dict[int, int]) -> int:
        t = self.cells[pos]
        if t == PLAIN:
            return 2
        if t == MOUNTAIN:
            return 3
        return ROAD_STEPS[traffic.get(pos, 0)]

    def fuel_cost(self, pos: int) -> int:
        return 1 if self.cells[pos] == PLAIN else 2

    def brands(self) -> list[int]:
        return sorted({b for b, _ in self.spots.values()})


@dataclass
class DayResult:
    pos: list[int]
    fuel: list[int]
    picks: list[tuple[int, int, int, int]] = field(default_factory=list)  # tick, agent, pos, brand
    refuels: list[tuple[int, int, int, int]] = field(default_factory=list)  # tick, agent, pos, before
    arrivals: list[list[tuple[int, int]]] = field(default_factory=list)  # per agent (tick, pos)
    errors: list[str] = field(default_factory=list)
    used_steps: list[int] = field(default_factory=list)
    idle_tail: list[int] = field(default_factory=list)  # steps after the last move landed
    fuel_spent: list[int] = field(default_factory=list)
    road_stay: dict[int, int] = field(default_factory=dict)
    stock_left: dict[int, int] = field(default_factory=dict)

    def brands(self) -> set[int]:
        return {b for _, _, _, b in self.picks}

    def balls(self) -> int:
        return len(self.picks)


def simulate_day(
    board: Board,
    day: int,
    kinds: list[int],
    pos: list[int],
    fuel: list[int],
    traffic: dict[int, int],
    plans: list[list[int]],
) -> DayResult:
    n = len(kinds)
    limit = board.day_steps[day]
    pos, fuel = list(pos), list(fuel)
    stock = {p: s for p, (_, s) in board.spots.items()}
    picked: set[tuple[int, int]] = set()
    res = DayResult(pos=pos, fuel=fuel, arrivals=[[(0, p)] for p in pos])
    cursor = [0] * n
    pending: list[tuple[int, int | None, int] | None] = [None] * n  # due, target, fuel
    last_land = [0] * n
    spent = [0] * n
    used = [0] * n

    def start(i: int, now: int) -> None:
        while pending[i] is None and cursor[i] < len(plans[i]):
            code = plans[i][cursor[i]]
            cursor[i] += 1
            if code < 0:
                due = now - code
                if due > limit:
                    res.errors.append(f"day{day} agent{i}: wait overruns the day")
                    due = limit
                pending[i] = (due, None, 0)
                used[i] += -code
                return
            to = board.neighbor(pos[i], code)
            if to is None:
                res.errors.append(f"day{day} agent{i}: illegal move {code} from {pos[i]}")
                continue
            cost = board.step_cost(pos[i], traffic)
            if now + cost > limit:
                res.errors.append(f"day{day} agent{i}: move does not land inside the day")
                cursor[i] = len(plans[i])
                return
            pending[i] = (now + cost, to, board.fuel_cost(pos[i]))
            used[i] += cost
            return

    for tick in range(limit + 1):
        if tick > 0:
            for i in range(n):
                p = pending[i]
                if p is None or p[0] != tick:
                    continue
                pending[i] = None
                _, to, cost = p
                if to is None:
                    continue
                if kinds[i] == PATROL:
                    if fuel[i] < cost:
                        res.errors.append(f"day{day} agent{i}: fuel short at tick {tick}")
                        continue
                    fuel[i] -= cost
                    spent[i] += cost
                pos[i] = to
                last_land[i] = tick
                res.arrivals[i].append((tick, to))
            for i in range(n):
                if kinds[i] != PATROL:
                    continue
                p = pos[i]
                if p in stock and stock[p] > 0 and (i, p) not in picked:
                    stock[p] -= 1
                    picked.add((i, p))
                    res.picks.append((tick, i, p, board.spots[p][0]))
            supply_cells = {pos[i] for i in range(n) if kinds[i] == SUPPLY}
            for i in range(n):
                if kinds[i] == PATROL and pos[i] in supply_cells and fuel[i] < board.fuel_cap:
                    res.refuels.append((tick, i, pos[i], fuel[i]))
                    fuel[i] = board.fuel_cap
            for i in range(n):
                if board.cells[pos[i]] == ROAD:
                    res.road_stay[pos[i]] = res.road_stay.get(pos[i], 0) + 1
        if tick < limit:
            for i in range(n):
                if pending[i] is not None and pending[i][0] <= tick:
                    pending[i] = None
                if pending[i] is None:
                    start(i, tick)

    res.pos, res.fuel = pos, fuel
    res.used_steps = used
    res.idle_tail = [limit - t for t in last_land]
    res.fuel_spent = spent
    res.stock_left = stock
    return res


def load_events(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def observed_days(events: list[dict]) -> dict[int, dict]:
    """Per day: the observed `GET /state` snapshot and the submitted plan."""
    days: dict[int, dict] = {}
    for e in events:
        if e["type"] == "day_state":
            o = e["observed"]
            days.setdefault(o["day"], {})["state"] = o
        elif e["type"] == "plan":
            days.setdefault(e["day"], {})["plan"] = e["actions"]
    return days


def traffic_of(state: dict) -> dict[int, int]:
    return {t["pos"]: t["status"] for t in state["traffics"]}
