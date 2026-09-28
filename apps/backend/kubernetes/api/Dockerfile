# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.97.1-bookworm AS builder
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY . .

# Runtime FLOW_LIKE_CONFIG_* sources override this public fallback. Keep the
# self-hosting example as the default; never bake deployment secrets into images.
ARG FLOW_LIKE_CONFIG=apps/backend/kubernetes/flow-like.config.example.json
RUN cp "$FLOW_LIKE_CONFIG" /app/flow-like.config.json

# Limit build parallelism to reduce memory pressure during lance compilation
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-k8s-api-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package k8s-api && \
    cp /app/target/release/k8s-api /tmp/k8s-api

# ============================================================
# Stage 4: Runtime - minimal image for production
# ============================================================
FROM debian:bookworm-slim

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

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
