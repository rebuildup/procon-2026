#!/usr/bin/env python3
"""Run reproducible candidate/kinds evaluation for issue 398; offline only."""
import argparse
import csv
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RAW = ROOT / "data/rehearsal/2026-10-10-105938.setting.json"
CASE = ROOT / "data/rehearsal/2026-10-10-105938.case.json"
ID = "fixture:rehearsal-20261010-105938"
SHA = "0878fa9f3a23d3942151a73cc3647cc7f96800fdb94134d161a76a3a58db107b"


def scenarios(full):
    trial = "fleet:budget=official:iters=8000"
    live = "fleet:budget=official:budget_ms=10000"
    cases = [
        ("trial-1s", "S,P,P,P", trial),
        ("trial-4p", "P,P,P,P", trial),
        ("live-1s", "S,P,P,P", live),
        ("live-4p", "P,P,P,P", live),
        ("anytime", "P,S,P,S", "anytime:budget_ms=5000:seed=42:budget=official"),
        ("coverage", "P,S,P,S", "coverage:budget=official"),
    ]
    if full:
        for kinds, label in [("S,P,P,P", "1s"), ("P,P,P,P", "4p")]:
            cases += [
                (f"det-{label}-20k", kinds, "fleet:budget=official:iters=20000:chains=4"),
                (f"det-{label}-60k", kinds, "fleet:budget=official:iters=60000:chains=4"),
            ]
            for ms in [250, 500, 1000, 2000, 5000, 10000]:
                for adapt in ["on", "off"]:
                    cand = f"fleet:budget=official:budget_ms={ms}:adapt={adapt}"
                    cases.append((f"live-{label}-{ms}-{adapt}", kinds, cand))
                    if ms in [500, 1000, 2000, 10000]:
                        cases.append((f"live-{label}-{ms}-{adapt}-repeat", kinds, cand))
    return cases


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


def score_key(row):
    return (row["variety"], row["daily_variety_sum"], row["total_balls"],
            -row["response_time_us"])


