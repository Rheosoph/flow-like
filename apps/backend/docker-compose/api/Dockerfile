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

# packages/api embeds flow-like.config.json at compile time. Use the
# docker-compose config by default so self-hosted images do not inherit the
# repo-root hosted production config.
ARG FLOW_LIKE_CONFIG=apps/backend/docker-compose/flow-like.config.example.json
RUN cp "$FLOW_LIKE_CONFIG" /app/flow-like.config.json

# Remove packages that require edition 2024
RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/executor/d' Cargo.toml

# Limit build parallelism to reduce memory pressure during lance compilation
ENV CARGO_BUILD_JOBS=2

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-api,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package docker-compose-api && \
    cp /app/target/release/docker-compose-api /tmp/docker-compose-api

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

COPY --from=builder /tmp/docker-compose-api /app/api

RUN useradd -r -u 1000 flowlike && \
    chown -R flowlike:flowlike /app

USER flowlike

EXPOSE 8080 9090

ENV RUST_LOG=info

CMD ["/app/api"]
