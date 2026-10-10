#!/usr/bin/env python3
"""Run reproducible candidate/kinds evaluation for issue 398; offline only."""
import argparse
import csv
import hashlib
import json
import os
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RAW = ROOT / "data/rehearsal/2026-10-10-105938.setting.json"
CASE = ROOT / "data/rehearsal/2026-10-10-105938.case.json"
ID = "fixture:rehearsal-20261010-105938"
SHA = "0878fa9f3a23d3942151a73cc3647cc7f96800fdb94134d161a76a3a58db107b"

ONE_SUPPLY = "S,P,P,P"
ALL_PATROL = "P,P,P,P"
ALTERNATING = "P,S,P,S"
TRIAL = "fleet:budget=official:iters=8000"
LIVE = "fleet:budget=official:budget_ms=10000"
ANYTIME = "anytime:budget_ms=5000:seed=42:budget=official"
COVERAGE = "coverage:budget=official"


def scenarios(full):
    """(label, kinds, candidate, repeats). Wall-clock candidates repeat."""
    cases = [
        ("trial-1s", ONE_SUPPLY, TRIAL, 1),
        ("trial-4p", ALL_PATROL, TRIAL, 1),
        ("live-1s", ONE_SUPPLY, LIVE, 1),
        ("live-4p", ALL_PATROL, LIVE, 1),
        ("anytime", ALTERNATING, ANYTIME, 1),
        ("coverage", ALTERNATING, COVERAGE, 1),
    ]
    if not full:
        return cases
    named = [(ONE_SUPPLY, "1s"), (ALL_PATROL, "4p"), (ALTERNATING, "psps")]
    # Stage A/B: every strategy under every composition.
    cases = []
    for kinds, tag in named:
        cases += [
            (f"trial-{tag}", kinds, TRIAL, 1),
            (f"det20k-{tag}", kinds, "fleet:budget=official:iters=20000:chains=4", 1),
            (f"det60k-{tag}", kinds, "fleet:budget=official:iters=60000:chains=4", 1),
            (f"live-{tag}", kinds, LIVE, 3),
            (f"anytime-{tag}", kinds, ANYTIME, 2),
            (f"coverage-{tag}", kinds, COVERAGE, 1),
        ]
    # Stage B: where the single Supply sits, and a second Supply.
    for kinds, tag in [("P,S,P,P", "s1"), ("P,P,S,P", "s2"), ("P,P,P,S", "s3"),
                       ("S,S,P,P", "2s")]:
        cases += [(f"trial-{tag}", kinds, TRIAL, 1), (f"live-{tag}", kinds, LIVE, 3)]
    # Stage C: search budget. `adapt=on` caps the window at
    # (patrols * spots)^2 / 3 ms, so the larger budgets only differ with adapt=off.
    for kinds, tag, repeats in [(ONE_SUPPLY, "1s", 3), (ALL_PATROL, "4p", 2)]:
        for ms in [250, 500, 1000, 2000, 5000, 10000]:
            for adapt in ["on", "off"]:
                cases.append((f"ms{ms}-{adapt}-{tag}", kinds,
                              f"fleet:budget=official:budget_ms={ms}:adapt={adapt}", repeats))
    # Seed sensitivity of the leading composition: is the top score one seed's luck?
    for seed in [1, 2, 3, 7, 2026]:
        cases += [
            (f"live-1s-seed{seed}", ONE_SUPPLY, f"{LIVE}:seed={seed}", 2),
            (f"det60k-1s-seed{seed}", ONE_SUPPLY,
             f"fleet:budget=official:iters=60000:chains=4:seed={seed}", 1),
        ]
    return cases


