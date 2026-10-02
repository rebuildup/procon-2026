#!/usr/bin/env bash
# Restart the official contest server (procon-server-linux-amd64) with quick.json.
# Used during live verification of CLI ↔ server integration.
#
# Usage: ./scripts/restart-official-server.sh [PORT]
#   Default PORT=8080.
#   -kind-deadline is fixed at 60s (operator-friendly)
#   -match-start-delay is fixed at 1s (faster verification)
set -euo pipefail

PORT="${1:-8080}"
SERVER_BIN="/tmp/official-server/server-bin/procon-server-linux-amd64"
QUICK_JSON="/tmp/official-server/config/quick.json"

if [[ ! -x "$SERVER_BIN" ]]; then
    echo "FATAL: $SERVER_BIN not found" >&2
    exit 2
fi
if [[ ! -f "$QUICK_JSON" ]]; then
    echo "FATAL: $QUICK_JSON not found (set up: unzip the contest server bundle)" >&2
    exit 2
fi

# Kill any previous instance.
pkill -f procon-server-linux-amd64 2>/dev/null || true
sleep 1

# Start fresh.
cd "$(dirname "$SERVER_BIN")"
nohup "./$(basename "$SERVER_BIN")" \
    -config "$QUICK_JSON" \
    -addr ":${PORT}" \
    -kind-deadline 60s \
    -match-start-delay 1s \
    > "/tmp/procon-server-${PORT}.log" 2>&1 &

SERVER_PID=$!
echo "started: pid=${SERVER_PID} port=${PORT} log=/tmp/procon-server-${PORT}.log"

# Wait until /setting responds with proper AuthError (server is up).
for i in $(seq 1 20); do
    if curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT}/setting" 2>/dev/null | grep -q "401\|403\|200"; then
        echo "ready: ${i}*0.5s elapsed"
        exit 0
    fi
    sleep 0.5
done

echo "FATAL: server did not respond within 10s" >&2
exit 3
