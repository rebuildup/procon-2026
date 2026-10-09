#!/usr/bin/env python3
"""Candidate-vs-candidate competitive comparison on the official engine.

Issue #377 — the single deterministic entry point all candidate tracks
(#374 / #375 / #376) are judged on. It takes 2+ candidate configs, runs
each of them over the same CaseSpec board set through `procon-eval`
(official `engine::MatchSim` replay — never a hand-rolled simulator), and
produces, per board:

* the official lexicographic result: variety (V), daily_variety_sum (D),
  total_balls (B), response_time (lower better)
* per-day B (`daily_ball_counts`) and per-day distinct series
  (`daily_varieties`) — the "which day improved" axis
* per-day acceptance (`rejected_days`, `day_budget_exceeded_days`) and the
  invalid counters (illegal / fuel_short / rejected / status)
* win / draw / loss against a reference config on the official dictionary
  order V → D → B → response

Provenance: every record carries the `procon-eval`-stamped
`source_commit_sha`; the runner also writes a `batch.json` per config
recording the exact command, candidate id, kinds pattern and board list,
so a fresh agent can resume or re-verify without this conversation.

Determinism: offline candidates (`:iters=` / `:trials=`) are fully
deterministic — same SHA + same boards ⇒ same V/D/B. Wall-clock configs
(`:budget_ms=`) are load-sensitive; their rows are marked
`"deterministic": false` in the summary and response times must be
re-measured sequentially (`--jobs 1`) before being cited as axis-4
evidence.

Usage:

  python3 scripts/campaign/eval_compare.py \
      --eval target/release/procon-eval \
      --name holdout2-baseline \
      --case-files docs/evidence/campaign-311/eval-377/corpus-holdout2 \
      --config 'fleet:budget=official:iters=20000:chains=4|kinds=S,P,P,P,P,P,P,P' \
      --config 'anytime:trials=64:seed=42:budget=official|kinds=P,S' \
      --reference 'anytime:trials=64:seed=42:budget=official' \
      --jobs 4

  # resume / re-aggregate only (skips finished batches):
  python3 scripts/campaign/eval_compare.py ... --no-run

Output layout (fleet-371 conventions):

  <out>/runs/<config-label>/runs.jsonl     one durable record per board
  <out>/runs/<config-label>/batch.json     exact command + provenance
  <out>/compare.json                       per-board rows + summary
  stdout                                   W/D/L + score totals table
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

# The harness always includes one bundled fixture next to `--case-files`
# (procon-eval has no "case files only" mode); it is stripped everywhere.
FILLER_CASE = "fixture:openapi-sample"


def case_files_from(paths: list[str]) -> list[Path]:
    files: list[Path] = []
    for p in paths:
        path = Path(p)
        if path.is_dir():
            files.extend(sorted(path.glob("*.json")))
        elif path.is_file():
            files.append(path)
        else:
            raise SystemExit(f"case path not found: {path}")
    if not files:
        raise SystemExit("no CaseSpec JSON files selected")
    return files


def config_label(config: str) -> str:
    """Filesystem-safe label for one config spec."""
    cand = config.split("|", 1)[0]
    label = re.sub(r"[^A-Za-z0-9=+.-]+", "-", cand)
    return label.strip("-")


def parse_config(config: str) -> tuple[str, str]:
    """`'candidate-id|kinds=P,S'` (kinds part optional) → (id, kinds)."""
    cand, *rest = config.split("|", 1)
    kinds = ""
    if rest:
        spec = rest[0]
        if not spec.startswith("kinds="):
            raise SystemExit(f"config extra must start with 'kinds=': {config}")
        kinds = spec[len("kinds="):]
    return cand, kinds


def load_runs(path: Path) -> dict[str, dict]:
    runs = {}
    with path.open() as f:
        for line in f:
            r = json.loads(line)
            if r["case_id"] == FILLER_CASE:
                continue
            runs[r["case_id"]] = r
    return runs


def lexicographic(r: dict) -> tuple:
    """Official dictionary order: V→D→B higher better, response lower."""
    return (r["variety"], r["daily_variety_sum"], r["total_balls"], -r["response_time_us"])


def invalid_count(r: dict) -> int:
    return (
        r["illegal_action_count"]
        + r["fuel_short_count"]
        + r["rejected_response_count"]
        + (r["status"] != "success")
    )


def run_batch(
    eval_bin: Path,
    cand: str,
    kinds: str,
    case_files: list[Path],
    jobs: int,
    out_dir: Path,
    sha: str,
    dry_run: bool,
) -> tuple[Path, bool]:
    """Run one candidate config over the board set; skip if already done."""
    runs_path = out_dir / "runs.jsonl"
    if runs_path.exists():
        have = len(load_runs(runs_path))
        if have >= len(case_files):
            return runs_path, False
        print(f"  [resume] {out_dir.name}: {have}/{len(case_files)} boards — re-running", file=sys.stderr)
    if dry_run:
        print(f"  [dry-run] would run {cand} kinds={kinds or '(default)'} over {len(case_files)} boards", file=sys.stderr)
        return runs_path, False
    cmd = [
        str(eval_bin),
        "--cases", "openapi-sample",
        "--case-files", ",".join(str(f) for f in case_files),
        "--candidates", cand,
        "--jobs", str(jobs),
        "--output-dir", str(out_dir),
        "--quiet",
    ]
    if kinds:
        cmd += ["--kinds", kinds]
    env_sha = {"PROCON_SOURCE_COMMIT_SHA": sha}
    t0 = time.time()
    proc = subprocess.run(cmd, capture_output=True, text=True, env={**dict(__import__("os").environ), **env_sha})
    if proc.returncode != 0:
        raise SystemExit(f"procon-eval failed for {cand}:\n{proc.stdout}\n{proc.stderr}")
    batch = {
        "command": cmd,
        "candidate_id": cand,
        "kinds": kinds,
        "case_files": [str(f) for f in case_files],
        "jobs": jobs,
        "source_commit_sha": sha,
        "wall_clock_s": round(time.time() - t0, 1),
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "batch.json").write_text(json.dumps(batch, indent=1) + "\n")
    return runs_path, True


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--eval", default="target/release/procon-eval", help="procon-eval binary")
    ap.add_argument("--name", required=True, help="batch name (output subdir)")
    ap.add_argument("--case-files", required=True, nargs="+",
                    help="CaseSpec JSON files or dirs (repeatable)")
    ap.add_argument("--config", required=True, action="append",
                    help="'candidate-id|kinds=P,S' — repeat for 2+ configs")
    ap.add_argument("--reference", help="candidate id of the W/D/L reference (default: first config)")
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("--out", required=True, help="output dir, e.g. docs/evidence/.../runs/<name>")
    ap.add_argument("--no-run", action="store_true", help="aggregate existing runs.jsonl only")
    args = ap.parse_args()

    eval_bin = Path(args.eval)
    if not args.no_run and not eval_bin.exists():
        raise SystemExit(f"eval binary not found: {eval_bin}")
    if len(args.config) < 2:
        raise SystemExit("give 2+ --config entries — a comparison needs a counterpart")

    out = Path(args.out)
    case_files = case_files_from(args.case_files)
    sha = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip() or "unknown"

    configs = [(cand, kinds) for cand, kinds in (parse_config(c) for c in args.config)]
    ref_id = args.reference or configs[0][0]
    ref_labels = [config_label(c) for c, _ in configs if c == ref_id]
    if not ref_labels:
        raise SystemExit(f"--reference {ref_id!r} is not among the configs")
    ref_label = ref_labels[0]

    all_runs: dict[str, dict[str, dict]] = {}
    for cand, kinds in configs:
        label = config_label(f"{cand}|{kinds}" if kinds else cand)
        print(f"[{label}] {cand} kinds={kinds or '(default)'} boards={len(case_files)}", file=sys.stderr)
        if args.no_run:
            runs_path = out / "runs" / label / "runs.jsonl"
            if not runs_path.exists():
                raise SystemExit(f"--no-run: missing {runs_path}")
        else:
            runs_path, _ = run_batch(eval_bin, cand, kinds, case_files, args.jobs,
                                     out / "runs" / label, sha, dry_run=False)
        all_runs[label] = load_runs(runs_path)

    # ---- per-board comparison -------------------------------------------
    ref_runs = all_runs[ref_label]
    boards = sorted(ref_runs)
    missing = {l: sorted(set(ref_runs) - set(r)) for l, r in all_runs.items() if set(r) != set(ref_runs)}
    if any(missing.values()):
        raise SystemExit(f"board sets differ between configs: {missing}")

    rows = []
    summary = {}
    totals = {l: [0, 0, 0] for l in all_runs}  # V, D, B sums
    for cand_label, runs in all_runs.items():
        w = d = l = 0
        bad = 0
        for case_id in boards:
            r = runs[case_id]
            bad += invalid_count(r)
            totals[cand_label][0] += r["variety"]
            totals[cand_label][1] += r["daily_variety_sum"]
            totals[cand_label][2] += r["total_balls"]
            if cand_label == ref_label:
                continue
            x = ref_runs[case_id]
            a, b = lexicographic(r), lexicographic(x)
            w += a > b
            d += a == b
            l += a < b
        if cand_label != ref_label:
            summary[cand_label] = {"win": w, "draw": d, "loss": l, "invalid_runs": bad}
        else:
            summary[cand_label] = {"reference": True, "invalid_runs": bad}

    for case_id in boards:
        ref = ref_runs[case_id]
        row = {
            "case_id": case_id,
            "boards": {
                label: {
                    "variety": r["variety"],
                    "daily_variety_sum": r["daily_variety_sum"],
                    "total_balls": r["total_balls"],
                    "response_time_us": r["response_time_us"],
                    "daily_ball_counts": r.get("daily_ball_counts", []),
                    "daily_varieties": r.get("daily_varieties", []),
                    "rejected_days": r.get("rejected_days", []),
                    "day_budget_exceeded_days": r.get("day_budget_exceeded_days", []),
                    "illegal_action_count": r["illegal_action_count"],
                    "fuel_short_count": r["fuel_short_count"],
                    "rejected_response_count": r["rejected_response_count"],
                    "status": r["status"],
                    "scored": r["scored"],
                    "source_commit_sha": r["source_commit_sha"],
                    "agent_kinds": r.get("agent_kinds"),
                }
                for label in all_runs if case_id in all_runs[label]
                for r in [all_runs[label][case_id]]
            },
        }
        # win/loss against the reference on the official dictionary order
        for label, b in row["boards"].items():
            if label == ref_label:
                b["vs_reference"] = "reference"
                continue
            a_row = all_runs[label][case_id]
            a, x = lexicographic(a_row), lexicographic(ref)
            b["vs_reference"] = "win" if a > x else ("draw" if a == x else "loss")
        rows.append(row)

    result = {
        "name": args.name,
        "reference": ref_label,
        "configs": {label: {"candidate_id": cand, "kinds": kinds} for (cand, kinds), label in
                    zip(configs, [config_label(f"{c}|{k}" if k else c) for c, k in configs])},
        "case_files": [str(f) for f in case_files],
        "source_commit_sha": sha,
        "deterministic": all(":budget_ms=" not in c for c, _ in configs),
        "summary": summary,
        "totals": {l: {"variety": v[0], "daily_variety_sum": v[1], "total_balls": v[2]}
                   for l, v in totals.items()},
        "boards": rows,
    }
    out.mkdir(parents=True, exist_ok=True)
    (out / "compare.json").write_text(json.dumps(result, indent=1) + "\n")

    # ---- console table ---------------------------------------------------
    print(f"\n== {args.name} ({len(boards)} boards, reference: {ref_label}) ==")
    for label in all_runs:
        v0, v1, v2 = totals[label]
        s = summary[label]
        tag = "reference" if s.get("reference") else f"W{s['win']} D{s['draw']} L{s['loss']}"
        print(f"{label:48} {tag:16} V {v0:5} D {v1:6} "
              f"B {v2:6} invalid={s['invalid_runs']}")
    losses = [(r["case_id"], l) for r in rows for l, b in r["boards"].items()
              if b["vs_reference"] == "loss"]
    if losses:
        print("\nlosses vs reference:")
        for case_id, label in losses:
            b = next(r for r in rows if r["case_id"] == case_id)["boards"][label]
            print(f"  {case_id} [{label}] V{b['variety']} D{b['daily_variety_sum']} "
                  f"B{b['total_balls']} vs reference "
                  f"V{ref_runs[case_id]['variety']} D{ref_runs[case_id]['daily_variety_sum']} "
                  f"B{ref_runs[case_id]['total_balls']}")


if __name__ == "__main__":
    main()
