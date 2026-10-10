"""F-series fuel decomposition from one procon-eval runs.jsonl (#433 / F3).

    python3 -I fseries_gain.py RUNS.jsonl

For every case, prints the (V, dV, B) of each candidate role and the §6.2
decomposition gains:
  gain(c)     = B(c+fuel_mix=on)  - B(c anchor)
  gain_no_mix = B(c+fuel_need=50) - B(c anchor)   (fuel_need alone)
  mix_effect  = gain - gain_no_mix                  (>0 synergy, <0 dilution)
Also flags dV losses vs the anchor (adopt-blocker per the batch protocol)
and max daily response time. Candidates are matched by role via substring
rules, so param reordering in the recorded id does not matter.
"""

from __future__ import annotations

import collections
import json
import sys


def role(candidate_id: str) -> str | None:
    if "iters" in candidate_id or "budget_ms" in candidate_id:
        if "chains=8" in candidate_id:
            side = "c8"
        elif "chains=4" in candidate_id:
            side = "c4"
        else:
            return None
        if "fuel_need" not in candidate_id:
            return f"{side}:anchor"
        if "fuel_mix" in candidate_id:
            return f"{side}:mix"
        return f"{side}:fuel"
    return None


def main() -> int:
    path = sys.argv[1]
    by: dict[str, dict[str, dict]] = collections.defaultdict(dict)
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            r = json.loads(line)
            k = role(r["candidate_id"])
            if k:
                by[r["case_id"]][k] = r
    total_gain = collections.Counter()
    dv_loss = []
    for case, sides in sorted(by.items()):
        anchors = [s for s in sides if s.endswith(":anchor")]
        if not anchors:
            continue
        row = []
        for side in sorted(sides):
            r = sides[side]
            ok = r["scored"] and not (r["illegal_action_count"] or r["fuel_short_count"]
                                      or r["rejected_response_count"]
                                      or r["day_budget_exceeded_days"])
            key = (r["variety"], r["daily_variety_sum"], r["total_balls"]) if ok else None
            max_day = max(r["daily_response_us"] or [0]) // 1000
            row.append((side, key, max_day, "OK" if ok else r["status"]))
        print(f"== {case}")
        for side, key, max_day, st in row:
            print(f"   {side:<12} V/D/B={key} maxday={max_day}ms {st}")
        for anchor in anchors:
            c = anchor[:2]
            a = sides[anchor]
            def kv(side):
                if side not in sides:
                    return None
                r = sides[side]
                return (r["variety"], r["daily_variety_sum"], r["total_balls"])
            ak = kv(anchor)
            for other, label in ((f"{c}:fuel", "fuel"), (f"{c}:mix", "mix")):
                ok_side = kv(other)
                if ok_side is None or ak is None:
                    continue
                gain = ok_side[2] - ak[2]
                dv = ok_side[1] - ak[1]
                tag = "mix" if label == "mix" else "fuel-only"
                total_gain[tag] += gain
                if dv < 0:
                    dv_loss.append((case, c, label, dv, ok_side[2] - ak[2]))
        print()
    print(f"TOTAL B gain: fuel-only {total_gain['fuel-only']}, mix {total_gain['mix']}")
    for case, c, label, dv, db in dv_loss:
        print(f"DV-LOSS {case} {c} {label}: dV {dv:+d} (B {db:+d})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())