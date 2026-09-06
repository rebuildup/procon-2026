# procon-2026 — sandbox images.
#
# Stages:
#   - toolchain : Rust + Bun + Node toolchain image used by devcontainer + CI.
#                 Pre-warms the Cargo registry for the workspace.
#   - dev       : extends `toolchain`, default command is a banner. devcontainer
#                 mounts the source tree at runtime and runs `just` / `cargo`.
#   - coordinator : multi-stage production image. Builds the `procon` release
#                   binary on a slim Rust image and packages it on top of
#                   `debian:bookworm-slim` for the contest venue. Entry point
#                   is `procon coordinator` (LAN match-server auto-detect +
#                   /agent POST with retry/timeout, all env-var driven).

# syntax=docker/dockerfile:1.7
# ----------------------------------------------------------------
# Stage 1: toolchain image — caches registry + system deps.
# ----------------------------------------------------------------
FROM rust:1.85-bookworm AS toolchain

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    CARGO_TERM_COLOR=always \
    RUST_BACKTRACE=1

# System deps for WebGPU / build helpers.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev \
        ca-certificates \
        curl \
        git \
        ripgrep \
 && rm -rf /var/lib/apt/lists/*

# Bun (pinned, reproducible).
RUN curl -fsSL https://bun.sh/install | bash -s -- "bun-v1.4.2"
ENV PATH=/root/.bun/bin:$PATH

# Node 22 LTS (fallback for tools that need npm).
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && rm -rf /var/lib/apt/lists/*

# Pre-warm Cargo registry for the workspace to keep cold builds small.
WORKDIR /workspace
COPY Cargo.toml Cargo.lock* ./
COPY crates/domain/Cargo.toml crates/domain/Cargo.toml
COPY crates/protocol/Cargo.toml crates/protocol/Cargo.toml
COPY crates/engine/Cargo.toml crates/engine/Cargo.toml
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml
RUN mkdir -p \
        crates/domain/src crates/protocol/src crates/engine/src crates/cli/src \
 && for d in crates/domain crates/protocol crates/engine crates/cli; do \
        echo "fn main() {}" > "$d/src/lib.rs"; \
    done \
 && echo "fn main() {}" > crates/cli/src/main.rs \
 && cargo fetch

# ----------------------------------------------------------------
# Stage 2: dev image — minimal, mounts source at runtime.
# ----------------------------------------------------------------
FROM toolchain AS dev

WORKDIR /workspace
ENV PATH=/root/.bun/bin:$PATH

# Default entrypoint prints a banner. Override with `just` or `cargo` / `bun`.
CMD ["bash", "-lc", "echo 'procon-2026 dev image ready. Try: just --list'"]

# ----------------------------------------------------------------
# Stage 3: builder — release build of the `procon` binary.
# Slim variant keeps the image small and avoids dev tooling we don't need.
# ----------------------------------------------------------------
FROM rust:1.85-slim-bookworm AS coordinator-builder

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    CARGO_TERM_COLOR=always \
    RUST_BACKTRACE=1

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev \
        ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Copy the workspace manifest layer so the dependency graph can be cached
# separately from the source tree (best-practice multi-stage layout).
COPY Cargo.toml Cargo.lock ./
COPY crates/domain/Cargo.toml crates/domain/Cargo.toml
COPY crates/protocol/Cargo.toml crates/protocol/Cargo.toml
COPY crates/engine/Cargo.toml crates/engine/Cargo.toml
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml
RUN mkdir -p \
        crates/domain/src crates/protocol/src crates/engine/src crates/cli/src \
 && for d in crates/domain crates/protocol crates/engine crates/cli; do \
        echo "fn main() {}" > "$d/src/lib.rs"; \
    done \
 && echo "fn main() {}" > crates/cli/src/main.rs \
 && cargo fetch --locked

# Now overlay the real source and build the release binary. We only ship
# the `procon` binary — the workspace as a whole stays buildable for tests.
COPY crates/ ./crates/
RUN cargo build --release --locked -p cli \
 && cp target/release/procon /workspace/procon \
 && strip /workspace/procon

# ----------------------------------------------------------------
# Stage 4: coordinator (production runtime).
# `debian:bookworm-slim` — small, with libc + ca-certificates for HTTPS.
# Distroless alternatives (`gcr.io/distroless/cc-debian12`) work too but
# have no shell for ops debugging; we keep slim for the contest venue.
# ----------------------------------------------------------------
FROM debian:bookworm-slim AS coordinator

LABEL org.opencontainers.image.title="procon-2026 coordinator" \
      org.opencontainers.image.description="Contest venue solver container. LAN match-server auto-detect + /agent POST with retry/timeout." \
      org.opencontainers.image.source="https://github.com/rebuildup/procon-2026" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"

# tini for clean signal forwarding (SIGTERM -> children).
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        tini \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Non-root runtime user.
RUN groupadd --system procon \
 && useradd --system --gid procon --home /app --shell /usr/sbin/nologin procon

COPY --from=coordinator-builder /workspace/procon /app/procon
RUN chown procon:procon /app/procon

USER procon

# Default to the coordinator subcommand. All behaviour is env-driven.
# See docs/ops/container.md for the full env-var reference.
ENV PROCON_SERVER=localhost:7878 \
    PROCON_HTTP_TIMEOUT_MS=5000 \
    PROCON_HTTP_RETRIES=3 \
    PROCON_HTTP_BACKOFF_MS=250 \
    RUST_LOG=info

ENTRYPOINT ["/usr/bin/tini", "--", "/app/procon"]
CMD ["coordinator", "--dry-run"]

# No HTTP listener here — the coordinator is an outbound HTTP client.
# EXPOSE is reserved for future engine server work (planned on 7878).
# EXPOSE 7878