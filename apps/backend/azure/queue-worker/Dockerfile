# syntax=docker/dockerfile:1.7

FROM rust:1.97.1-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

ENV CARGO_BUILD_JOBS=2

RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-azure-queue-worker,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package azure-queue-worker \
    && strip -s /app/target/release/azure-queue-worker \
    && cp /app/target/release/azure-queue-worker /tmp/azure-queue-worker

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/azure-queue-worker /app/queue-worker

USER 10001:10001
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    AZURE_QUEUE_BATCH_SIZE=1 \
    AZURE_QUEUE_VISIBILITY_TIMEOUT_SECS=300 \
    AZURE_QUEUE_RENEWAL_INTERVAL_SECS=60 \
    AZURE_QUEUE_MAX_DEQUEUE_COUNT=3

STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/queue-worker"]
