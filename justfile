# procon-2026 — deterministic validation entry point.
# Use `just` (https://github.com/casey/just). Fallback: read each recipe and run.

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# Common
default:
    @just --list

# ----------------------------------------------------------------
# Format
# ----------------------------------------------------------------

fmt:
    cargo fmt --all
    @echo "--- frontend ---"
    cd packages/frontend && bun run format

fmt-check:
    cargo fmt --all -- --check
    @echo "--- frontend ---"
    cd packages/frontend && bun run format:check

# ----------------------------------------------------------------
# Static checks (no tests)
# ----------------------------------------------------------------

check: fmt-check lint test-fast
    @echo "✓ just check passed"

lint:
    @echo "--- rust clippy ---"
    cargo clippy --workspace --all-targets --locked -- -D warnings
    @echo "--- frontend ---"
    cd packages/frontend && bun run check
    cd packages/frontend && bun run lint

# ----------------------------------------------------------------
# Tests
# ----------------------------------------------------------------

test: test-fast test-integration
    @echo "✓ just test passed"

test-fast:
    cargo test --workspace --locked
    @echo "--- frontend unit ---"
    cd packages/frontend && bun test

test-integration:
    @echo "--- protocol round-trip + replay determinism ---"
    cargo test --workspace --locked -- --ignored --nocapture

# ----------------------------------------------------------------
# Build
# ----------------------------------------------------------------

build:
    cargo build --release --workspace --locked
    @echo "--- frontend ---"
    cd packages/frontend && bun run build

# ----------------------------------------------------------------
# Local CI parity
# ----------------------------------------------------------------

ci: check test build
    @echo "✓ just ci passed (local CI parity)"

# ----------------------------------------------------------------
# Smoke
# ----------------------------------------------------------------

smoke-cli:
    cargo run -q --bin procon -- version
    cargo run -q --bin procon -- hello >/dev/null

# ----------------------------------------------------------------
# Warmup (pre-match cold/warm/smoke timing) — see crates/cli/src/warmup.rs
# ----------------------------------------------------------------

# Run the warmup timing harness against every built-in solver and emit a
# human-readable summary table. Use `--json` for CI / scripted consumers.
warmup:
    cargo run -q --bin procon -- warmup

# Run the warmup harness and emit a machine-readable JSON report. Output is
# suitable for piping to `jq` or archiving as a build artifact.
warmup-json:
    cargo run -q --bin procon -- warmup --json

# ----------------------------------------------------------------
# Protocol drift
# ----------------------------------------------------------------

protocol-consistency:
    bash scripts/check-protocol-consistency.sh

# ----------------------------------------------------------------
# Cleanup
# ----------------------------------------------------------------

clean:
    cargo clean
    cd packages/frontend && bun run clean || true