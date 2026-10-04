#!/usr/bin/env sh
set -eu

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

configured="$(git config --get core.hooksPath || true)"
if [ -n "$configured" ]; then
  echo "core.hooksPath is already configured ($configured); refusing to overwrite it." >&2
  echo "Point your existing pre-push dispatcher at: sh $repo_root/scripts/ci-local.sh" >&2
  exit 1
fi

hook_dir="$(git rev-parse --git-path hooks)"
hook="$hook_dir/pre-push"
mkdir -p "$hook_dir"

payload="$(mktemp)"
trap 'rm -f "$payload"' EXIT
cat >"$payload" <<'EOF'
#!/usr/bin/env sh
# rebuildup-local-ci-dispatch
set -eu
repo_root="$(git rev-parse --show-toplevel)"
exec sh "$repo_root/scripts/ci-local.sh"
EOF

if [ -e "$hook" ]; then
  # Byte-identical means we already installed it: re-running is a no-op.
  if cmp -s "$payload" "$hook"; then
    echo "Pre-push hook already installed: $hook"
    exit 0
  fi
  # Anything else — a foreign hook, or ours with local edits appended — is
  # refused rather than truncated. Matching the marker alone used to be
  # enough to overwrite, which silently deleted local additions.
  echo "Existing pre-push hook found at $hook; refusing to overwrite it." >&2
  if grep -q "rebuildup-local-ci-dispatch" "$hook" 2>/dev/null; then
    echo "It already contains the dispatcher plus local changes; drop your" >&2
    echo "additions, or keep the hook and delegate to:" >&2
    echo "  sh $repo_root/scripts/ci-local.sh" >&2
  fi
  exit 1
fi

cp "$payload" "$hook"
chmod +x "$hook"

echo "Installed pre-push hook: $hook"
