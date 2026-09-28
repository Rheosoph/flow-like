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
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY . .

ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-sink-services-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package docker-compose-sink-services && \
    cp /app/target/release/docker-compose-sink-services /tmp/docker-compose-sink-services

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

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
