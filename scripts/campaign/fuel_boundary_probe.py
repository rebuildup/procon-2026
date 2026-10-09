#!/usr/bin/env python3
"""Official-simple-server fuel executability probe (Issue #342 P0-C).

Settles whether a patrol Move is permitted when `fuel` covers only the
DEPARTURE cell's fuel cost while the ARRIVAL cell costs more — the case the
conservative `rules::move_fuel` (afford `max(origin, destination)`) and the
spec-text reading (afford + charge origin only) disagree on.

One official-server instance per case. Records every HTTP round-trip and the
day-boundary `GET /` states into `<output-dir>/<case>/`.

Usage:
  python3 scripts/campaign/fuel_boundary_probe.py \
      --server-binary <procon-server-darwin-arm64|linux-amd64|...> \
      --output-dir <empty dir>
"""
from __future__ import annotations

import argparse
import hashlib
import json
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

# Terrain codes (試合設定JSONについて.md): 0 平地 / 1 道路 / 2 山地 / 3 池.
PLAIN, ROAD, MOUNTAIN, POND = 0, 1, 2, 3
# Answer codes: 0..5 = 上左,上右,右,右下,左下,左. Negative -N = wait N steps.
DIR_RIGHT, DIR_LOWER_RIGHT, DIR_LEFT = 2, 3, 5


def base_cells() -> list[list[int]]:
    """8x8 平地 with 山地 at pos 1 = (x=1, y=0). Agent starts on pos 0 (平地)."""
    cells = [[PLAIN] * 8 for _ in range(8)]
    cells[0][1] = MOUNTAIN
    return cells


# name -> (agent_start, fuel_limits, day0_plan, expected_revision_sign)
# daySteps is fixed at 16; a 平地 move costs 2 steps, a 山地 move 3.
# Plans pad the remainder with a single wait-run so Σ weighted == 16.
CASES: list[dict] = [
    {
        "name": "A_plain1_to_mountain",
        "note": "DECISIVE: fuel=1, 平地->山地. spec-text ACCEPT / conservative REJECT.",
        "agent_starts": [0],
        "fuel_limits": 1,
        "day0_plan": [[2, -14]],
    },
    {
        "name": "B_mountain1_to_plain",
        "note": "control: fuel=1, 山地->平地. Departure cost 2 > 1 -> REJECT under both surviving rules.",
        "agent_starts": [1],
        "fuel_limits": 1,
        "day0_plan": [[5, -13]],
    },
    {
        "name": "C_plain2_to_mountain",
        "note": "fuel=2, 平地->山地. ACCEPT everywhere; fuel after (1 vs 0) settles the CHARGE rule.",
        "agent_starts": [0],
        "fuel_limits": 2,
        "day0_plan": [[2, -14]],
    },
    {
        "name": "D_plain1_to_plain",
        "note": "fuel=1, 平地->平地. ACCEPT everywhere (LowerRight from (0,0) even row -> (1,1)).",
        "agent_starts": [0],
        "fuel_limits": 1,
        "day0_plan": [[3, -14]],
    },
    {
        "name": "A2_plain0_to_mountain",
        "note": "fuel=0, 平地->山地. REJECT: cannot pay even the departure cost.",
        "agent_starts": [0],
        "fuel_limits": 0,
        "day0_plan": [[2, -14]],
    },
    {
        "name": "E_step_mismatch",
        "note": "invalid-response: Σ weighted 2 != daySteps 16 -> whole response REJECT.",
        "agent_starts": [0],
        "fuel_limits": 10,
        "day0_plan": [[2]],
    },
    {
        "name": "F_wait_only",
        "note": "wait control: [-16] ACCEPT.",
        "agent_starts": [0],
        "fuel_limits": 10,
        "day0_plan": [[-16]],
    },
]


def run_case(binary: Path, out_root: Path, case: dict, day_seconds: int) -> dict:
    case_dir = out_root / case["name"]
    case_dir.mkdir(parents=True, exist_ok=True)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    cells = base_cells()
    height, width = len(cells), len(cells[0])
    cfg = {
        "problem": {
            "spots": [{"brand": 0, "pos": width + 2, "stocks": 2}],
            "agentStarts": case["agent_starts"],
            "fuelLimits": case["fuel_limits"],
            "daySteps": [16, 16],
            "daySeconds": [day_seconds, day_seconds],
            "busyThreshold": 2,
            "jammedThreshold": 5,
            "width": width,
            "height": height,
            "cells": cells,
        },
        "teams": [{"token": "local-test"}],
    }
    (case_dir / "config.json").write_text(json.dumps(cfg, indent=2) + "\n")
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
            request("/agent", [0])  # kind 0 = 巡回車 (finite fuel in GET /)
            time.sleep(2.8)
            _, state0 = request("/")
            timeline.append({"when": "day0-start", "state": state0})
            _, answer = request("/", case["day0_plan"])
            timeline.append({"when": "day0-answer", "plan": case["day0_plan"], "answer": answer})
            time.sleep(day_seconds + 1.5)
            _, state1 = request("/")
            timeline.append({"when": "day1-start", "state": state1})
            _, answer1 = request("/", [[-16]])
            timeline.append({"when": "day1-answer", "plan": [[-16]], "answer": answer1})
        finally:
            server.terminate()
            server.wait(timeout=5)

    result = {
        "case": case["name"],
        "note": case["note"],
        "fuelLimits": case["fuel_limits"],
        "daySteps": [16, 16],
        "kinds": [0],
        "timeline": timeline,
        "requests": records,
    }
    (case_dir / "result.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--day-seconds", type=int, default=6)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=False)
    summary = []
    for case in CASES:
        result = run_case(args.server_binary, args.output_dir, case, args.day_seconds)
        row = {"case": result["case"], "note": result["note"]}
        for step in result["timeline"]:
            if step["when"].endswith("answer"):
                row[step["when"]] = step["answer"]
            elif step["when"].endswith("start") and isinstance(step.get("state"), dict):
                agents = step["state"].get("agents", [])
                row[step["when"]] = [
                    {"pos": a.get("pos"), "fuel": a.get("fuel"), "kind": a.get("kind")}
                    for a in agents
                ]
        summary.append(row)
        print(json.dumps(row, ensure_ascii=False))
    digest = hashlib.sha256(args.server_binary.read_bytes()).hexdigest()
    (args.output_dir / "summary.json").write_text(
        json.dumps({"binary_sha256": digest, "cases": summary}, indent=2, ensure_ascii=False) + "\n"
    )


if __name__ == "__main__":
    main()
