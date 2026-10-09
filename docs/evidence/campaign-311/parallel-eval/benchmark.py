#!/usr/bin/env python3
"""Parallel evaluation benchmark (Issue #335).

Run from the repo root after `cargo build --release -p bench --bin procon-eval`.

Usage:
    python3 docs/evidence/campaign-311/parallel-eval/benchmark.py [--rounds 3]
"""
import subprocess, time, random, json, sys, os

def run_eval(jobs, outdir, extra_args=None):
    cmd = [
        './target/release/procon-eval',
        '--cases', 'all',
        '--case-files', 'crates/bench/tests/fixtures/arena-701.json',
        '--kinds', 'P,S,P,S,P',
        '--candidates', 'coverage:budget=official:park=none,coverage:budget=official:park=far',
        '--baseline', 'coverage:budget=official:park=none',
        '--jobs', str(jobs),
        '--output-dir', outdir,
    ]
    if extra_args:
        cmd.extend(extra_args)
    env = os.environ.copy()
    env['PROCON_SOURCE_COMMIT_SHA'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    # Monotonic clock: wall-clock can jump (NTP, suspend) and corrupt elapsed.
    start = time.perf_counter()
    # check=True: a failed run must raise, never be recorded as a fast success.
    subprocess.run(cmd, capture_output=True, env=env, check=True)
    return time.perf_counter() - start

def main():
    rounds = 3
    if '--rounds' in sys.argv:
        idx = sys.argv.index('--rounds')
        rounds = int(sys.argv[idx + 1])

    jobs_list = [1, 2, 4, 8]
    results = {j: [] for j in jobs_list}

    # Warmup
    print("Warmup...")
    run_eval(1, '/tmp/bench-warmup')

    for rnd in range(rounds):
        order = jobs_list.copy()
        random.shuffle(order)
        print(f"Round {rnd+1}: order={order}")
        for jobs in order:
            elapsed = run_eval(jobs, f'/tmp/bench-r{rnd}-j{jobs}')
            results[jobs].append(elapsed)
            print(f"  jobs={jobs}: {elapsed:.3f}s")

    print(f"\n{'jobs':>4} {'mean':>8} {'min':>8} {'max':>8} {'std':>8} {'speedup':>8}")
    base = sum(results[1]) / len(results[1])
    for jobs in jobs_list:
        runs = results[jobs]
        mean = sum(runs) / len(runs)
        speedup = base / mean if mean > 0 else 0
        std = (sum((r - mean)**2 for r in runs) / len(runs)) ** 0.5
        print(f"{jobs:>4} {mean:>8.3f} {min(runs):>8.3f} {max(runs):>8.3f} {std:>8.3f} {speedup:>7.2f}x")

if __name__ == '__main__':
    main()
