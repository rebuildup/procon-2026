import hashlib, json, os, platform, subprocess, time
from pathlib import Path
root = Path.cwd()
out = root / 'target/rarity-413-evaluation'
out.mkdir(parents=True, exist_ok=True)
if (out / 'manifest.json').exists():
    raise SystemExit('Existing evaluation output; use a fresh checkout.')
sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
assert sha == 'd87c66d169087e917881fb4eb676a3b2cac8b06c'
binary = root / 'target/release/procon-eval'
prefix = 'fleet:budget=official:iters=20000:chains=4:seed=42'
commands = []
inputs = {}
for stage in ['hard', 'holdout', 'official']:
    args = [str(binary), '--cases', 'all' if stage == 'official' else 'openapi-sample']
    paths = sorted(root.glob(f'docs/evidence/campaign-311/fleet-371/corpus-{stage}/*.json')) if stage != 'official' else []
    if paths:
        args += ['--case-files', ','.join(str(p.relative_to(root)) for p in paths)]
    for p in paths:
        inputs[str(p.relative_to(root))] = hashlib.sha256(p.read_bytes()).hexdigest()
    args += ['--kinds', 'S,P,P,P,P,P,P,P', '--jobs', '1', '--candidates', prefix + ':rarity=0,' + prefix + ':rarity=2', '--baseline', prefix + ':rarity=0', '--output-dir', str(out / stage), '--quiet']
    commands.append((stage,args))
for p in sorted(root.glob('crates/bench/fixtures/maps/*.json')):
    inputs[str(p.relative_to(root))] = hashlib.sha256(p.read_bytes()).hexdigest()
manifest = dict(schema_version=1, issue=417, execution_generation=1, source_commit_sha=sha, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), platform=platform.platform(), inputs=inputs, commands=commands, pid=os.getpid(), status='running', started_at=time.time(), results=[])
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
env = dict(os.environ, PROCON_SOURCE_COMMIT_SHA=sha)
for stage,args in commands:
    print('START',stage,flush=True)
    started = time.monotonic()
    with (out/f'{stage}.log').open('w') as log:
        child = subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT)
        manifest['active_process'] = dict(stage=stage,pid=child.pid)
        (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
        code = child.wait()
    result = dict(stage=stage,exit_code=code,elapsed_seconds=time.monotonic()-started)
    manifest['results'].append(result)
    manifest['active_process'] = None
    (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print('END',json.dumps(result),flush=True)
    if code:
        manifest['status'] = 'failed'
        break
else:
    manifest['status'] = 'completed'
manifest['finished_at'] = time.time()
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
