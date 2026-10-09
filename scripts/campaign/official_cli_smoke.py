#!/usr/bin/env python3
"""Run the real CLI against the official binary, recording submitted plans and states.

This checks protocol acceptance, not arena rank. Answer windows are shortened.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import threading
import time
import urllib.error
import urllib.request

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--server-binary', type=Path, required=True)
p.add_argument('--cli', type=Path, default=Path('target/release/procon'))
p.add_argument('--case-file', type=Path, default=Path('crates/bench/tests/fixtures/arena-701.json'))
p.add_argument('--candidate', default='coverage:budget=official')
p.add_argument('--kinds', default='P,S')
p.add_argument('--seven-agents', action='store_true')
p.add_argument('--seconds', type=int, default=4)
p.add_argument('--output-dir', type=Path, required=True)
a = p.parse_args()
a.output_dir.mkdir(parents=True, exist_ok=False)
case = json.loads(a.case_file.read_text())
problem = {key: case[key] for key in ('spots', 'fuelLimits', 'daySteps', 'busyThreshold', 'jammedThreshold')}
problem.update(case['map'])
problem['agentStarts'] = case['agents'][:]
if a.seven_agents:
    problem['agentStarts'] = (case['agents'] * 7)[:7]
    problem['fuelLimits'] = min(problem['fuelLimits'], 12)
problem['daySeconds'] = [a.seconds] * len(problem['daySteps'])
config_path = a.output_dir / 'config.json'
config_path.write_text(json.dumps({'problem': problem, 'teams': [{'token': 'local-test'}]}, indent=2) + '\n')
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
records = []
class Proxy(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_GET(self):
        self.forward()
    def do_POST(self):
        self.forward()
    def forward(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0))) or None
        req = urllib.request.Request(f'http://127.0.0.1:{port}{self.path}', data=body,
            headers={'Procon-Token': 'local-test', 'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(req, timeout=3) as response:
                status, raw = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read()
        records.append({'method': self.command, 'path': self.path,
            'body': json.loads(body) if body else None, 'status': status,
            'response': json.loads(raw) if raw else None})
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(raw)

proxy = ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
thread = threading.Thread(target=proxy.serve_forever, daemon=True)
thread.start()
with (a.output_dir / 'server.log').open('w') as server_log, (a.output_dir / 'cli.log').open('w') as cli_log:
    server = subprocess.Popen([str(a.server_binary.resolve()), '-config', str(config_path),
        '-addr', f'127.0.0.1:{port}', '-kind-deadline', '3s', '-match-start-delay', '1s'],
        stdout=server_log, stderr=subprocess.STDOUT)
    code = None
    try:
        time.sleep(.3)
        env = os.environ.copy()
        env.update(PROCON_TOKEN='local-test', RUST_LOG='info')
        command = [str(a.cli.resolve()), 'coordinator', '--server',
            f'http://127.0.0.1:{proxy.server_port}', '--candidate', a.candidate,
            '--kinds', a.kinds, '--exit-after-match']
        result = subprocess.run(command, env=env, stdout=cli_log, stderr=subprocess.STDOUT,
            timeout=20 + a.seconds * len(problem['daySteps']))
        code = result.returncode
    finally:
        server.terminate()
        server.wait(timeout=5)
        proxy.shutdown()
        proxy.server_close()
        revisions = [r['response']['revision'] for r in records
            if r['method'] == 'POST' and r['path'] == '/' and isinstance(r['response'], dict)
            and 'revision' in r['response']]
        summary = {'source_sha': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
            'server_sha256': hashlib.sha256(a.server_binary.read_bytes()).hexdigest(),
            'cli_sha256': hashlib.sha256(a.cli.read_bytes()).hexdigest(),
            'candidate': a.candidate, 'kinds': a.kinds, 'exit_code': code,
            'days': len(problem['daySteps']), 'revisions': revisions,
            'limitation': 'Local official binary, single team, shortened windows; not arena score parity.'}
        (a.output_dir / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
        (a.output_dir / 'requests.json').write_text(json.dumps(records, indent=2) + '\n')
assert code == 0 and len(revisions) == len(problem['daySteps']) and all(r >= 0 for r in revisions), summary
print(json.dumps(summary))
