#!/usr/bin/env bash
set -euo pipefail

readonly SOURCE_SHA="5ddb153a16d44bb34c2c3ed5cea4aa8044f95bf0"
readonly REPO_ROOT="$(git rev-parse --show-toplevel)"
readonly INSTALL_DIR="${1:?usage: build-pinned.sh INSTALL_DIR}"
readonly WORKTREE="$INSTALL_DIR/source"
WORKTREE_CREATED=0
BUILD_SUCCEEDED=0

cleanup() {
  if [[ "$WORKTREE_CREATED" == 1 && "$BUILD_SUCCEEDED" != 1 ]]; then
    git -C "$REPO_ROOT" worktree remove --force "$WORKTREE" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

mkdir -p "$INSTALL_DIR"
if [[ -e "$WORKTREE" ]]; then
  git -C "$WORKTREE" rev-parse --is-inside-work-tree >/dev/null
  [[ "$(git -C "$WORKTREE" rev-parse HEAD)" == "$SOURCE_SHA" ]]
  [[ -z "$(git -C "$WORKTREE" status --porcelain)" ]]
else
  git -C "$REPO_ROOT" worktree add --detach "$WORKTREE" "$SOURCE_SHA" >/dev/null
  WORKTREE_CREATED=1
fi
[[ "$(git -C "$WORKTREE" rev-parse HEAD)" == "$SOURCE_SHA" ]]

build_env=(env -i "PATH=$PATH" "HOME=${HOME:?HOME must be set for Cargo}" "TMPDIR=/tmp")
for name in CARGO_HOME RUSTUP_HOME RUSTUP_TOOLCHAIN HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY \
  http_proxy https_proxy all_proxy no_proxy SSL_CERT_FILE SSL_CERT_DIR; do
  if [[ -n "${!name:-}" ]]; then
    build_env+=("$name=${!name}")
  fi
done
"${build_env[@]}" cargo build --release --workspace --locked --manifest-path "$WORKTREE/Cargo.toml"

readonly BUILT_BINARY="$WORKTREE/target/release/procon"
readonly TEMP_BINARY="$INSTALL_DIR/procon.new"
install -m 0755 "$BUILT_BINARY" "$TEMP_BINARY"
"$TEMP_BINARY" coordinator --help >"$INSTALL_DIR/coordinator-help.txt"
grep -q -- '--kinds-auto' "$INSTALL_DIR/coordinator-help.txt"
grep -q -- '--exit-after-match' "$INSTALL_DIR/coordinator-help.txt"
mv "$TEMP_BINARY" "$INSTALL_DIR/procon"

python3 - "$SOURCE_SHA" "$INSTALL_DIR/procon" "$INSTALL_DIR/build.json" <<'PY'
import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path

source_sha, binary_path, manifest_path = sys.argv[1:]
rustc = subprocess.run(["rustc", "-Vv"], check=True, capture_output=True, text=True).stdout
cargo = subprocess.run(["cargo", "-V"], check=True, capture_output=True, text=True).stdout.strip()
manifest = {
    "source_commit_sha": source_sha,
    "binary_sha256": hashlib.sha256(Path(binary_path).read_bytes()).hexdigest(),
    "platform": platform.platform(),
    "build_flags": ["--release", "--workspace", "--locked"],
    "rustc_vv": rustc,
    "cargo_version": cargo,
    "release_profile": {
        "opt_level": 3,
        "lto": "thin",
        "codegen_units": 1,
        "strip": "debuginfo",
    },
}
path = Path(manifest_path)
temporary = path.with_suffix(".json.new")
temporary.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
temporary.chmod(0o644)
temporary.replace(path)
PY

rm -f "$INSTALL_DIR/coordinator-help.txt"
BUILD_SUCCEEDED=1
printf 'Built pinned solver source %s\n' "$SOURCE_SHA"
sha256sum "$INSTALL_DIR/procon"
