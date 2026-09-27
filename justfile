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
# Canonical evaluation — see docs/evaluation.md
#
# Offline solver comparison through `engine::MatchSim`. This is the path
# that produces the official score tuple (variety, daily_variety_sum,
# total_balls, response_time) and ranks lexicographically.
#
# It is NOT `procon tournament` (the real-server wire/protocol smoke
# harness — see `--protocol-progress` there). Run `just eval-list` to see
# the candidate axis.
# ----------------------------------------------------------------

# Default candidate set and baseline for `just eval`. Override per-call:
#   just eval-quiet
eval_candidates := "loop:iterations=32:initial=greedy,loop:iterations=128:initial=greedy,loop:initial=sweep-row,loop:initial=sweep-col,loop:iterations=64:initial=random:seed=42,greedy,wait"
eval_baseline := "wait"

# List the registered candidate algorithms and their parameter schemas.
eval-list:
    cargo run -q -p bench --bin procon-eval -- --list-candidates

# Run the standard evaluation batch: every bundled fixture against the
# default candidate set, ranked against the `wait` baseline.
#
# Artifacts: target/eval/{runs.jsonl,aggregate.json,report.md}
eval:
    PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" cargo run -q -p bench --bin procon-eval -- \
        --cases all \
        --candidates "{{eval_candidates}}" \
        --baseline "{{eval_baseline}}" \
        --output-dir target/eval \
        --markdown-out target/eval/report.md

# Same batch, artifacts only (no Markdown on stdout) — for scripted runs.
eval-quiet:
    PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" cargo run -q -p bench --bin procon-eval -- \
        --cases all \
        --candidates "{{eval_candidates}}" \
        --baseline "{{eval_baseline}}" \
        --output-dir target/eval \
        --markdown-out target/eval/report.md \
        --quiet

# ----------------------------------------------------------------
# Arena — unofficial practice server (Issue #179)
# See docs/contest/arena.md. The token is NEVER passed as an argument:
# inject it from Infisical as PROCON_ARENA_TOKEN, e.g.
#   infisical run --env=dev -- just arena-next --candidate 'multiloop:kmeans=4:seed=42'
# ----------------------------------------------------------------

# Wait for the next :00/:15/:30/:45 match, register inside the pre-match
# window, play exactly one match through the canonical coordinator, append
# evidence to target/arena/runs.jsonl, and exit. Extra flags are forwarded
# verbatim to `procon coordinator --arena --next` (e.g. `--candidate`,
# `--solver`), so keep values whitespace-free.
arena-next *args:
    PROCON_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" cargo run -q --release --bin procon -- coordinator --arena --next {{args}}

# Convert the arena's same-board replay JSON into an offline CaseSpec.
# Re-run it through the evaluator with:
#   cargo run -q -p bench --bin procon-eval -- --case-files <output> ...
arena-import input output:
    cargo run -q --bin procon -- arena import-case --input "{{input}}" --output "{{output}}"

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