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

# The API embeds this public hub/OIDC configuration at compile time.
# A self-hosted build must not inherit the repository's hosted configuration.
ARG FLOW_LIKE_CONFIG=apps/backend/kubernetes/flow-like.config.example.json
RUN cp "$FLOW_LIKE_CONFIG" /app/flow-like.config.json

# Remove packages that require edition 2024 or other backends
RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/runtime/d' Cargo.toml

# Limit build parallelism to reduce memory pressure during lance compilation
ENV CARGO_BUILD_JOBS=2

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-k8s-api,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package k8s-api && \
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
