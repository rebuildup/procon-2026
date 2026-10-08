#!/usr/bin/env python3
"""Run the #312 common corpus with explicit official budgets and kind parity."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parents[2]
# Short names of the bundled fixture corpus (`--cases all`). Kept explicit so
# the diagnose driver can build the same corpus procon-eval batches.
BUNDLED = ['openapi-sample', 'small-8x8', 'medium-16x16', 'large-32x32', 'road-heavy',
           'road-heavy-v2', 'mountain-heavy', 'series-many', 'series-few']
CANDIDATES = ['multiloop:grid=2x2:walk=astar:assign=all:budget=official',
              'multiloop:kmeans=5:seed=42:walk=astar:assign=all:budget=official',
              'multiloop:kmeans=8:seed=42:walk=astar:assign=all:budget=official',
              'loop:iterations=128:initial=greedy:walk=astar:assign=all:budget=official']
BASELINE = CANDIDATES[0]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output-dir', type=Path, default=ROOT / 'target/campaign312')
p.add_argument('--profile', choices=['debug', 'release'], default='debug')
p.add_argument('--kinds', default='P,S')
p.add_argument('--cases', default='all', help="bundled fixture names, or 'all'")
p.add_argument('--case-files', default='crates/bench/tests/fixtures/arena-701.json,'
                'docs/evidence/campaign-311/baseline-312/import-live-match.json')
p.add_argument('--generated', default='open:seed=1:days=3:fuel=tight,'
                'road-heavy:seed=1:days=3:fuel=tight,many-series:seed=3:days=3')
p.add_argument('--diagnose', action='store_true',
                help='also write diagnostics.jsonl (daily series sets + stop reasons)')
p.add_argument('--skip-batch', action='store_true',
                help='skip the procon-eval batch and only record diagnostics')
a = p.parse_args()
if a.skip_batch and not a.diagnose:
    p.error('--skip-batch requires --diagnose')
a.output_dir.mkdir(parents=True, exist_ok=True)
if not a.skip_batch and (a.output_dir / 'runs.jsonl').exists():
    p.error('output already contains runs.jsonl; use a fresh directory')
if a.diagnose and (a.output_dir / 'diagnostics.jsonl').exists():
    p.error('output already contains diagnostics.jsonl; use a fresh directory')
env = os.environ.copy()
env.update(CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='2', TMPDIR=str(ROOT / '.runtime/tmp'))
Path(env['TMPDIR']).mkdir(parents=True, exist_ok=True)
sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
env['PROCON_SOURCE_COMMIT_SHA'] = sha
response_time_policy = {
    'debug': 'Measured daily solver time; debug profile is exploratory, not competition timing evidence.',
    'release': 'Measured daily solver time under the release profile; '
               'this is the competition-representative response_time axis.',
}[a.profile]
metadata = dict(source_commit_sha=sha, profile=a.profile, kinds=a.kinds,
                platform=platform.platform(), processor=platform.processor(),
                rustc=subprocess.check_output(['rustc', '+stable', '--version'], text=True).strip(),
                oracle_refill=False, response_time_policy=response_time_policy)
if not a.skip_batch:
    build = ['cargo', '+stable', 'build', '--offline', '--locked', '-p', 'bench', '--bin', 'procon-eval']
    if a.profile == 'release':
        build.append('--release')
    subprocess.run(build, cwd=ROOT, env=env, check=True)
    command = [str(ROOT / 'target' / a.profile / 'procon-eval'), '--cases', a.cases,
               '--case-files', a.case_files,
               '--generated', a.generated,
               '--kinds', a.kinds, '--candidates', ','.join(CANDIDATES), '--baseline', BASELINE,
               '--output-dir', str(a.output_dir.resolve())]
    (a.output_dir / 'environment.json').write_text(
        json.dumps(dict(metadata, command=command, build=build), indent=2) + '\n')
    subprocess.run(command, cwd=ROOT, env=env, check=True)
if a.diagnose:
    build = ['cargo', '+stable', 'build', '--offline', '--locked', '-p', 'bench',
             '--example', 'campaign_diagnose']
    if a.profile == 'release':
        build.append('--release')
    subprocess.run(build, cwd=ROOT, env=env, check=True)
    if a.cases.strip().lower() == 'all':
        fixtures = BUNDLED
    else:
        fixtures = [n.strip() for n in a.cases.split(',') if n.strip()]
    sources = [f'fixture:{n}' for n in fixtures]
    sources += [s.strip() for s in a.case_files.split(',') if s.strip()]
    sources += [f'generated:{s.strip()}' for s in a.generated.split(',') if s.strip()]
    diag_bin = str(ROOT / 'target' / a.profile / 'examples' / 'campaign_diagnose')
    chunks = []
    for source in sources:
        for candidate in CANDIDATES:
            chunks.append(subprocess.check_output(
                [diag_bin, source, candidate, a.kinds], cwd=ROOT, env=env, text=True))
    (a.output_dir / 'diagnostics.jsonl').write_text(''.join(chunks))
    (a.output_dir / 'diagnostics-environment.json').write_text(json.dumps(dict(
        metadata, diagnose_build=build,
        diagnose_command=[diag_bin, '<CASE_SOURCE>', '<CANDIDATE>', a.kinds],
        sources=sources, candidates=CANDIDATES), indent=2) + '\n')
