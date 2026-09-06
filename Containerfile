# procon-2026 — development sandbox image.
# Multi-stage: toolchain build image + lean dev image that mounts source.

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