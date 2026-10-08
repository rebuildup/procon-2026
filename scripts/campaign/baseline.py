#!/usr/bin/env python3
"""Run the #312 common corpus with explicit official budgets and kind parity."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parents[2]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output-dir', type=Path, default=ROOT / 'target/campaign312')
p.add_argument('--profile', choices=['debug', 'release'], default='debug')
p.add_argument('--kinds', default='P,S')
a = p.parse_args()
a.output_dir.mkdir(parents=True, exist_ok=True)
if (a.output_dir / 'runs.jsonl').exists():
    p.error('output already contains runs.jsonl; use a fresh directory')
env = os.environ.copy()
env.update(CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='2', TMPDIR=str(ROOT / '.runtime/tmp'))
Path(env['TMPDIR']).mkdir(parents=True, exist_ok=True)
sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
env['PROCON_SOURCE_COMMIT_SHA'] = sha
build = ['cargo', '+stable', 'build', '--offline', '--locked', '-p', 'bench', '--bin', 'procon-eval']
if a.profile == 'release':
    build.append('--release')
subprocess.run(build, cwd=ROOT, env=env, check=True)
baseline = 'multiloop:grid=2x2:walk=astar:assign=all:budget=official'
candidates = [baseline, 'multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official',
              'multiloop:kmeans=8:seed=42:walk=astar:assign=all:budget=official',
              'loop:iterations=128:initial=greedy:walk=astar:assign=all:budget=official']
command = [str(ROOT / 'target' / a.profile / 'procon-eval'), '--cases', 'all',
           '--case-files', 'crates/bench/tests/fixtures/arena-701.json,docs/evidence/campaign-311/baseline-312/import-live-match.json',
           '--generated', 'open:seed=1:days=3:fuel=tight,road-heavy:seed=1:days=3:fuel=tight,many-series:seed=3:days=3',
           '--kinds', a.kinds, '--candidates', ','.join(candidates), '--baseline', baseline,
           '--output-dir', str(a.output_dir.resolve())]
metadata = dict(source_commit_sha=sha, profile=a.profile, kinds=a.kinds,
                platform=platform.platform(), processor=platform.processor(),
                rustc=subprocess.check_output(['rustc', '+stable', '--version'], text=True).strip(),
                command=command, build=build, oracle_refill=False,
                response_time_policy='Measured daily solver time; debug profile is exploratory, not competition timing evidence.')
(a.output_dir / 'environment.json').write_text(json.dumps(metadata, indent=2) + '\n')
subprocess.run(command, cwd=ROOT, env=env, check=True)
