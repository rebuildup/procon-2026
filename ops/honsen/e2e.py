#!/usr/bin/env python3
"""End-to-end check of the pinned coordinator binary against the official simple server.

Starts the organiser-distributed `procon-server-*` binary with one bench case,
puts a recording proxy in front of it, runs `procon coordinator` through the
proxy and judges the match from what the *server* answered: a day counts as
accepted only when a `POST /` got `revision >= 0` back.

The proxy can inject faults (see --fault) and the harness can kill and restart
the coordinator mid-match (see --kill-day), so the same script covers both the
plain full-match runs and the failure drills listed in docs/ops/honsen-runbook.md.

Nothing here talks to anything but 127.0.0.1.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

TOKEN = 'local-e2e-token'

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument('--server-binary', type=Path, required=True)
p.add_argument('--cli', type=Path, required=True)
p.add_argument('--case-file', type=Path, required=True, help='bench case JSON (map/spots/agents/daySteps/...)')
p.add_argument('--candidate', default='fleet:budget=official:budget_ms=10000')
p.add_argument('--kinds', default='auto', help="'auto' for --kinds-auto, 'none' to omit, or a pattern such as S,P")
p.add_argument('--seconds', type=int, default=0, help='override every daySeconds entry (0 keeps the case file)')
p.add_argument('--days', type=int, default=0, help='truncate the match to this many days (0 keeps all)')
p.add_argument('--teams', type=int, default=10, help='teams registered on the server (only ours plays)')
p.add_argument('--kind-deadline', default='5s')
p.add_argument('--match-start-delay', default='5s')
p.add_argument('--fault', action='append', default=[],
               help='JSON fault, e.g. \'{"method":"POST","path":"/","day":2,"action":"drop_after","times":1}\'. '
                    'actions: drop_before, drop_after, status (with "status"), badjson, delay (with "delay_ms")')
p.add_argument('--kill-day', type=int, default=-1,
               help='kill -9 the coordinator once, when it fetches this day, then start it again')
p.add_argument('--kill-after-post', action='store_true',
               help='with --kill-day: kill after that day was accepted instead of before it was answered')
p.add_argument('--expect-missed-days', type=int, default=0,
               help='days allowed to end without an accepted plan (for drills that lose a day on purpose)')
p.add_argument('--record', action='store_true',
               help='run with --record-dir <output-dir>/records and check the record against what the proxy saw')
p.add_argument('--slot-window-ms', type=int, default=0,
               help='set PROCON_SUPPLY_SLOT_WINDOW_MS for the coordinator (0 leaves it unset)')
p.add_argument('--output-dir', type=Path, required=True)
a = p.parse_args()

a.output_dir.mkdir(parents=True, exist_ok=False)
case = json.loads(a.case_file.read_text())
problem = {key: case[key] for key in ('spots', 'fuelLimits', 'daySteps', 'busyThreshold', 'jammedThreshold')}
problem.update(case['map'])
problem['agentStarts'] = case['agents'][:]
problem['daySeconds'] = case.get('daySeconds') or [60] * len(problem['daySteps'])
if a.days:
    problem['daySteps'] = problem['daySteps'][:a.days]
    problem['daySeconds'] = problem['daySeconds'][:a.days]
if a.seconds:
    problem['daySeconds'] = [a.seconds] * len(problem['daySteps'])
days_total = len(problem['daySteps'])
teams = [{'name': 'us', 'token': TOKEN}] + [{'name': f'idle{i}', 'token': f'idle-token-{i}'} for i in range(1, a.teams)]
config_path = a.output_dir / 'config.json'
config_path.write_text(json.dumps({'problem': problem, 'teams': teams}, indent=2) + '\n')

with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]

faults = [dict(json.loads(f), fired=0) for f in a.fault]
records = []
setting_seen = {}
lock = threading.Lock()
current_day = {'value': -1}
kill_request = threading.Event()
kill_done = threading.Event()


def pick_fault(method, path):
    for fault in faults:
        if fault.get('method', method) != method or fault.get('path', path) != path:
            continue
        if 'day' in fault and fault['day'] != current_day['value']:
            continue
        if fault['fired'] >= fault.get('times', 1):
            continue
        fault['fired'] += 1
        return fault
    return None


class Proxy(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        self.forward()

    def do_POST(self):
        self.forward()

    def drop(self):
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.close_connection = True

    def forward(self):
        received = time.time()
        body = self.rfile.read(int(self.headers.get('Content-Length', 0))) or None
        path = self.path.split('?')[0]
        with lock:
            fault = pick_fault(self.command, path)
        action = fault['action'] if fault else None
        record = {'t': received, 'method': self.command, 'path': path, 'day_context': current_day['value'],
                  'fault': action, 'has_token_header': self.headers.get('Procon-Token') == TOKEN}
        if action == 'drop_before':
            record.update(status=None, response=None, forwarded=False)
            with lock:
                records.append(record)
            return self.drop()
        if action == 'status':
            record.update(status=fault['status'], response=None, forwarded=False)
            with lock:
                records.append(record)
            self.send_response(fault['status'])
            self.send_header('Content-Length', '0')
            self.end_headers()
            return None
        req = urllib.request.Request(f'http://127.0.0.1:{port}{self.path}', data=body, method=self.command,
                                     headers={'Procon-Token': self.headers.get('Procon-Token', ''),
                                              'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(req, timeout=10) as response:
                status, raw = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read()
        except OSError as error:
            record.update(status=None, response=str(error), forwarded=True)
            with lock:
                records.append(record)
            return self.drop()
        try:
            parsed = json.loads(raw) if raw else None
        except ValueError:
            parsed = raw.decode('utf-8', 'replace')
        record.update(status=status, forwarded=True, answered=time.time())
        if self.command == 'POST' and path == '/':
            record['response'] = parsed
            record['body_agents'] = len(json.loads(body)) if body else 0
            record['body'] = json.loads(body) if body else None
        elif self.command == 'GET' and path == '/' and isinstance(parsed, dict):
            record['response'] = {'day': parsed.get('day'), 'endsAt': parsed.get('endsAt'),
                                  'agents': parsed.get('agents')}
            with lock:
                current_day['value'] = parsed.get('day', current_day['value'])
        elif self.command == 'POST':
            record['response'] = parsed
            record['body'] = json.loads(body) if body else None
        else:
            record['response'] = {'startsAt': parsed.get('startsAt')} if isinstance(parsed, dict) else parsed
            if path == '/setting' and isinstance(parsed, dict):
                setting_seen.setdefault('first', parsed)
        with lock:
            records.append(record)
        if a.kill_day >= 0 and not kill_request.is_set():
            hit_get = (not a.kill_after_post and self.command == 'GET' and path == '/'
                       and isinstance(parsed, dict) and parsed.get('day') == a.kill_day)
            hit_post = (a.kill_after_post and self.command == 'POST' and path == '/'
                        and current_day['value'] == a.kill_day and status == 200)
            if hit_get or hit_post:
                kill_request.set()
                if hit_get:
                    kill_done.wait(timeout=5)
                    return self.drop()
        if action == 'drop_after':
            return self.drop()
        if action == 'delay':
            time.sleep(fault['delay_ms'] / 1000)
        if action == 'badjson':
            raw = b'<html>502 Bad Gateway</html>'
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)
        return None


proxy = ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
threading.Thread(target=proxy.serve_forever, daemon=True).start()

env = os.environ.copy()
for key in list(env):
    if key.startswith('PROCON_'):
        del env[key]
env.update(PROCON_TOKEN=TOKEN, RUST_LOG='info', RUST_BACKTRACE='1', NO_COLOR='1',
           PROCON_RESUME_RUNNING_MATCH='1')
if a.slot_window_ms:
    env['PROCON_SUPPLY_SLOT_WINDOW_MS'] = str(a.slot_window_ms)
command = [str(a.cli.resolve()), 'coordinator', '--server', f'http://127.0.0.1:{proxy.server_port}',
           '--candidate', a.candidate, '--exit-after-match',
           '--state-output', str(a.output_dir / 'state.json')]
if a.record:
    command += ['--record-dir', str(a.output_dir / 'records'), '--http-log-dir', str(a.output_dir / 'logs')]
if a.kinds == 'auto':
    command.append('--kinds-auto')
elif a.kinds != 'none':
    command += ['--kinds', a.kinds]

exit_codes = []
started = time.time()
budget = 60 + sum(problem['daySeconds'])
with (a.output_dir / 'server.log').open('w') as server_log, (a.output_dir / 'cli.log').open('w') as cli_log:
    server = subprocess.Popen([str(a.server_binary.resolve()), '-config', str(config_path),
                               '-addr', f'127.0.0.1:{port}', '-kind-deadline', a.kind_deadline,
                               '-match-start-delay', a.match_start_delay],
                              stdout=server_log, stderr=subprocess.STDOUT)
    try:
        time.sleep(0.3)
        cli = subprocess.Popen(command, env=env, stdout=cli_log, stderr=subprocess.STDOUT)
        restarted = False
        while time.time() - started < budget:
            if kill_request.is_set() and not restarted:
                cli.kill()
                exit_codes.append(('killed', cli.wait()))
                kill_done.set()
                cli_log.write('\n===== harness: coordinator killed, restarting =====\n')
                cli_log.flush()
                cli = subprocess.Popen(command, env=env, stdout=cli_log, stderr=subprocess.STDOUT)
                restarted = True
            if cli.poll() is not None:
                break
            time.sleep(0.1)
        if cli.poll() is None:
            cli.kill()
            exit_codes.append(('timeout', cli.wait()))
        else:
            exit_codes.append(('exit', cli.returncode))
    finally:
        server.terminate()
        server.wait(timeout=5)
        proxy.shutdown()
        proxy.server_close()

days = []
for day in range(days_total):
    gets = [r for r in records if r['method'] == 'GET' and r['path'] == '/' and isinstance(r.get('response'), dict)
            and r['response'].get('day') == day]
    posts = [r for r in records if r['method'] == 'POST' and r['path'] == '/' and r['day_context'] == day]
    accepted = [r for r in posts if r.get('status') == 200 and isinstance(r.get('response'), dict)
                and r['response'].get('revision', -1) >= 0]
    entry = {'day': day, 'day_seconds': problem['daySeconds'][day], 'post_attempts': len(posts),
             'accepted': len(accepted),
             'revisions': [r['response'].get('revision') for r in posts if isinstance(r.get('response'), dict)]}
    if gets and accepted:
        first_get = gets[0]
        last_ok = accepted[-1]
        entry['answer_seconds'] = round(last_ok['answered'] - first_get['t'], 3)
        entry['margin_seconds'] = round(first_get['response']['endsAt'] - last_ok['answered'], 3)
    days.append(entry)
missed = [d['day'] for d in days if not d['accepted']]
kinds = [r.get('body') for r in records if r['method'] == 'POST' and r['path'] == '/agent' and r.get('status') == 200]
final_exit = exit_codes[-1] if exit_codes else None
summary = {
    'case': a.case_file.name,
    'size': f"{problem['width']}x{problem['height']}",
    'agents': len(problem['agentStarts']),
    'days_total': days_total,
    'source_sha': subprocess.run(['git', 'rev-parse', 'HEAD'], capture_output=True, text=True).stdout.strip(),
    'server_sha256': hashlib.sha256(a.server_binary.read_bytes()).hexdigest(),
    'cli_sha256': hashlib.sha256(a.cli.read_bytes()).hexdigest(),
    'candidate': a.candidate, 'kinds_flag': a.kinds, 'kinds_accepted': kinds[-1] if kinds else None,
    'faults': [{k: v for k, v in f.items()} for f in faults],
    'kill_day': a.kill_day, 'kill_after_post': a.kill_after_post,
    'exit_codes': exit_codes,
    'token_header_on_every_request': all(r['has_token_header'] for r in records),
    'token_in_cli_log': TOKEN in (a.output_dir / 'cli.log').read_text(errors='replace'),
    'days': days, 'missed_days': missed,
    'wall_seconds': round(time.time() - started, 1),
    'limitation': 'Official simple server on localhost, idle opponents; proves protocol acceptance, not score.',
}
ok = (final_exit is not None and final_exit[0] == 'exit' and final_exit[1] == 0
      and len(missed) <= a.expect_missed_days and kinds)


def check_http_log(runs, problems):
    """Compare logs/<run-id>/http.jsonl and the console copy with what crossed the proxy."""
    exchanges = []
    console = (a.output_dir / 'cli.log').read_text(errors='replace')
    for run in runs:
        path = a.output_dir / 'logs' / run.name / 'http.jsonl'
        if not path.is_file():
            problems.append(f'{run.name}: http.jsonl missing')
            continue
        text = path.read_text()
        if TOKEN in text:
            problems.append(f'{run.name}: http.jsonl contains the token')
        for raw in text.splitlines():
            if not raw.strip():
                continue
            try:
                entry = json.loads(raw)
            except ValueError:
                problems.append(f'{run.name}: unparsable http.jsonl line')
                continue
            if entry['type'] == 'exchange':
                exchanges.append(entry)
                head = f"[HTTP #{entry['n']}] {entry['ts']} {entry['method']} {entry['url']} -> "
                if head not in console:
                    problems.append(f"{run.name}: request #{entry['n']} is in http.jsonl but not on the console")
                for side in ('request_body', 'response_body'):
                    if entry[side] and entry[side] not in console:
                        problems.append(f"{run.name}: {side} of #{entry['n']} is not on the console in full")
    if '[MASKED]' not in console:
        problems.append('console shows no masked token header')
    # The killed process may have had one request in flight; allow that much.
    slack = 1 if a.kill_day >= 0 else 0
    if not len(records) - slack <= len(exchanges) <= len(records):
        problems.append(f'http.jsonl has {len(exchanges)} exchanges, the proxy saw {len(records)} requests')
    if slack == 0:
        for entry, seen in zip(exchanges, records):
            where = f"request #{entry['n']}"
            if entry['method'] != seen['method'] or not entry['url'].endswith(seen['path']):
                problems.append(f'{where}: logged {entry["method"]} {entry["url"]}, proxy saw {seen["method"]} {seen["path"]}')
            if seen.get('body') is not None and json.loads(entry['request_body'] or 'null') != seen['body']:
                problems.append(f'{where}: logged request body differs from what the proxy received')
            if not seen.get('fault') and entry['status'] != seen.get('status'):
                problems.append(f'{where}: logged status {entry["status"]}, proxy answered {seen.get("status")}')
            if seen.get('fault') in ('drop_before', 'drop_after') and entry['result'] != 'failed':
                problems.append(f'{where}: a dropped connection was logged as {entry["result"]}')


def check_record():
    """Compare the run directories with what crossed the proxy."""
    problems = []
    runs = sorted(d for d in (a.output_dir / 'records').glob('*') if d.is_dir())
    plans, results, observed, lines = {}, {}, set(), 0
    for run in runs:
        for name in ('manifest.json', 'setting.json', 'events.jsonl', 'coordinator.log', 'state.json'):
            if not (run / name).is_file():
                problems.append(f'{run.name}: {name} missing')
            elif TOKEN in (run / name).read_text(errors='replace'):
                problems.append(f'{run.name}: {name} contains the token')
        if (run / 'setting.json').is_file():
            saved = json.loads((run / 'setting.json').read_text())
            served = dict(setting_seen.get('first', {}), startsAt=saved.get('startsAt'))
            if saved != served:
                problems.append(f'{run.name}: setting.json differs from GET /setting')
        events = (run / 'events.jsonl').read_text().splitlines() if (run / 'events.jsonl').is_file() else []
        for raw in events:
            if not raw.strip():
                continue
            try:
                event = json.loads(raw)
            except ValueError:
                problems.append(f'{run.name}: unparsable event line')
                continue
            lines += 1
            if event['type'] == 'plan':
                plans[event['day']] = event['actions']
            elif event['type'] == 'day_result':
                results[event['day']] = (event['status'], event['revision'])
            elif event['type'] == 'day_state':
                observed.add(event['observed']['day'])
    for day in range(days_total):
        posts = [r for r in records if r['method'] == 'POST' and r['path'] == '/' and r['day_context'] == day
                 and r.get('body') is not None]
        accepted = [r for r in posts if r.get('status') == 200 and isinstance(r.get('response'), dict)
                    and r['response'].get('revision', -1) >= 0]
        if posts and plans.get(day) != posts[-1]['body']:
            problems.append(f'day {day}: recorded plan differs from the POST body')
        if posts and len(plans.get(day, [])) != len(problem['agentStarts']):
            problems.append(f'day {day}: plan does not cover every agent')
        if accepted:
            want = ('accepted', accepted[-1]['response']['revision'])
            # A day whose verdict arrived just as the process was killed has a plan but no result.
            if day in results and tuple(results[day]) != want:
                problems.append(f'day {day}: recorded verdict {results[day]} != server {want}')
            if day not in results and a.kill_day != day:
                problems.append(f'day {day}: accepted by the server but no verdict recorded')
    show = subprocess.run([str(a.cli.resolve()), 'record', 'show', str(runs[-1])], capture_output=True, text=True) \
        if runs else None
    if show is None or show.returncode != 0:
        problems.append('procon record show failed')
    check_http_log(runs, problems)
    return {'runs': [r.name for r in runs], 'event_lines': lines, 'days_with_plan': sorted(plans),
            'days_observed': sorted(observed), 'continues': [json.loads((r / 'manifest.json').read_text())
                                                             .get('continues_run_id') for r in runs],
            'problems': problems}


if a.record:
    summary['record'] = check_record()
    ok = ok and not summary['record']['problems']
summary['verdict'] = 'PASS' if ok else 'FAIL'
(a.output_dir / 'summary.json').write_text(json.dumps(summary, indent=2, ensure_ascii=False) + '\n')
(a.output_dir / 'requests.json').write_text(json.dumps(records, indent=1, ensure_ascii=False) + '\n')
print(json.dumps({k: summary[k] for k in ('verdict', 'case', 'size', 'agents', 'days_total', 'missed_days',
                                          'exit_codes', 'wall_seconds', 'kinds_accepted', 'record')
                  if k in summary}, ensure_ascii=False))
for d in days:
    print('  day', d['day'], 'accepted' if d['accepted'] else 'MISSED', 'posts=%d' % d['post_attempts'],
          'answer=%ss' % d.get('answer_seconds'), 'margin=%ss' % d.get('margin_seconds'))
sys.exit(0 if ok else 1)
