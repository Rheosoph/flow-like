# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.97.1-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY . .

# Remove packages that require edition 2024 or other backends
RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/runtime/d' Cargo.toml

# Limit build parallelism to reduce memory pressure during lance compilation
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-k8s-executor,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package k8s-executor \
      --package flow-like-execution-manager --bin k8s-executor --bin execution-slot && \
    cp /app/target/release/k8s-executor /tmp/k8s-executor && \
    cp /app/target/release/execution-slot /tmp/execution-slot

# ============================================================
# Stage 4: Runtime - minimal for sandboxed execution
# ============================================================
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 1000 executor

WORKDIR /app

COPY --from=builder /tmp/k8s-executor /app/executor
COPY --from=builder /tmp/execution-slot /app/execution-slot

RUN chown -R executor:executor /app

USER executor

ENV RUST_LOG=info

ENTRYPOINT ["/app/executor"]
