#!/usr/bin/env python3
"""Resumable batch driver for the Windows/WSL long-run solver campaign.

Design constraints (see docs/evaluation.md):

* ``procon-eval`` writes ``runs.jsonl`` only when a batch *finishes*, so a
  killed process loses that batch.  Batches are therefore small (one archetype
  slice or the fixture set) and are the unit of durability: a finished batch
  whose ``runs.jsonl`` holds exactly ``candidates x cases`` lines is skipped on
  re-run, which is what makes the whole campaign interruptible.
* Every batch records the command, source SHA, jobs, wall clock and the exact
  case / candidate lists in ``batch.json`` so a fresh agent can resume without
  this conversation.
* Nothing here scores anything.  Ranking is ``procon-eval``'s own
  ``aggregate.json`` (official lexicographic tuple); cross-batch merging is
  ``crates/bench/examples/eval_merge.rs``.

    python3 scripts/longrun_eval.py --group A --jobs 8
    python3 scripts/longrun_eval.py --status
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVAL = ROOT / "target" / "release" / "procon-eval"
OUT_ROOT = ROOT / "target" / "longrun"

CASE_FILES = [
    "crates/bench/tests/fixtures/arena-701.json",
    "docs/evidence/campaign-311/baseline-312/import-live-match.json",
]

FIXTURES = "small-8x8,medium-16x16,mountain-heavy,road-heavy,road-heavy-v2,series-few,series-many,large-32x32,openapi-sample"
ANCHOR_FIXTURE = "small-8x8"

ARCHES = [
    "open",
    "road-heavy",
    "mountain-heavy",
    "many-series",
    "few-series",
    "sparse-spots",
    "dense-spots",
    "pond-obstacle",
]

# Exploration seeds. Fixed up-front, never mixed with the holdout set below.
EXPLORE_SEEDS = [1, 2, 3, 4, 5, 6, 7, 8]
# Selection/holdout seeds. Only used for confirmation batches, never for
# tuning. Declared here so the split is visible in the durable manifest.
HOLDOUT_SEEDS = [101, 102, 103, 104, 105, 106, 107, 108]

# Candidate tiers.
#   TIER_CORE  — broad screening, runs over the whole corpus.
#   TIER_HEAVY — wall-clock / trial-heavy probes, runs over a case subset only
#                (they dominate the budget; a 10-day board at budget_ms=15000
#                alone costs 150 s of decide() time).
TIER_CORE = [
    "coverage:budget=official:park=none",
    "coverage:budget=official:park=far",
    "coverage:budget=official:park=far:fuelaware=true",
    "anytime:budget_ms=0:seed=42:budget=official",
    "anytime:budget_ms=1000:seed=42:budget=official",
    "search:trials=16:budget=official",
    "search:trials=16:park=far:budget=official",
    "search:trials=16:lookahead=next:budget=official",
    "multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official",
    "loop:iterations=128:initial=greedy:walk=astar:assign=all:budget=official",
    "astar:policy=step:budget=official",
]
TIER_HEAVY = [
    "coverage:budget=official:park=none",
    "coverage:budget=official:park=far",
    "anytime:budget_ms=0:seed=42:budget=official",
    "anytime:budget_ms=1000:seed=42:budget=official",
    "anytime:budget_ms=5000:seed=42:budget=official",
    "anytime:budget_ms=15000:seed=42:budget=official",
    "search:trials=64:budget=official",
    "search:trials=64:park=far:budget=official",
]
BASELINE = "coverage:budget=official:park=none"


def gen_specs_for(arch: str, seeds: list[int]) -> list[str]:
    """All generated selections belonging to one archetype."""
    specs: list[str] = []
    for s in seeds:
        specs.append(f"{arch}:seed={s}:size=medium:days=3")
    for s in seeds[:4]:
        specs.append(f"{arch}:seed={s}:size=large:days=3")
    # Fuel-stress axis (#204) on the archetypes where fuel actually binds.
    if arch in ("road-heavy", "open", "many-series", "dense-spots"):
        for s in seeds[:3]:
            specs.append(f"{arch}:seed={s}:size=medium:days=3:fuel=tight")
    # Day-structure axis: single-day and two-day boards.
    if arch in ("open", "mountain-heavy", "sparse-spots", "few-series"):
        for s in seeds[:2]:
            specs.append(f"{arch}:seed={s}:size=small:days=1")
    if arch in ("dense-spots", "pond-obstacle"):
        for s in seeds[:3]:
            specs.append(f"{arch}:seed={s}:days=2")
    return specs


def batches_for(group: str) -> list[dict]:
    """Return the batch table for one campaign group.

    Every batch carries its own case list, candidate tier, kinds pattern and
    baseline so two batches are never silently compared with different axes.
    """
    if group == "A":
        out = [
            {
                "name": "fixtures",
                "cases": FIXTURES,
                "case_files": CASE_FILES,
                "generated": [],
                "candidates": TIER_CORE,
                "kinds": "P,S",
            }
        ]
        for arch in ARCHES:
            out.append(
                {
                    "name": f"gen-{arch}",
                    "cases": "",
                    "case_files": [],
                    "generated": gen_specs_for(arch, EXPLORE_SEEDS),
                    "candidates": TIER_CORE,
                    "kinds": "P,S",
                }
            )
        return out

    if group == "SEQ":
        # Sequential (jobs=1) re-measurement of the finalists. Axis 4
        # (response time) is only trustworthy without CPU contention, so the
        # finalists get an uncontended pass on the same corpus.
        cands = [
            "coverage:budget=official:park=none",
            "coverage:budget=official:park=far",
            "coverage:budget=official:park=far:fuelaware=true",
            "anytime:budget_ms=0:seed=42:budget=official",
            "anytime:budget_ms=1000:seed=42:budget=official",
            "search:trials=16:budget=official",
            "search:trials=16:budget=official:lookahead=next",
            "search:trials=16:budget=official:park=far:lookahead=next",
            "search:trials=64:budget=official:lookahead=next",
        ]
        out = [
            {
                "name": "fixtures",
                "cases": FIXTURES,
                "case_files": CASE_FILES,
                "generated": [],
                "candidates": cands,
                "kinds": "P,S",
            }
        ]
        for arch in ARCHES:
            out.append(
                {
                    "name": f"gen-{arch}",
                    "cases": "",
                    "case_files": [],
                    "generated": gen_specs_for(arch, EXPLORE_SEEDS),
                    "candidates": cands,
                    "kinds": "P,S",
                }
            )
        return out

    if group == "C":
        # Follow-up on the group-A headline: lookahead=next dominates axes 1-3,
        # so sweep its interaction with trials and park mode on the same corpus
        # (same seeds, same baseline) as group A.
        cands = [
            "coverage:budget=official:park=none",
            "search:trials=16:budget=official:lookahead=next",
            "search:trials=16:budget=official:park=far:lookahead=next",
            "search:trials=32:budget=official:lookahead=next",
            "search:trials=64:budget=official:lookahead=next",
            "search:trials=64:budget=official:park=far:lookahead=next",
            "search:trials=128:budget=official:lookahead=next",
        ]
        out = [
            {
                "name": "fixtures",
                "cases": FIXTURES,
                "case_files": CASE_FILES,
                "generated": [],
                "candidates": cands,
                "kinds": "P,S",
            }
        ]
        for arch in ARCHES:
            out.append(
                {
                    "name": f"gen-{arch}",
                    "cases": "",
                    "case_files": [],
                    "generated": gen_specs_for(arch, EXPLORE_SEEDS),
                    "candidates": cands,
                    "kinds": "P,S",
                }
            )
        return out

    if group == "B":
        # Heavy tier on a fixed, representative subset: one board per archetype
        # at medium/3-day, plus both import cases. Same baseline as group A so
        # the tiers are joinable through coverage:park=none.
        gen = [f"{a}:seed=2:size=medium:days=3" for a in ARCHES]
        gen += [f"{a}:seed=3:size=large:days=3" for a in ("dense-spots", "many-series")]
        gen += [f"{a}:seed=4:size=medium:days=3:fuel=tight" for a in ("road-heavy", "open")]
        return [
            {
                "name": "heavy-subset",
                "cases": FIXTURES,
                "case_files": CASE_FILES,
                "generated": gen,
                "candidates": TIER_HEAVY,
                "kinds": "P,S",
            }
        ]

    if group == "KINDS":
        # Agent-kind axis (#235): the same boards under three compositions.
        gen = [f"{a}:seed=5:size=medium:days=3" for a in ARCHES]
        cases = "small-8x8,medium-16x16,large-32x32,series-many"
        return [
            {
                "name": f"kinds-{label}",
                "cases": cases,
                "case_files": ["crates/bench/tests/fixtures/arena-701.json"],
                "generated": gen,
                "candidates": TIER_CORE,
                "kinds": kinds,
            }
            for label, kinds in (
                ("balanced", "P,S,P,S,P"),
                ("patrol-heavy", "P,P,S,P,P"),
                ("supply-heavy", "S,S,P,S,P"),
            )
        ]

    if group == "HOLDOUT":
        # Confirmation only. Same shape as group A, disjoint seeds.
        out = []
        for arch in ARCHES:
            out.append(
                {
                    "name": f"hold-{arch}",
                    "cases": "",
                    "case_files": [],
                    "generated": gen_specs_for(arch, HOLDOUT_SEEDS),
                    "candidates": TIER_CORE,
                    "kinds": "P,S",
                }
            )
        return out

    raise SystemExit(f"unknown group {group!r} (expected A | B | C | KINDS | HOLDOUT)")


def expected_records(batch: dict) -> int:
    # Same anchor rule as batch_command: no fixtures named => small-8x8 is
    # added, so the batch still has one case to hang the generated ones on.
    fixture_names = batch["cases"] or ANCHOR_FIXTURE
    n_cases = len([c for c in fixture_names.split(",") if c])
    n_cases += len(batch["case_files"])
    n_cases += len(batch["generated"])
    return n_cases * len(batch["candidates"])


def batch_command(batch: dict, group: str, jobs: int, out_dir: Path) -> list[str]:
    cmd = [
        str(EVAL),
        "--candidates",
        ",".join(batch["candidates"]),
        "--baseline",
        BASELINE,
        "--jobs",
        str(jobs),
        "--output-dir",
        str(out_dir),
        "--quiet",
    ]
    # `--cases` must always name at least one fixture (load_corpus rejects an
    # empty selection before it ever looks at `--generated`). Generated-only
    # batches therefore carry `small-8x8` as a repeated anchor: the same
    # (candidate, case) shows up in every batch, so a merge that does not
    # reproduce identical scores there has a provenance bug.
    cmd += ["--cases", batch["cases"] or ANCHOR_FIXTURE]
    if batch["case_files"]:
        cmd += ["--case-files", ",".join(batch["case_files"])]
    if batch["generated"]:
        cmd += ["--generated", ",".join(batch["generated"])]
    if batch["kinds"]:
        cmd += ["--kinds", batch["kinds"]]
    return cmd


def is_done(out_dir: Path, expected: int) -> bool:
    runs = out_dir / "runs.jsonl"
    if not runs.exists():
        return False
    try:
        n = sum(1 for line in runs.open() if line.strip())
    except OSError:
        return False
    if n != expected:
        return False
    # Provenance must match this worktree's HEAD, otherwise the batch is a
    # different evaluated state and must not be silently reused.
    sha_file = out_dir / "source-sha.txt"
    head = git_head()
    if sha_file.exists() and sha_file.read_text().strip() != head:
        return False
    return True


def git_head() -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.strip()


def sample_metrics() -> dict:
    def read(path: str) -> str:
        try:
            return Path(path).read_text().strip()
        except OSError:
            return ""

    loadavg = read("/proc/loadavg").split()
    meminfo = {}
    for line in read("/proc/meminfo").splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            meminfo[k] = v.strip()
    return {
        "ts": time.time(),
        "load1": float(loadavg[0]) if loadavg else None,
        "mem_available": meminfo.get("MemAvailable"),
        "mem_free": meminfo.get("MemFree"),
        "swap_free": meminfo.get("SwapFree"),
    }


def run_batch(batch: dict, group: str, jobs: int) -> dict:
    out_dir = OUT_ROOT / group / batch["name"]
    out_dir.mkdir(parents=True, exist_ok=True)
    expected = expected_records(batch)

    if is_done(out_dir, expected):
        return {"batch": batch["name"], "status": "skipped", "expected": expected}

    cmd = batch_command(batch, group, jobs, out_dir)
    sha = git_head()
    (out_dir / "source-sha.txt").write_text(sha + "\n")
    started = time.time()
    metrics = []

    env = dict(os.environ)
    env["PROCON_SOURCE_COMMIT_SHA"] = sha

    proc = subprocess.Popen(
        cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
    )
    while proc.poll() is None:
        metrics.append(sample_metrics())
        time.sleep(5)
    output = proc.stdout.read() if proc.stdout else ""
    ended = time.time()

    (out_dir / "batch.json").write_text(
        json.dumps(
            {
                "batch": batch["name"],
                "group": group,
                "command": cmd,
                "source_commit_sha": sha,
                "jobs": jobs,
                "expected_records": expected,
                "exit_code": proc.returncode,
                "started_unix": started,
                "ended_unix": ended,
                "wall_s": round(ended - started, 3),
                "cases": {
                    "fixtures": batch["cases"],
                    "case_files": batch["case_files"],
                    "generated": batch["generated"],
                },
                "candidates": batch["candidates"],
                "baseline": BASELINE,
                "kinds": batch["kinds"],
                "metrics": metrics,
                "tail": output[-4000:],
            },
            indent=2,
        )
        + "\n"
    )

    return {
        "batch": batch["name"],
        "status": "ok" if proc.returncode == 0 else f"exit{proc.returncode}",
        "expected": expected,
        "wall_s": round(ended - started, 2),
        "out": str(out_dir.relative_to(ROOT)),
        "tail": output.strip().splitlines()[-1] if output.strip() else "",
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--group", default="A")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--only", default="", help="run only batches whose name contains this")
    ap.add_argument("--status", action="store_true")
    args = ap.parse_args()

    if not EVAL.exists() and not args.status:
        raise SystemExit(f"missing {EVAL} — cargo build --release --locked -p bench --bin procon-eval")

    batches = batches_for(args.group)
    if args.only:
        batches = [b for b in batches if args.only in b["name"]]

    if args.status:
        for g in ("A", "B", "C", "KINDS", "HOLDOUT"):
            for b in batches_for(g):
                d = OUT_ROOT / g / b["name"]
                exp = expected_records(b)
                done = is_done(d, exp)
                n = 0
                if (d / "runs.jsonl").exists():
                    n = sum(1 for line in (d / "runs.jsonl").open() if line.strip())
                print(f"{'DONE' if done else 'TODO':4} {g}/{b['name']:24} {n}/{exp}")
        return 0

    failures = 0
    for batch in batches:
        res = run_batch(batch, args.group, args.jobs)
        print(json.dumps(res), flush=True)
        if res["status"] not in ("ok", "skipped"):
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