def confirmation():
    """Stage D: enough repeats to tell a setting's spread from a real difference."""
    cases = [(f"confirm-ms{ms}", ONE_SUPPLY, f"fleet:budget=official:budget_ms={ms}", 10)
             for ms in [250, 500, 10000]]
    # Past the adaptive cap: does a longer window make the day-1 variety stick?
    cases += [(f"confirm-ms{ms}-off", ONE_SUPPLY,
               f"fleet:budget=official:budget_ms={ms}:adapt=off", 10) for ms in [1000, 2000]]
    # `iters` candidates do not read the clock: repeats must agree exactly, and
    # only the seed moves the score. More iterations on the same seeds shows
    # whether a longer search removes the seed dependence.
    cases.append(("confirm-det60k-seed42-again", ONE_SUPPLY,
                  "fleet:budget=official:iters=60000:chains=4", 3))
    for iters in [20000, 60000, 120000, 240000]:
        for seed in [42, 1, 2, 3, 7, 11, 12, 2026]:
            cases.append((f"confirm-det{iters // 1000}k-seed{seed}", ONE_SUPPLY,
                          f"fleet:budget=official:iters={iters}:chains=4:seed={seed}", 1))
    return cases


def paired():
    """Stage D A/B: arms alternate run by run so machine load hits them alike."""
    arms = [("ab-ms10000", LIVE, 20),
            ("ab-ms1000-off", "fleet:budget=official:budget_ms=1000:adapt=off", 20),
            ("ab-ms5000-off", "fleet:budget=official:budget_ms=5000:adapt=off", 8),
            ("ab-ms10000-off", "fleet:budget=official:budget_ms=10000:adapt=off", 8)]
    return [(label, index, ONE_SUPPLY, candidate)
            for index in range(1, 21) for label, candidate, repeats in arms if index <= repeats]


def validate():
    content = RAW.read_bytes()
    assert hashlib.sha256(content).hexdigest() == SHA, "raw JSON was changed"
    raw = json.loads(content)
    case = json.loads(CASE.read_text(encoding="utf8"))
    assert case["case_id"] == ID
    assert case["source"] == {"kind": "fixture", "name": "rehearsal-20261010-105938"}
    assert case["seed"] is None and case["generation_parameters"] is None
    assert {k: v for k, v in case.items() if k not in (
        "case_id", "source", "seed", "generation_parameters")} == raw
    assert raw["map"]["width"] == raw["map"]["height"] == len(raw["map"]["cells"]) == 16
    assert all(len(row) == 16 for row in raw["map"]["cells"])
    assert len(raw["agents"]) == 4 and len(raw["spots"]) == 16
    assert len({spot["brand"] for spot in raw["spots"]}) == 10
    assert raw["fuelLimits"] == 40 and raw["players"] == 10
    assert raw["daySteps"] == [32, 36, 40, 40, 48] and raw["daySeconds"] == [60]*5
    assert (raw["busyThreshold"], raw["jammedThreshold"]) == (1, 2)


def vdb(row):
    return (row["variety"], row["daily_variety_sum"], row["total_balls"])


def summarize(rows):
    """One line per (candidate, kinds): worst and best V/D/B over the repeats."""
    groups = {}
    for row in rows:
        groups.setdefault((row["label"], row["kinds"], row["candidate"]), []).append(row)
    out = []
    for (label, kinds, candidate), runs in groups.items():
        good = [r for r in runs if r.get("scored")]
        entry = dict(label=label, kinds=kinds, candidate=candidate, runs=len(runs),
                     excluded=len(runs) - len(good), sha=runs[0].get("sha"))
        if good:
            scores = [vdb(r) for r in good]
            entry.update(
                worst=min(scores), best=max(scores), stable=min(scores) == max(scores),
                t_median_us=int(statistics.median(r["response_time_us"] for r in good)),
                max_day_us=max(r["max_day_us"] for r in good),
                daily_varieties=[r["daily_varieties"] for r in good],
                load=max(r.get("load1") or 0 for r in good))
        out.append(entry)
    # Official order on the worst observed score: a candidate is only as good
    # as its unlucky run. Response time breaks V/D/B ties only.
    ranked = sorted([e for e in out if "worst" in e],
                    key=lambda e: (e["worst"], -e["t_median_us"]), reverse=True)
    return ranked, [e for e in out if "worst" not in e]


