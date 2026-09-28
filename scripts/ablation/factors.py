"""Factor decomposition for the Issue #243 verdict.

Reads the seven condition directories produced by run_matrix.sh and prints
the tier-sum contrasts that the verdict quotes, so the numbers in the Issue
comment are computed, not hand-copied.
"""

import json
import sys
from pathlib import Path

DATA = Path(__file__).resolve().parents[2] / "target" / "abl"
sys.path.insert(0, str(Path(__file__).parent))
from aggregate import CONDITIONS, short  # noqa: E402

FUEL_AWARE = ["R1 region-STAR", "R2 loop+astar", "R3 loop ext+astar"]
ALL4 = FUEL_AWARE + ["R4 anchor"]


def load(dirname):
    path = DATA / dirname / "runs.jsonl"
    return {
        short(r["candidate_id"]): r
        for r in map(json.loads, path.read_text().splitlines())
        if "match489" in r["case_id"]
    }


def tiers(cond, cands):
    return tuple(sum(cond[c][f] for c in cands) for f in ["variety", "daily_variety_sum", "total_balls"])


def main():
    C = {lab: load(d) for lab, d in CONDITIONS}

    print("=== tier sums (t1, t2, t3) ===")
    for lab, _ in CONDITIONS:
        print(f"  {lab:20s} all4={tiers(C[lab], ALL4)}  fuel-aware3={tiers(C[lab], FUEL_AWARE)}")

    g = lambda lab: tiers(C[lab], ALL4)
    gf = lambda lab: tiers(C[lab], FUEL_AWARE)

    print("\n=== contrasts (all4 / fuel-aware3) ===")
    print("FUEL     B-A :", tuple(b - a for a, b in zip(g("A current"), g("B fuel-inf"))), "/",
          tuple(b - a for a, b in zip(gf("A current"), gf("B fuel-inf"))))
    print("ORACLE   C-A :", tuple(b - a for a, b in zip(g("A current"), g("C oracle-refill"))), "/",
          tuple(b - a for a, b in zip(gf("A current"), gf("C oracle-refill"))))
    print("KINDS  D1-A :", tuple(b - a for a, b in zip(g("A current"), g("D1 kinds P,S"))))
    print("KINDS  D2-A :", tuple(b - a for a, b in zip(g("A current"), g("D2 kinds P,P,S"))), "/",
          tuple(b - a for a, b in zip(gf("A current"), gf("D2 kinds P,P,S"))))
    print("KINDS  D3-A :", tuple(b - a for a, b in zip(g("A current"), g("D3 kinds P,P,P"))), "/",
          tuple(b - a for a, b in zip(gf("A current"), gf("D3 kinds P,P,P"))))
    print("SUPPLY  A-E :", tuple(a - b for a, b in zip(g("A current"), g("E assign=patrol"))), "/",
          tuple(a - b for a, b in zip(gf("A current"), gf("E assign=patrol"))))

    print("\n=== per-candidate: supply-tour value (A - E), tier3 ===")
    for c in ALL4:
        print(f"  {c:24s} A={C['A current'][c]['total_balls']:4d}  "
              f"E={C['E assign=patrol'][c]['total_balls']:4d}  "
              f"delta={C['A current'][c]['total_balls'] - C['E assign=patrol'][c]['total_balls']:+4d}")

    print("\n=== region penalty (R1 - R2) per condition ===")
    for lab, _ in CONDITIONS:
        r1, r2 = C[lab]["R1 region-STAR"], C[lab]["R2 loop+astar"]
        print(f"  {lab:20s} t1={r1['variety'] - r2['variety']:+3d} "
              f"t2={r1['daily_variety_sum'] - r2['daily_variety_sum']:+4d} "
              f"t3={r1['total_balls'] - r2['total_balls']:+4d}")

    print("\n=== tier1 per condition ===")
    for lab, _ in CONDITIONS:
        print(f"  {lab:20s} " + "  ".join(f"{c.split()[0]}={C[lab][c]['variety']}" for c in ALL4))

    print("\n=== slot-4 balls (the kmeans=4 surplus agent) ===")
    for lab, _ in CONDITIONS:
        print(f"  {lab:20s} " + "  ".join(
            f"{c.split()[0]}={(C[lab][c]['agent_ball_counts'] or [None] * 5)[4]}" for c in FUEL_AWARE))

    print("\n=== fuel_short per candidate ===")
    for lab, _ in CONDITIONS:
        print(f"  {lab:20s} " + "  ".join(f"{c.split()[0]}={C[lab][c]['fuel_short_count']}" for c in ALL4))
    return 0


if __name__ == "__main__":
    sys.exit(main())
