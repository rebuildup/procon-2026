#!/usr/bin/env python3
"""Release smoke suite: candidate registry resolution + official CLI acceptance.

Run at the release SHA to prove the battle candidates still parse+build through
the committed registry and that the official simple answer server accepts a
minimal coordinator session. This is a **smoke**, not a benchmark — runtime is
bounded (target < ~5 min end-to-end).

Steps
-----
1. Candidate resolution — `procon-eval --worker` on a tiny 1-day 8x8 case.
   Each candidate id is parsed, schema-validated, built and run for one
   simulated day through `bench::CandidateRegistry::builtin()`.
   - Required PASS: `coverage:budget=official:park=far`,
     `coverage:budget=official:park=none`.
   - Probe-only (recorded, never fails the suite): `anytime`,
     `coverage:budget=official:park=far:fuelaware=true`. These are expected
     NOT to be constructible today; the suite records the exact error text so
     a later registry change is visible in the evidence.

2. Official CLI acceptance — reuses `scripts/campaign/official_cli_smoke.py`
   against the vendored simple answer server
   (`docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip`,
   binary `procon-server-darwin-arm64`, sha256
   `cd552d93980c6b2a503c859dedc0357666823451662381cfa886260f5a81e041`).
   A minimal `coordinator --candidate coverage:budget=official:park=far`
   session must be accepted by the server: one revision per day, all >= 1.

Usage
-----
    python3 scripts/campaign/release_smoke.py
    python3 scripts/campaign/release_smoke.py --skip-cli   # step 1 only
    python3 scripts/campaign/release_smoke.py --json-out target/release-smoke/summary.json

Exit status is non-zero when a required step fails. Probe-only facts never
change the exit status. Transient artefacts land under `target/release-smoke/`
(git-ignored); durable evidence is written separately under
`docs/evidence/campaign-311/release-smoke/`.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time
import zipfile

ROOT = Path(__file__).resolve().parents[2]

# --- step 1: candidate resolution ------------------------------------------
# Required battle candidates. Failures here fail the suite.
REQUIRED_CANDIDATES = (
    'coverage:budget=official:park=far',
    'coverage:budget=official:park=none',
)
# Probe-only ids: record constructibility facts, never fail the suite.
# Expected today: `anytime` is not a registered algorithm, and `coverage`
# takes only `budget` / `park` — `fuelaware` is an unknown parameter.
PROBE_CANDIDATES = (
    'anytime',
    'coverage:budget=official:park=far:fuelaware=true',
)

# Smallest legal official board (edge 8..=32, agents 3..=8, spots >= agents).
# One day keeps the smoke bounded while still exercising parse + build_solver +
# one full MatchSim day through the committed registry.
TINY_CASE = {
    'case_id': 'smoke:tiny-8x8',
    'seed': None,
    'generation_parameters': None,
    'source': {'kind': 'fixture', 'name': 'tiny-8x8'},
    'startsAt': 0,
    'daySeconds': [1],
    'daySteps': [16],  # W+H = 16 for an 8x8 board
    'map': {'height': 8, 'width': 8, 'cells': [[0] * 8 for _ in range(8)]},
    'spots': [
        {'brand': 0, 'pos': 1, 'stocks': 3},
        {'brand': 1, 'pos': 2, 'stocks': 3},
        {'brand': 2, 'pos': 3, 'stocks': 3},
    ],
    'agents': [16, 17, 18],
    'fuelLimits': 48,
    'players': 1,
    'busyThreshold': 1,
    'jammedThreshold': 2,
}

# --- step 2: official CLI acceptance ---------------------------------------
SERVER_ZIP = ROOT / 'docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip'
SERVER_NAME = 'procon-server-darwin-arm64'
SERVER_SHA256 = 'cd552d93980c6b2a503c859dedc0357666823451662381cfa886260f5a81e041'
# Relative to ROOT; `target/` is git-ignored so the binary is never committed.
SERVER_DIR = ROOT / 'target/official-server/simple-server'
CLI_SMOKE = ROOT / 'scripts/campaign/official_cli_smoke.py'
CASE_FILE = ROOT / 'crates/bench/tests/fixtures/arena-701.json'
CLI_CANDIDATE = 'coverage:budget=official:park=far'

ANSWER_REVISION_MIN = 1  # official server answers are 1-based


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def rel(path: Path) -> str:
    """Path relative to ROOT when possible — keeps evidence free of $HOME."""
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return path.name


def ensure_binaries() -> tuple[Path, Path]:
    """Return (procon-eval, procon) release paths, building them if missing."""
    eval_bin = ROOT / 'target/release/procon-eval'
    cli_bin = ROOT / 'target/release/procon'
    missing = [p for p in (eval_bin, cli_bin) if not p.is_file()]
    if missing:
        print(f'[build] missing {[rel(p) for p in missing]}; cargo build --release')
        env = os.environ.copy()
        env['CARGO_INCREMENTAL'] = '0'
        subprocess.run(
            ['cargo', 'build', '--release', '--locked', '--bin', 'procon', '--bin', 'procon-eval'],
            cwd=ROOT, env=env, check=True,
        )
    return eval_bin, cli_bin


def ensure_server() -> Path:
    """Extract and verify the vendored official simple answer server."""
    binary = SERVER_DIR / SERVER_NAME
    if not binary.is_file():
        print(f'[server] extracting {SERVER_NAME} from {rel(SERVER_ZIP)}')
        SERVER_DIR.mkdir(parents=True, exist_ok=True)
        # The vendored zip nests `簡易サーバー.zip` (the platform binaries).
        # Names are UTF-8; Python's zipfile decodes them reliably.
        with zipfile.ZipFile(SERVER_ZIP) as outer:
            inner_name = next(
                (n for n in outer.namelist() if n.endswith('簡易サーバー.zip')),
                None,
            )
            if inner_name is None:
                raise RuntimeError(f'簡易サーバー.zip not found inside {rel(SERVER_ZIP)}')
            with outer.open(inner_name) as fh:
                inner_bytes = fh.read()
        with zipfile.ZipFile(__import__('io').BytesIO(inner_bytes)) as inner:
            try:
                inner.extract(SERVER_NAME, SERVER_DIR)
            except KeyError as exc:
                raise RuntimeError(f'{SERVER_NAME} not found in 簡易サーバー.zip') from exc
        binary.chmod(0o755)
    digest = sha256_file(binary)
    if digest != SERVER_SHA256:
        raise RuntimeError(
            f'server sha256 mismatch: got {digest}, expected {SERVER_SHA256} '
            f'({rel(binary)})'
        )
    return binary


def run_candidate_smoke(eval_bin: Path, out_dir: Path) -> dict:
    """Step 1 — parse+build each candidate id through the builtin registry."""
    case_json = json.dumps(TINY_CASE)
    results = {}
    for kind, ids in (('required', REQUIRED_CANDIDATES), ('probe', PROBE_CANDIDATES)):
        for cid in ids:
            t0 = time.monotonic()
            proc = subprocess.run(
                [str(eval_bin), '--worker', '--candidate-id', cid, '--case-json', case_json],
                cwd=ROOT, capture_output=True, text=True, timeout=120,
            )
            elapsed = time.monotonic() - t0
            record = None
            parse_error = None
            for line in proc.stdout.splitlines():
                line = line.strip()
                if line.startswith('{'):
                    try:
                        record = json.loads(line)
                    except json.JSONDecodeError as exc:
                        parse_error = f'bad worker JSON: {exc}'
            if record is None:
                parse_error = parse_error or (
                    f'no RunRecord on stdout (exit={proc.returncode}): '
                    f'{proc.stderr.strip()[:200] or proc.stdout.strip()[:200]}'
                )
                results[cid] = {
                    'kind': kind,
                    'ok': False,
                    'status': None,
                    'scored': None,
                    'detail': parse_error,
                    'elapsed_s': round(elapsed, 3),
                }
                continue
            status = record.get('status')
            scored = bool(record.get('scored'))
            detail = record.get('status_detail')
            # A required candidate must parse, build AND finish one day scored.
            ok = status == 'success' and scored
            results[cid] = {
                'kind': kind,
                'ok': ok,
                'status': status,
                'scored': scored,
                'detail': detail,
                'variety': record.get('variety'),
                'total_balls': record.get('total_balls'),
                'elapsed_s': round(elapsed, 3),
            }
    summary = {
        'tiny_case': TINY_CASE['case_id'],
        'eval_binary': rel(eval_bin),
        'candidates': results,
        'required_ok': all(results[c]['ok'] for c in REQUIRED_CANDIDATES),
        'probe_facts': {
            c: {
                'constructible': results[c]['ok'],
                'status': results[c]['status'],
                'detail': results[c]['detail'],
            }
            for c in PROBE_CANDIDATES
        },
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / 'candidate-smoke.json').write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + '\n'
    )
    return summary


def run_cli_smoke(cli_bin: Path, server_bin: Path, out_dir: Path) -> dict:
    """Step 2 — minimal coordinator session against the official server."""
    if out_dir.exists():
        shutil.rmtree(out_dir)
    cmd = [
        sys.executable, str(CLI_SMOKE),
        '--server-binary', str(server_bin),
        '--cli', str(cli_bin),
        '--case-file', str(CASE_FILE),
        '--candidate', CLI_CANDIDATE,
        '--seconds', '3',
        '--output-dir', str(out_dir),
    ]
    t0 = time.monotonic()
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, timeout=300)
    elapsed = time.monotonic() - t0
    summary_path = out_dir / 'summary.json'
    smoke_summary = None
    if summary_path.is_file():
        smoke_summary = json.loads(summary_path.read_text())
    revisions = (smoke_summary or {}).get('revisions') or []
    accepted = (
        proc.returncode == 0
        and smoke_summary is not None
        and len(revisions) == (smoke_summary.get('days') or 0)
        and all(isinstance(r, int) and r >= ANSWER_REVISION_MIN for r in revisions)
    )
    return {
        'ok': accepted,
        'exit_code': proc.returncode,
        'candidate': CLI_CANDIDATE,
        'days': (smoke_summary or {}).get('days'),
        'revisions': revisions,
        'min_revision': min(revisions) if revisions else None,
        'revision_floor': ANSWER_REVISION_MIN,
        'source_sha': (smoke_summary or {}).get('source_sha'),
        'server_sha256': (smoke_summary or {}).get('server_sha256'),
        'cli_sha256': (smoke_summary or {}).get('cli_sha256'),
        'elapsed_s': round(elapsed, 3),
        'output_dir': rel(out_dir),
        'stderr_tail': proc.stderr.strip()[-400:] if proc.returncode != 0 else '',
        'limitation': (smoke_summary or {}).get('limitation'),
    }


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--skip-cli', action='store_true',
                   help='run only the candidate-resolution smoke')
    p.add_argument('--output-dir', type=Path, default=ROOT / 'target/release-smoke',
                   help='transient artefact directory (default: target/release-smoke)')
    p.add_argument('--json-out', type=Path, default=None,
                   help='also write the suite summary JSON to this path')
    args = p.parse_args()

    out_dir: Path = args.output_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    print('=' * 72)
    print('release smoke')
    print('=' * 72)
    load = os.getloadavg() if hasattr(os, 'getloadavg') else (0.0, 0.0, 0.0)
    print(f'source_sha : {subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()}')
    print(f'platform   : {platform.system()} {platform.machine()} ({platform.platform()})')
    print(f'loadavg    : {load[0]:.2f} {load[1]:.2f} {load[2]:.2f}')
    print(f'output_dir : {rel(out_dir)}')
    print()

    eval_bin, cli_bin = ensure_binaries()
    print(f'binaries   : {rel(eval_bin)}, {rel(cli_bin)}')

    # ---- step 1: candidate resolution ----
    print('-' * 72)
    print('step 1/2  candidate resolution (procon-eval --worker, tiny 1-day case)')
    print('-' * 72)
    cand = run_candidate_smoke(eval_bin, out_dir)
    for cid, r in cand['candidates'].items():
        mark = 'PASS' if r['ok'] else ('FAIL' if r['kind'] == 'required' else 'FACT')
        detail = r['detail'] or ''
        extra = f'  ({detail})' if detail else ''
        print(f'  [{mark}] {cid}  status={r["status"]} scored={r["scored"]}{extra}')
    print(f'  required: {"PASS" if cand["required_ok"] else "FAIL"}')
    print()

    # ---- step 2: official CLI acceptance ----
    cli_result = None
    if args.skip_cli:
        print('step 2/2  official CLI acceptance — SKIPPED (--skip-cli)')
    else:
        print('-' * 72)
        print('step 2/2  official CLI acceptance (official_cli_smoke.py)')
        print('-' * 72)
        try:
            server_bin = ensure_server()
            print(f'  server   : {rel(server_bin)}  sha256={sha256_file(server_bin)[:16]}…')
            cli_result = run_cli_smoke(cli_bin, server_bin, out_dir / 'official-cli')
        except Exception as exc:  # noqa: BLE001 — record, do not fabricate
            cli_result = {
                'ok': False,
                'error': f'{type(exc).__name__}: {exc}',
                'candidate': CLI_CANDIDATE,
            }
        if cli_result.get('ok'):
            print(f'  [PASS] candidate={cli_result["candidate"]} '
                  f'days={cli_result["days"]} revisions={cli_result["revisions"]}')
        else:
            print(f'  [FAIL] {cli_result}')
        print()

    # ---- summary ----
    required_ok = cand['required_ok']
    cli_ok = True if args.skip_cli else bool(cli_result and cli_result.get('ok'))
    overall = required_ok and cli_ok
    print('=' * 72)
    print('summary')
    print('=' * 72)
    print(f'  candidate build  : {"PASS" if required_ok else "FAIL"}')
    for cid in REQUIRED_CANDIDATES:
        r = cand['candidates'][cid]
        print(f'    - {cid}: {"PASS" if r["ok"] else "FAIL"}'
              + (f' ({r["detail"]})' if r['detail'] else ''))
    print('  probe facts (informational):')
    for cid in PROBE_CANDIDATES:
        r = cand['candidates'][cid]
        state = 'constructible' if r['ok'] else 'NOT constructible'
        print(f'    - {cid}: {state}'
              + (f' — {r["detail"]}' if r['detail'] else ''))
    if args.skip_cli:
        print('  official CLI     : SKIPPED')
    elif cli_result and cli_result.get('ok'):
        print(f'  official CLI     : PASS  revisions={cli_result["revisions"]} '
              f'(min={cli_result["min_revision"]}, floor={cli_result["revision_floor"]})')
    else:
        print(f'  official CLI     : FAIL  {cli_result}')
    print(f'  overall          : {"PASS" if overall else "FAIL"}')
    print('=' * 72)

    summary = {
        'suite': 'release-smoke',
        'source_sha': subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'platform': {'system': platform.system(), 'machine': platform.machine()},
        'loadavg': [round(x, 2) for x in load],
        'candidate': cand,
        'official_cli': cli_result,
        'overall': 'PASS' if overall else 'FAIL',
    }
    (out_dir / 'summary.json').write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + '\n'
    )
    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(summary, indent=2, ensure_ascii=False) + '\n')
    return 0 if overall else 1


if __name__ == '__main__':
    sys.exit(main())
