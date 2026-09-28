"""Aggregate the Issue #243 ablation matrix from per-condition runs.jsonl.

Every condition directory holds one runs.jsonl. We keep only the match-489
records (the runner always adds `small-8x8` as a loader sanity fixture) and
emit:

1. the per-(condition, candidate) Markdown table — tier1-3, distinct series,
   agents that reached >=1 ball, zero-arrival agents (all / Patrol only,
   because Supply is structurally barred from collecting), fuel_short /
   illegal counts (AC: record how they surface);
2. a per-condition tier-sum block, which is what the verdict quotes;
3. the region-cause delta: region STAR vs the region-less same-walk loop,
   under every condition (owner's scope addition to #243).

Exit status is non-zero when the expected rows are missing, so a
half-finished matrix cannot be mistaken for a finished one.
"""

import json
import sys
from pathlib import Path

# (label, output dir) — keep in sync with run_matrix.sh
CONDITIONS = [
    ("A current", "A-current"),
    ("B fuel-inf", "B-fuelinf"),
    ("C oracle-refill", "C-oracle"),
    ("D1 kinds P,S", "D1-kinds-ps"),
    ("D2 kinds P,P,S", "D2-kinds-ppsp"),
    ("D3 kinds P,P,P", "D3-kinds-ppp"),
    ("E assign=patrol", "E-assign"),
]

# Short labels for the four candidates (owner's region-cause axis).
SHORT = {
    "multiloop:kmeans=4:seed=42:walk=astar": "R1 region-STAR",
    "multiloop:kmeans=4:seed=42:walk=astar:assign=patrol": "R1 region-STAR",
    "loop:iterations=32:initial=greedy:walk=astar": "R2 loop+astar",
    "loop:iterations=32:initial=greedy:walk=astar:assign=patrol": "R2 loop+astar",
    "loop:iterations=32:initial=greedy:operators=extended:walk=astar": "R3 loop ext+astar",
    "loop:iterations=32:initial=greedy:operators=extended:walk=astar:assign=patrol": "R3 loop ext+astar",
    "loop:iterations=32:initial=greedy": "R4 anchor",
    "loop:iterations=32:initial=greedy:assign=patrol": "R4 anchor",
}

ORDER = ["R1 region-STAR", "R2 loop+astar", "R3 loop ext+astar", "R4 anchor"]

MATCH_CASE = "match489"
N_CANDIDATES = 4


def rows(root: Path):
    for label, dirname in CONDITIONS:
        path = root / dirname / "runs.jsonl"
        if not path.exists():
            print(f"<!-- missing {path} -->", file=sys.stderr)
            continue
        for line in path.read_text().splitlines():
            rec = json.loads(line)
            if MATCH_CASE not in rec["case_id"]:
                continue
            yield label, rec


def short(candidate_id: str) -> str:
    return SHORT.get(candidate_id, candidate_id)


def patrol_zero(rec) -> int:
    counts = rec.get("agent_ball_counts") or []
    kinds = rec.get("agent_kinds") or []
    return sum(1 for c, k in zip(counts, kinds) if c == 0 and k == "Patrol")


def main(root_arg: str) -> int:
    root = Path(root_arg)

    print("### Ablation matrix (match 489)\n")
    print(
        "| condition | candidate | tier1 series | tier2 daily Σ | tier3 balls | "
        "agents ≥1 ball | zero-arrival (all / Patrol) | fuel_short | illegal | status |"
    )
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|---|")

    seen = set()
    per_condition = {}
    for label, rec in rows(root):
        counts = rec.get("agent_ball_counts") or []
        kinds = rec.get("agent_kinds") or []
        n_agents = len(counts) or len(kinds)
        agents_hit = sum(1 for c in counts if c >= 1)
        zero = sum(1 for c in counts if c == 0)
        n_patrol = sum(1 for k in kinds if k == "Patrol")
        composition = "".join("P" if k == "Patrol" else "S" for k in kinds)
        print(
            f"| {label} | `{short(rec['candidate_id'])}` | {rec['variety']} | "
            f"{rec['daily_variety_sum']} | {rec['total_balls']} | "
            f"{agents_hit}/{n_agents} | {zero} / {patrol_zero(rec)} of {n_patrol} | "
            f"{rec['fuel_short_count']} | {rec['illegal_action_count']} | "
            f"{rec['status']} ({composition}) |"
        )
        seen.add((label, rec["candidate_id"]))
        per_condition.setdefault(label, []).append(rec)

    print("\n### Per-condition tier sums (all 4 candidates)\n")
    print("| condition | Σ tier1 | Σ tier2 | Σ tier3 | Σ fuel_short | Σ zero-arrival (Patrol) |")
    print("|---|---:|---:|---:|---:|---:|")
    for label, _ in CONDITIONS:
        recs = per_condition.get(label, [])
        if not recs:
            continue
        print(
            f"| {label} | {sum(r['variety'] for r in recs)} | "
            f"{sum(r['daily_variety_sum'] for r in recs)} | "
            f"{sum(r['total_balls'] for r in recs)} | "
            f"{sum(r['fuel_short_count'] for r in recs)} | "
            f"{sum(patrol_zero(r) for r in recs)} |"
        )

    # Owner's scope addition: does kmeans region-splitting explain the
    # throughput shortfall? Compare region STAR against the region-less
    # loop that uses the *same* walk, per condition.
    print("\n### Region-cause axis (R1 region-STAR − R2 region-less, same walk)\n")
    print("| condition | Δ tier1 | Δ tier2 | Δ tier3 (balls) | R1 balls | R2 balls |")
    print("|---|---:|---:|---:|---:|---:|")
    for label, _ in CONDITIONS:
        recs = per_condition.get(label, [])
        by_short = {short(r["candidate_id"]): r for r in recs}
        r1, r2 = by_short.get("R1 region-STAR"), by_short.get("R2 loop+astar")
        if not (r1 and r2):
            continue
        print(
            f"| {label} | {r1['variety'] - r2['variety']} | "
            f"{r1['daily_variety_sum'] - r2['daily_variety_sum']} | "
            f"{r1['total_balls'] - r2['total_balls']} | "
            f"{r1['total_balls']} | {r2['total_balls']} |"
        )

    # Same question with the extended-ops control, to separate "regions"
    # from "tour quality".
    print("\n### R3 control (region-less + extended ops) vs R1\n")
    print("| condition | Δ tier1 | Δ tier2 | Δ tier3 (balls) | R1 balls | R3 balls |")
    print("|---|---:|---:|---:|---:|---:|")
    for label, _ in CONDITIONS:
        recs = per_condition.get(label, [])
        by_short = {short(r["candidate_id"]): r for r in recs}
        r1, r3 = by_short.get("R1 region-STAR"), by_short.get("R3 loop ext+astar")
        if not (r1 and r3):
            continue
        print(
            f"| {label} | {r1['variety'] - r3['variety']} | "
            f"{r1['daily_variety_sum'] - r3['daily_variety_sum']} | "
            f"{r1['total_balls'] - r3['total_balls']} | "
            f"{r1['total_balls']} | {r3['total_balls']} |"
        )

    expected = len(CONDITIONS) * N_CANDIDATES
    print(f"\n<!-- rows={len(seen)}/{expected} -->", file=sys.stderr)
    return 0 if len(seen) >= expected else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "target/abl"))
