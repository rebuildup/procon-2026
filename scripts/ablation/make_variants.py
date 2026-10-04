"""Build the counterfactual case variants for the Issue #243 ablation.

Input : target/abl/match489-case.json   (imported via `procon arena import-case`)
Output: target/abl/match489-fuelinf.json (same board, fuelLimits raised)

The fuel-infinity variant rewrites ONLY `fuelLimits` (and the case identity,
so the two boards cannot be confused when records are merged later).
Everything else — map, spots, agents, daySteps, daySeconds, thresholds — is
byte-identical.
"""

import json
import sys

FUEL_INF = 100000  # non-binding: any patrol can walk every step of every day


def main(src: str, dst: str) -> int:
    with open(src, encoding="utf-8") as fh:
        case = json.load(fh)
    before = case["fuelLimits"]
    case["fuelLimits"] = FUEL_INF
    # Distinct identity so a merged runs.jsonl can never confuse the two.
    case["case_id"] = "fixture:match489-fuelinf"
    if isinstance(case.get("source"), dict) and "name" in case["source"]:
        case["source"]["name"] = "match489-fuelinf"
    with open(dst, "w", encoding="utf-8") as fh:
        json.dump(case, fh, indent=2, ensure_ascii=False)
    print(f"fuelLimits {before} -> {case['fuelLimits']}, case_id={case['case_id']} -> {dst}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
