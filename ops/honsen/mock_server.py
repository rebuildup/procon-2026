#!/usr/bin/env python3
"""A stand-in contest server for smoke tests where no official server binary exists.

The organisers ship `procon-server-*` for macOS and Linux only, so a Windows
machine has nothing to practise against. This script speaks the same four
endpoints (GET /setting, POST /agent, GET /, POST /) with the same token
header and the same 401 / 403 envelopes, driven by one bench case file.

It checks the shape of what it receives (token, agent count, JSON) and
accepts every well-formed plan: it does not simulate the game, agents never
move, and no score exists. It proves that a binary starts, talks the
protocol, answers every day and writes its records — nothing about plan
quality. Use ops/honsen/e2e.py with the official server for that.

Listens on 127.0.0.1 only. Exits by itself two seconds after the last day.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
import time

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument('--case-file', type=Path, required=True, help='bench case JSON (map/spots/agents/daySteps/...)')
p.add_argument('--port', type=int, default=18080)
p.add_argument('--token', default='local-e2e-token')
p.add_argument('--days', type=int, default=3)
p.add_argument('--day-seconds', type=int, default=6)
p.add_argument('--start-delay', type=int, default=2, help='seconds between POST /agent and the first day')
p.add_argument('--summary', type=Path, help='write what the server saw to this JSON file on exit')
a = p.parse_args()

case = json.loads(a.case_file.read_text(encoding='utf-8'))
days = min(a.days, len(case['daySteps']))
lock = threading.Lock()
state = {'starts_at': 0, 'kinds': None, 'revision': 0, 'accepted_days': [], 'requests': 0, 'bad_token': 0}


def setting():
    return {'startsAt': state['starts_at'], 'daySeconds': [a.day_seconds] * days,
            'daySteps': case['daySteps'][:days], 'map': case['map'], 'spots': case['spots'],
            'agents': case['agents'], 'fuelLimits': case['fuelLimits'], 'players': 2,
            'busyThreshold': case['busyThreshold'], 'jammedThreshold': case['jammedThreshold']}


def current_day(now):
    """The 0-based day at `now`, or None before the start / after the end."""
    if not state['starts_at'] or now < state['starts_at']:
        return None
    day = int(now - state['starts_at']) // a.day_seconds
    return day if day < days else None


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, status, payload):
        raw = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def handle_request(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        path = self.path.split('?')[0]
        now = time.time()
        with lock:
            state['requests'] += 1
            if self.headers.get('Procon-Token') != a.token:
                state['bad_token'] += 1
                return self.reply(401, {'code': 'AuthError', 'message': 'invalid token'})
            day = current_day(now)
            if self.command == 'GET' and path == '/setting':
                return self.reply(200, setting())
            if self.command == 'POST' and path == '/agent':
                if state['starts_at']:
                    return self.reply(403, {'code': 'AccessTimeError', 'message': 'agent kind submission is closed'})
                kinds = json.loads(body)
                if len(kinds) != len(case['agents']):
                    return self.reply(400, {'message': 'wrong number of agents'})
                state['kinds'] = kinds
                state['starts_at'] = int(now) + a.start_delay
                return self.reply(200, {})
            if path == '/' and day is None:
                return self.reply(403, {'code': 'AccessTimeError', 'message': 'match is not running'})
            if self.command == 'GET' and path == '/':
                agents = [{'kind': kind, 'pos': pos, 'fuel': case['fuelLimits']}
                          for kind, pos in zip(state['kinds'], case['agents'])]
                return self.reply(200, {'endsAt': state['starts_at'] + (day + 1) * a.day_seconds, 'day': day,
                                        'agents': agents, 'others': [], 'traffics': []})
            if self.command == 'POST' and path == '/':
                plan = json.loads(body)
                well_formed = (isinstance(plan, list) and len(plan) == len(case['agents'])
                               and all(isinstance(row, list) and all(isinstance(x, int) for x in row) for row in plan))
                if not well_formed:
                    return self.reply(200, {'revision': -1})
                state['revision'] += 1
                if day not in state['accepted_days']:
                    state['accepted_days'].append(day)
                return self.reply(200, {'revision': state['revision']})
            return self.reply(404, {'message': 'not found'})

    do_GET = handle_request
    do_POST = handle_request


server = ThreadingHTTPServer(('127.0.0.1', a.port), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
print(f'mock contest server on http://127.0.0.1:{a.port} ({days} days x {a.day_seconds}s)', flush=True)
deadline = time.time() + 600
while time.time() < deadline:
    with lock:
        started = state['starts_at']
    if started and time.time() > started + days * a.day_seconds + 2:
        break
    time.sleep(0.2)
server.shutdown()
with lock:
    summary = {'days': days, 'accepted_days': sorted(state['accepted_days']), 'requests': state['requests'],
               'bad_token': state['bad_token'], 'kinds': state['kinds']}
if a.summary:
    a.summary.write_text(json.dumps(summary) + '\n', encoding='utf-8')
print(json.dumps(summary), flush=True)
raise SystemExit(0 if summary['accepted_days'] == list(range(days)) else 1)
