"""Win/draw/loss of one candidate against another over `procon-eval` runs (#403).

    python3 docs/research/round1-20261010/tools/compare_runs.py RUNS.jsonl BASE_PREFIX NEW_PREFIX [-v]

Order is the official V -> D -> B; a run that is not `scored` counts as a
loss for its side and is listed. `small-8x8` is skipped (procon-eval needs
one bundled fixture on the command line; it is not part of any set here).
"""

from __future__ import annotations

import collections
import json
import sys


def main() -> int:
    path, base, new = sys.argv[1:4]
    verbose = "-v" in sys.argv
    by: dict[str, dict[str, dict]] = collections.defaultdict(dict)
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            r = json.loads(line)
            for side in (base, new):
                if r["candidate_id"].startswith(side):
                    by[r["case_id"]][side] = r
    tally = collections.Counter()
    total = {base: [0, 0, 0], new: [0, 0, 0]}
    slow = 0
    for case, sides in sorted(by.items()):
        if case == "fixture:small-8x8" or len(sides) < 2:
            continue
        key = {}
        for side, r in sides.items():
            ok = r["scored"] and not (r["illegal_action_count"] or r["fuel_short_count"]
                                      or r["rejected_response_count"] or r["day_budget_exceeded_days"])
            key[side] = (r["variety"], r["daily_variety_sum"], r["total_balls"]) if ok else None
            if ok:
                for i in range(3):
                    total[side][i] += key[side][i]
        if key[base] is None or key[new] is None:
            verdict = "invalid"
        else:
            verdict = "win" if key[new] > key[base] else "loss" if key[new] < key[base] else "draw"
        tally[verdict] += 1
        ms = max(sides[new]["daily_response_us"] or [0]) // 1000
        slow = max(slow, ms)
        if verbose or verdict in ("loss", "invalid"):
            print(f"{case:<44} {base}={key[base]} {new}={key[new]} {verdict} max_day={ms}ms "
                  f"{sides[new]['status']}")
    print(f"{new} vs {base}: W/D/L {tally['win']}/{tally['draw']}/{tally['loss']} invalid {tally['invalid']}"
          f" | sum V/D/B {new} {total[new]} {base} {total[base]} | slowest day {slow} ms")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
