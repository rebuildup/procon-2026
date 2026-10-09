#!/usr/bin/env python3
"""Reproduce phantom refueling against the organizer's local server binary."""
import argparse
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import time
import urllib.error
import urllib.request

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--server-binary', type=Path, required=True)
p.add_argument('--output-dir', type=Path, required=True)
a = p.parse_args()
a.output_dir.mkdir(parents=True, exist_ok=False)
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
cells = [[0] * 8 for _ in range(8)]
cells[0][1] = cells[0][2] = 1
config = {'problem': {'spots': [{'brand': 0, 'pos': 2, 'stocks': 4}],
    'agentStarts': [0, 2], 'fuelLimits': 2, 'daySteps': [4], 'daySeconds': [5],
    'busyThreshold': 3, 'jammedThreshold': 8, 'width': 8, 'height': 8, 'cells': cells},
    'teams': [{'token': 'local-test'}]}
config_path = a.output_dir / 'config.json'
config_path.write_text(json.dumps(config, indent=2) + '\n')
records = []
def request(path, body=None):
    req = urllib.request.Request(f'http://127.0.0.1:{port}{path}',
        data=None if body is None else json.dumps(body).encode(),
        headers={'Procon-Token': 'local-test', 'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(req, timeout=2) as response:
            raw = response.read()
            status, value = response.status, json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        status, value = error.code, json.loads(error.read())
    records.append({'path': path, 'body': body, 'status': status, 'response': value})
    return status, value

with (a.output_dir / 'server.log').open('w') as log:
    server = subprocess.Popen([str(a.server_binary.resolve()), '-config', str(config_path),
        '-addr', f'127.0.0.1:{port}', '-kind-deadline', '2s', '-match-start-delay', '1s'],
        stdout=log, stderr=subprocess.STDOUT)
    try:
        time.sleep(.3)
        assert request('/agent', [0, 1])[0] == 200
        time.sleep(2.9)
        assert request('/')[0] == 200
        assert request('/', [[2, 2, -1], [5, 5, -2]]) == (200, {'revision': -1})
        assert request('/', [[2, -2], [5, 5, -2]]) == (200, {'revision': 1})
    finally:
        server.terminate()
        server.wait(timeout=5)
        evidence = {'binary_sha256': hashlib.sha256(a.server_binary.read_bytes()).hexdigest(),
            'source_sha': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
            'requests': records}
        (a.output_dir / 'result.json').write_text(json.dumps(evidence, indent=2) + '\n')
print('Official server: phantom-contact plan rejected; corrected prefix accepted.')
