#!/usr/bin/env sh
set -eu

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

echo "[pre-push] procon-2026 full local CI"
just ci
just protocol-consistency