def report(rows, output):
    (output / "results.json").write_text(json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf8")
    cols = ["label", "kinds", "candidate", "status", "scored",
            "variety", "daily_variety_sum", "total_balls", "response_time_us",
            "max_day_us", "sha", "exit_code", "reason"]
    with (output / "results.csv").open("w", encoding="utf8", newline="") as f:
        writer = csv.DictWriter(f, cols, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    good = sorted([r for r in rows if r.get("scored")], key=score_key, reverse=True)
    lines = [
        "# 予行演習マップ 2026-10-10 offline 比較",
        "",
        f"- 入力SHA256: {SHA}", f"- 対象: {ID}",
        "- 公式順位 V > D > B > T (Tは小さい方)。対象外の bundled fixture は集計しない。",
        "- この値は単独offline予測。全10チームの渋滞・車種提出30秒制限・HTTP受理は未確認。",
        "",
        "| 順位 | 実験 | kinds | candidate | V | D | B | T(ms) | 1日最大(ms) |",
        "|---:|---|---|---|---:|---:|---:|---:|---:|",
    ]
    for index, r in enumerate(good, 1):
        lines.append(f"| {index} | {r['label']} | {r['kinds']} | {r['candidate']} | "
                     f"{r['variety']} | {r['daily_variety_sum']} | {r['total_balls']} | "
                     f"{r['response_time_us']/1000:.1f} | {r['max_day_us']/1000:.1f} |")
    trials = {r["label"]: r for r in good if r["label"] in ("trial-1s", "trial-4p")}
    choice = "未判定"
    if len(trials) == 2:
        s, p = trials["trial-1s"], trials["trial-4p"]
        choice = ("P,P,P,P" if
                  (p["variety"], p["daily_variety_sum"]) >= (s["variety"], s["daily_variety_sum"])
                  and p["total_balls"]*100 >= s["total_balls"]*105
                  else "S,P,P,P")
    lines.extend(["", f"- kinds-auto 判定式（iters8000のoffline近似）: {choice}",
                  "- kinds-auto 自体の1.5秒制限は別途 coordinator で検証。",
                  f"- 成功: {len(good)} / {len(rows)}", ""])
    failures = [r for r in rows if not r.get("scored")]
    for r in failures:
        lines.append(f"- 採点除外 {r['label']}: {r.get('status')}, {r.get('reason')}")
    live = [r for r in good if r["label"].startswith("live-")]
    if live:
        winner = max(live, key=score_key)
        lines += ["", "## 本番向け暫定候補（まだ確定ではない）", "",
                  f"- candidate: {winner['candidate']}",
                  f"- kinds: {winner['kinds']}",
                  f"- score V/D/B/T: {winner['variety']}/{winner['daily_variety_sum']}/{winner['total_balls']}/{winner['response_time_us']}us",
                  "- 確定には反復・公式サーバHTTP受理・期限内提出・Windows動作の確認が必要。"]
    (output / "report.md").write_text("\n".join(lines) + "\n", encoding="utf8")
    print("Completed:", len(good), "/", len(rows), "; kinds-auto proxy:", choice,
          "; report:", output / "report.md", flush=True)


def run(binary, out, label, kinds, candidate):
    folder = out / label
    folder.mkdir(parents=True, exist_ok=True)
    cmd = [str(binary), "--cases", "small-8x8", "--case-files", str(CASE),
           "--candidates", candidate, "--baseline", candidate,
           "--kinds", kinds, "--jobs", "1", "--output-dir", str(folder)]
    (folder / "cmd.json").write_text(json.dumps(cmd, indent=2), encoding="utf8")
    try:
        done = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True,
                              timeout=420, check=False)
        (folder / "stdout.log").write_text(done.stdout, encoding="utf8")
        (folder / "stderr.log").write_text(done.stderr, encoding="utf8")
        records = (folder / "runs.jsonl").read_text(encoding="utf8").splitlines()
        matches = [json.loads(line) for line in records
                   if line.strip() and json.loads(line).get("case_id") == ID]
        if len(matches) != 1:
            raise ValueError(f"expected 1 rehearsal record, got {len(matches)}")
        r = matches[0]
        expected = ["Supply" if k == "S" else "Patrol" for k in kinds.split(",")]
        valid = (done.returncode == 0 and r.get("scored") is True
                 and r.get("status") == "success" and r.get("agent_kinds") == expected
                 and r.get("illegal_action_count", 0) == 0
                 and r.get("fuel_short_count", 0) == 0
                 and r.get("rejected_response_count", 0) == 0
                 and not r.get("day_budget_exceeded_days")
                 and len(r.get("daily_response_us", [])) == 5)
        return dict(label=label, kinds=kinds, candidate=candidate,
                    status=r.get("status"), scored=valid,
                    variety=r.get("variety"), daily_variety_sum=r.get("daily_variety_sum"),
                    total_balls=r.get("total_balls"), response_time_us=r.get("response_time_us"),
                    max_day_us=max(r.get("daily_response_us") or [0]),
                    sha=r.get("source_commit_sha"), exit_code=done.returncode,
                    reason=r.get("status_detail") if not valid else "")
    except (OSError, ValueError, json.JSONDecodeError, subprocess.TimeoutExpired) as err:
        return dict(label=label, kinds=kinds, candidate=candidate,
                    status="harness_error", scored=False, reason=str(err))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", type=Path, default=ROOT / "target/release/procon-eval")
    parser.add_argument("--out", type=Path, default=ROOT / "target/rehearsal-20261010")
    parser.add_argument("--full", action="store_true")
    args = parser.parse_args()
    validate()
    binary = args.bin.resolve()
    if not binary.is_file():
        parser.error(f"build procon-eval first; not found: {binary}")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for label, kinds, candidate in scenarios(args.full):
        print(f"Running {label}: {candidate} / {kinds}", flush=True)
        rows.append(run(binary, out, label, kinds, candidate))
        report(rows, out)
    return 0 if all(r.get("scored") for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
