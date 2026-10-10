#!/usr/bin/env bash
# Build one pinned commit into $HONSEN_HOME/releases/<name>/ for `honsen`.
#
#   build-release.sh <name> <git-sha> [candidate] [kinds-args]
#
# The commit is checked out into its own detached worktree and built with its
# own CARGO_TARGET_DIR, so the binary cannot be a stale artefact of another
# checkout (a shared target dir has produced exactly that before). The source
# must be a commit: uncommitted work cannot be released.
set -euo pipefail

NAME="${1:?usage: build-release.sh <name> <git-sha> [candidate] [kinds-args]}"
SHA_IN="${2:?usage: build-release.sh <name> <git-sha> [candidate] [kinds-args]}"
CANDIDATE="${3:-fleet:budget=official:budget_ms=59000:adapt=off}"
KINDS_ARGS="${4---kinds-auto}"
HONSEN_HOME="${HONSEN_HOME:-$HOME/.procon-honsen}"
DEST="$HONSEN_HOME/releases/$NAME"
BUILD_ROOT="$HONSEN_HOME/build"

REPO="$(git rev-parse --show-toplevel)"
SHA="$(git -C "$REPO" rev-parse --verify "${SHA_IN}^{commit}")"
[ ! -e "$DEST" ] || { echo "release already exists: $DEST (pick another name)" >&2; exit 1; }

SRC="$BUILD_ROOT/src-${SHA:0:12}"
TARGET="$BUILD_ROOT/target-${SHA:0:12}"
mkdir -p "$BUILD_ROOT"
if [ ! -d "$SRC" ]; then
    git -C "$REPO" worktree add --detach "$SRC" "$SHA" >/dev/null
fi
[ "$(git -C "$SRC" rev-parse HEAD)" = "$SHA" ] || { echo "worktree $SRC is not at $SHA" >&2; exit 1; }
[ -z "$(git -C "$SRC" status --porcelain)" ] || { echo "worktree $SRC is dirty" >&2; exit 1; }

echo "building $SHA -> $DEST"
(cd "$SRC" && CARGO_TARGET_DIR="$TARGET" cargo build --release --locked -p cli)

mkdir -p "$DEST"
cp "$TARGET/release/procon" "$DEST/procon"
chmod 755 "$DEST/procon"
DIGEST="$(shasum -a 256 "$DEST/procon" | awk '{print $1}')"
"$DEST/procon" version >/dev/null
cat >"$DEST/MANIFEST" <<EOF
SOURCE_SHA=$SHA
BINARY_SHA256=$DIGEST
CANDIDATE='$CANDIDATE'
KINDS_ARGS='$KINDS_ARGS'
DESCRIPTION='built $(date '+%Y-%m-%d %H:%M %z') on $(uname -sm), rustc $(rustc -V | awk '{print $2}')'
EOF
echo "release $NAME"
echo "  source $SHA"
echo "  sha256 $DIGEST"
echo "next: honsen use $NAME"
