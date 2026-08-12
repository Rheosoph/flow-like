# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.97.1-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY . .

# Remove packages that require edition 2024 or have heavy dependencies
RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/executor/d' Cargo.toml

ENV CARGO_BUILD_JOBS=2

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-sink-services \
    cargo build --release --package docker-compose-sink-services && \
    cp /app/target/release/docker-compose-sink-services /tmp/docker-compose-sink-services

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /tmp/docker-compose-sink-services /app/sink-services

RUN useradd -r -u 1000 flowlike && \
    chown -R flowlike:flowlike /app

USER flowlike

ENV RUST_LOG=info

CMD ["/app/sink-services"]
