#!/usr/bin/env python3
"""Official-simple-server move-timing parity probe (Issue #342 timing check).

Settles whether the post-parity `MatchSim` tick timeline (a Move reserved at
tick `t` lands — and pays its departure fuel — at the reflection of
`t + terrain_step_cost(origin)`, Q6/Q10) reproduces the official simple
answer server's day-boundary position/fuel for identical wire plans.

One official-server instance per case. Records every HTTP round-trip and the
day-boundary `GET /` states into `<output-dir>/<case>/`, then replays the
same config + plans through the `timing_parity` diagnostic example
(`crates/bench/examples/timing_parity.rs`) and compares.

Usage:
  python3 scripts/campaign/timing_probe.py \
      --server-binary <procon-server-darwin-arm64> \
      --sim-binary <target/debug/examples/timing_parity> \
      --output-dir <empty dir>
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

# Terrain codes (試合設定JSONについて.md): 0 平地 / 1 道路 / 2 山地 / 3 池.
PLAIN, ROAD, MOUNTAIN = 0, 1, 2
# Wire action codes: 0..5 = official hex order (0 左上 .. 5 左). -N = wait N.
RIGHT, LEFT = 2, 5

WIDTH, HEIGHT = 12, 8
FUEL = 20


def base_cells() -> list[list[int]]:
    cells = [[PLAIN] * WIDTH for _ in range(HEIGHT)]
    return cells


# name -> dict(agent_starts, kinds, fuel_limits, cells_overrides, day0, day1, note)
# daySteps = [16, 16]. Weighted step sums are computed with the official
# table (平地 2 / 山 3 / 道路 1) and must equal 16 exactly.
CASES: list[dict] = [
    {
        "name": "V1_single_move",
        "note": "control: one plain move then waits. Move issued t0 lands t2.",
        "agent_starts": [0],
        "kinds": [0],
        "fuel_limits": FUEL,
        "cells": {},
        "day0": [[RIGHT, -14]],
        "day1": [[-16]],
    },
    {
        "name": "V2_seven_moves",
        "note": "7 consecutive plain moves (weights 7x2) + wait 2 = 16.",
        "agent_starts": [0],
        "kinds": [0],
        "fuel_limits": FUEL,
        "cells": {},
        "day0": [[RIGHT] * 7 + [-2]],
        "day1": [[-16]],
    },
    {
        "name": "V3_boundary_move",
        "note": "wait 14 then one plain move: issued t14, due t16 == day limit.",
        "agent_starts": [0],
        "kinds": [0],
        "fuel_limits": FUEL,
        "cells": {},
        "day0": [[-14, RIGHT]],
        "day1": [[-16]],
    },
    {
        "name": "V4_mountain_control",
        "note": "start on 山地; move to 平地 issued t0, due t3 (weight 3, fuel 2).",
        "agent_starts": [0],
        "kinds": [0],
        "fuel_limits": FUEL,
        "cells": {(0, 0): MOUNTAIN},
        "day0": [[RIGHT, -13]],
        "day1": [[-16]],
    },
    {
        "name": "V5_mountain_boundary",
        "note": "start on 山地; wait 13 then move: issued t13, due t16.",
        "agent_starts": [0],
        "kinds": [0],
        "fuel_limits": FUEL,
        "cells": {(0, 0): MOUNTAIN},
        "day0": [[-13, RIGHT]],
        "day1": [[-16]],
    },
    {
        "name": "V6_road_cadence",
        "note": "start on 道路(smooth): 8 moves (1 + 7x2) + wait 1 = 16; tests road step cost 1.",
        "agent_starts": [0],
        "kinds": [0],
        "fuel_limits": FUEL,
        "cells": {(0, 0): ROAD},
        "day0": [[RIGHT] * 8 + [-1]],
        "day1": [[-16]],
    },
    {
        "name": "V7_supply_refuel",
        "note": "patrol (fuel 4) drives 2 cells to a supply agent; Q22 same-cell refill.",
        "agent_starts": [0, 2],
        "kinds": [0, 1],
        "fuel_limits": 4,
        "cells": {},
        "day0": [[RIGHT, RIGHT, -12], [-16]],
        "day1": [[-16], [-16]],
    },
]


def run_case(
    binary: Path, sim_binary: Path, out_root: Path, case: dict, day_seconds: int
) -> dict:
    case_dir = out_root / case["name"]
    case_dir.mkdir(parents=True, exist_ok=True)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    cells = base_cells()
    for (row, col), terrain in case["cells"].items():
        cells[row][col] = terrain
    problem = {
        "spots": [{"brand": 0, "pos": WIDTH + 2, "stocks": 1}],
        "agentStarts": case["agent_starts"],
        "fuelLimits": case["fuel_limits"],
        "daySteps": [16, 16],
        "daySeconds": [day_seconds, day_seconds],
        "busyThreshold": 2,
        "jammedThreshold": 5,
        "width": WIDTH,
        "height": HEIGHT,
        "cells": cells,
        # Engine-side hints the simple server ignores; the diagnostic example
        # needs them for the wire setting conversion.
        "players": 1,
        "startsAt": 0,
    }
    cfg = {"problem": problem, "teams": [{"token": "local-test"}]}
    (case_dir / "config.json").write_text(json.dumps(cfg, indent=2) + "\n")
    plans = {"kinds": case["kinds"], "days": [case["day0"], case["day1"]]}
    (case_dir / "plans.json").write_text(json.dumps(plans, indent=2) + "\n")
    records: list[dict] = []

    def request(path: str, body=None):
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}{path}",
            data=None if body is None else json.dumps(body).encode(),
            headers={"Procon-Token": "local-test", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=5) as response:
                raw = response.read()
                status, value = response.status, (json.loads(raw) if raw else None)
        except urllib.error.HTTPError as error:
            raw = error.read().decode()
            try:
                status, value = error.code, json.loads(raw)
            except json.JSONDecodeError:
                status, value = error.code, raw
        records.append({"path": path, "body": body, "status": status, "response": value})
        return status, value

    def agents_of(state):
        if not isinstance(state, dict):
            return state
        return [
            {"pos": a.get("pos"), "fuel": a.get("fuel"), "kind": a.get("kind")}
            for a in state.get("agents", [])
        ]

    timeline = []
    with (case_dir / "server.log").open("w") as log:
        server = subprocess.Popen(
            [
                str(binary.resolve()),
                "-config",
                str(case_dir / "config.json"),
                "-addr",
                f"127.0.0.1:{port}",
                "-kind-deadline",
                "2s",
                "-match-start-delay",
                "1s",
            ],
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        try:
            time.sleep(0.3)
            request("/setting")
            request("/agent", case["kinds"])
            time.sleep(2.8)
            _, state0 = request("/")
            timeline.append({"when": "day0-start", "state": agents_of(state0)})
            _, answer0 = request("/", case["day0"])
            timeline.append(
                {"when": "day0-answer", "plan": case["day0"], "answer": answer0}
            )
            time.sleep(day_seconds + 1.5)
            _, state1 = request("/")
            timeline.append({"when": "day1-start", "state": agents_of(state1)})
            _, answer1 = request("/", case["day1"])
            timeline.append(
                {"when": "day1-answer", "plan": case["day1"], "answer": answer1}
            )
        finally:
            server.terminate()
            server.wait(timeout=5)

    # Replay the same config + plans through the post-parity MatchSim.
    sim_run = subprocess.run(
        [str(sim_binary.resolve()), str(case_dir / "config.json"), str(case_dir / "plans.json")],
        capture_output=True,
        text=True,
        timeout=120,
    )
    (case_dir / "sim_stdout.jsonl").write_text(sim_run.stdout)
    (case_dir / "sim_stderr.txt").write_text(sim_run.stderr)
    predictions = {}
    for line in sim_run.stdout.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("kind") == "day_start" and record.get("day") == 1:
            predictions = {
                "agents": [
                    {"pos": a["pos"], "fuel": a["fuel"], "kind": a["kind"]}
                    for a in record["agents"]
                ]
            }

    result = {
        "case": case["name"],
        "note": case["note"],
        "fuelLimits": case["fuel_limits"],
        "daySteps": [16, 16],
        "kinds": case["kinds"],
        "day0_plan": case["day0"],
        "timeline": timeline,
        "requests": records,
        "sim_prediction_day1": predictions,
        "sim_returncode": sim_run.returncode,
    }
    (case_dir / "result.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False) + "\n"
    )
    return result


def sanitize_logs(out_root: Path) -> None:
    """Strip local absolute paths from server logs before they are archived."""
    replacements = [
        (os.path.expanduser("~"), "<home>"),
        (str(out_root.resolve()), "<out>"),
    ]
    for log in out_root.glob("*/server.log"):
        text = log.read_text(errors="replace")
        for needle, token in replacements:
            text = text.replace(needle, token)
        log.write_text(text)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-binary", type=Path, required=True)
    parser.add_argument("--sim-binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--day-seconds", type=int, default=6)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=False)
    summary = []
    for case in CASES:
        result = run_case(
            args.server_binary, args.sim_binary, args.output_dir, case, args.day_seconds
        )
        row = {
            "case": result["case"],
            "note": result["note"],
            "day0_plan": result["day0_plan"],
        }
        for step in result["timeline"]:
            if step["when"] in ("day0-start", "day1-start"):
                row[step["when"]] = step["state"]
            elif step["when"] == "day0-answer":
                row["day0_answer"] = step["answer"]
        row["sim_prediction_day1"] = result["sim_prediction_day1"]
        row["sim_returncode"] = result["sim_returncode"]
        summary.append(row)
        print(json.dumps(row, ensure_ascii=False))
    sanitize_logs(args.output_dir)
    digest = hashlib.sha256(args.server_binary.read_bytes()).hexdigest()
    (args.output_dir / "summary.json").write_text(
        json.dumps(
            {
                "binary_sha256": digest,
                "sim_binary": args.sim_binary.name,
                "cases": summary,
            },
            indent=2,
            ensure_ascii=False,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
