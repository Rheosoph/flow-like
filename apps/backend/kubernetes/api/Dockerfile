# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.93-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && rustup toolchain install nightly-2025-12-15 \
    && rustup default nightly-2025-12-15

WORKDIR /app

COPY . .

# Remove packages that require edition 2024 or other backends
RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/runtime/d' Cargo.toml

# Limit build parallelism to reduce memory pressure during lance compilation
ENV CARGO_BUILD_JOBS=2
ENV RUSTFLAGS="--cfg tokio_unstable"

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-k8s-api \
    cargo build --release --package k8s-api && \
    cp /app/target/release/k8s-api /tmp/k8s-api

# ============================================================
# Stage 4: Runtime - minimal image for production
# ============================================================
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /tmp/k8s-api /app/api

RUN useradd -r -u 1000 flowlike && \
    chown -R flowlike:flowlike /app

USER flowlike

EXPOSE 8080 9090

ENV RUST_LOG=info

CMD ["/app/api"]