def fmt(score):
    return "/".join(str(x) for x in score)


def report(rows, output):
    (output / "results.json").write_text(json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf8")
    cols = ["label", "run", "kinds", "candidate", "status", "scored",
            "variety", "daily_variety_sum", "total_balls", "response_time_us",
            "max_day_us", "daily_varieties", "load1", "sha", "exit_code", "reason"]
    with (output / "results.csv").open("w", encoding="utf8", newline="") as f:
        writer = csv.DictWriter(f, cols, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    ranked, unscored = summarize(rows)
    (output / "summary.json").write_text(
        json.dumps(ranked + unscored, indent=2, ensure_ascii=False), encoding="utf8")
    shas = sorted({r["sha"] for r in rows if r.get("sha")})
    lines = [
        "# 予行演習マップ 2026-10-10 offline 比較",
        "",
        f"- 入力SHA256: {SHA}", f"- 対象: {ID}",
        f"- source SHA: {', '.join(shas) or '不明'}",
        "- 公式順位 V > D > B > T (Tは小さい方)。対象外の bundled fixture は集計しない。",
        "- 順位は反復中の最悪 V/D/B で付ける。T は V/D/B 同点のときだけ使う。",
        "- この値は単独offline予測。全10チームの渋滞・車種提出30秒制限・HTTP受理は未確認。",
        "",
        "| 順位 | 実験 | Kinds | Candidate | 回数 | 除外 | V/D/B 最悪 | V/D/B 最良 | 安定 | "
        "T 中央(ms) | 1日最大(ms) | 日別V |",
        "|---:|---|---|---|---:|---:|---|---|---|---:|---:|---|",
    ]
    for index, e in enumerate(ranked, 1):
        days = " ".join(sorted({"-".join(map(str, d)) for d in e["daily_varieties"]}))
        lines.append(
            f"| {index} | {e['label']} | {e['kinds']} | {e['candidate']} | {e['runs']} | "
            f"{e['excluded']} | {fmt(e['worst'])} | {fmt(e['best'])} | "
            f"{'yes' if e['stable'] else 'NO'} | {e['t_median_us']/1000:.1f} | "
            f"{e['max_day_us']/1000:.1f} | {days} |")
    trials = {e["label"]: e for e in ranked if e["label"] in ("trial-1s", "trial-4p")}
    choice = "未判定"
    if len(trials) == 2:
        s, p = trials["trial-1s"]["worst"], trials["trial-4p"]["worst"]
        choice = (ALL_PATROL if (p[0], p[1]) >= (s[0], s[1]) and p[2]*100 >= s[2]*105
                  else ONE_SUPPLY)
    scored = sum(1 for r in rows if r.get("scored"))
    lines += ["", f"- kinds-auto 判定式（iters8000のoffline近似）: {choice}",
              "- kinds-auto 自体の1.5秒制限は別途 coordinator で検証。",
              f"- 採点対象: {scored} / {len(rows)} run", ""]
    for r in rows:
        if not r.get("scored"):
            lines.append(f"- 採点除外 {r['label']}#{r.get('run')}: {r.get('status')}, {r.get('reason')}")
    (output / "report.md").write_text("\n".join(lines) + "\n", encoding="utf8")
    print("Completed:", scored, "/", len(rows), "; kinds-auto proxy:", choice,
          "; report:", output / "report.md", flush=True)


def load1():
    try:
        return round(os.getloadavg()[0], 2)
    except (AttributeError, OSError):  # Windows has no load average
        return None


def run(binary, out, label, index, kinds, candidate, resume):
    folder = out / label / f"run{index}"
    folder.mkdir(parents=True, exist_ok=True)
    saved = folder / "row.json"
    if resume and saved.is_file():
        return json.loads(saved.read_text(encoding="utf8"))
    row = measure(binary, folder, label, index, kinds, candidate)
    saved.write_text(json.dumps(row, ensure_ascii=False), encoding="utf8")
    return row


def measure(binary, folder, label, index, kinds, candidate):
    cmd = [str(binary), "--cases", "small-8x8", "--case-files", str(CASE),
           "--candidates", candidate, "--baseline", candidate,
           "--kinds", kinds, "--jobs", "1", "--output-dir", str(folder)]
    (folder / "cmd.json").write_text(json.dumps(cmd, indent=2), encoding="utf8")
    base = dict(label=label, run=index, kinds=kinds, candidate=candidate, load1=load1())
    try:
        done = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True,
                              timeout=900, check=False)
        (folder / "stdout.log").write_text(done.stdout, encoding="utf8")
        (folder / "stderr.log").write_text(done.stderr, encoding="utf8")
        records = (folder / "runs.jsonl").read_text(encoding="utf8").splitlines()
        matches = [json.loads(line) for line in records
                   if line.strip() and json.loads(line).get("case_id") == ID]
        if len(matches) != 1:
            raise ValueError(f"expected 1 rehearsal record, got {len(matches)}")
        r = matches[0]
        expected = ["Supply" if k == "S" else "Patrol" for k in kinds.split(",")]
        problems = [name for name, bad in [
            ("exit_code", done.returncode != 0),
            ("not scored", r.get("scored") is not True or r.get("status") != "success"),
            ("timeout", bool(r.get("timeout"))),
            ("agent_kinds", r.get("agent_kinds") != expected),
            ("illegal_action", r.get("illegal_action_count", 0) != 0),
            ("fuel_short", r.get("fuel_short_count", 0) != 0),
            ("rejected", r.get("rejected_response_count", 0) != 0),
            ("day_budget", bool(r.get("day_budget_exceeded_days"))),
            ("days", len(r.get("daily_response_us", [])) != 5),
        ] if bad]
        return dict(base, status=r.get("status"), scored=not problems,
                    variety=r.get("variety"), daily_variety_sum=r.get("daily_variety_sum"),
                    total_balls=r.get("total_balls"), response_time_us=r.get("response_time_us"),
                    max_day_us=max(r.get("daily_response_us") or [0]),
                    daily_varieties=r.get("daily_varieties"),
                    sha=r.get("source_commit_sha"), exit_code=done.returncode,
                    reason="; ".join(problems + [r.get("status_detail") or ""]).strip("; "))
    except (OSError, ValueError, json.JSONDecodeError, subprocess.TimeoutExpired) as err:
        return dict(base, status="harness_error", scored=False, reason=str(err))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", type=Path, default=ROOT / "target/release/procon-eval")
    parser.add_argument("--out", type=Path, default=ROOT / "target/rehearsal-20261010")
    parser.add_argument("--full", action="store_true")
    parser.add_argument("--confirm", action="store_true",
                        help="repeat the leading settings instead of the comparison sweep")
    parser.add_argument("--paired", action="store_true",
                        help="interleaved A/B of the production window against fixed windows")
    parser.add_argument("--only", default="", help="run only labels containing this text")
    parser.add_argument("--resume", action="store_true",
                        help="reuse runs already measured under --out instead of repeating them")
    args = parser.parse_args()
    validate()
    binary = args.bin.resolve()
    if not binary.is_file():
        parser.error(f"build procon-eval first; not found: {binary}")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    if args.paired:
        plan = paired()
    else:
        plan = [(label, index, kinds, candidate)
                for label, kinds, candidate, repeats in (
                    confirmation() if args.confirm else scenarios(args.full))
                for index in range(1, repeats + 1)]
    for label, index, kinds, candidate in plan:
        if args.only not in label:
            continue
        print(f"Running {label}#{index}: {candidate} / {kinds}", flush=True)
        rows.append(run(binary, out, label, index, kinds, candidate, args.resume))
        report(rows, out)
    return 0 if rows and all(r.get("scored") for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
