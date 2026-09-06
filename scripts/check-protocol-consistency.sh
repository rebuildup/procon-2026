#!/usr/bin/env bash
# Verify that PROTOCOL_VERSION matches between the Rust and TypeScript sides.
set -euo pipefail

# Resolve project root so the script works from any cwd.
project_root=$(git rev-parse --show-toplevel 2>/dev/null || dirname "$(dirname "$0")")
cd "$project_root"

rust_file="crates/protocol/src/lib.rs"
ts_file="packages/frontend/src/lib/protocol.ts"

# Prefer `rg`; fall back to `grep -Eo` if not installed.
if command -v rg >/dev/null 2>&1; then
  rust_major=$(rg -o 'major:[[:space:]]*[0-9]+' "$rust_file" | head -1 | rg -o '[0-9]+')
  ts_major=$(rg -o 'major:[[:space:]]*[0-9]+' "$ts_file" | head -1 | rg -o '[0-9]+')
else
  rust_major=$(grep -Eo 'major:[[:space:]]*[0-9]+' "$rust_file" | head -1 | grep -Eo '[0-9]+')
  ts_major=$(grep -Eo 'major:[[:space:]]*[0-9]+' "$ts_file" | head -1 | grep -Eo '[0-9]+')
fi

echo "Rust PROTOCOL_VERSION major=$rust_major"
echo "TS   PROTOCOL_VERSION major=$ts_major"

if [ -z "$rust_major" ] || [ -z "$ts_major" ]; then
  echo "::error::Could not parse PROTOCOL_VERSION in one of the files."
  exit 1
fi

if [ "$rust_major" != "$ts_major" ]; then
  echo "::error::PROTOCOL_VERSION major mismatch between Rust and TypeScript"
  exit 1
fi

echo "✓ protocol major version: $rust_major"