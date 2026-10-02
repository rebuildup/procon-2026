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

if [ -e "$hook" ] && ! grep -q "rebuildup-local-ci-dispatch" "$hook" 2>/dev/null; then
  echo "Existing pre-push hook found at $hook; refusing to overwrite it." >&2
  exit 1
fi

cat >"$hook" <<'EOF'
#!/usr/bin/env sh
# rebuildup-local-ci-dispatch
set -eu
repo_root="$(git rev-parse --show-toplevel)"
exec sh "$repo_root/scripts/ci-local.sh"
EOF
chmod +x "$hook"

echo "Installed pre-push hook: $hook"
