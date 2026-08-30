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
    --mount=type=cache,target=/app/target,id=cargo-target-gcp-queue-worker,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package gcp-queue-worker \
    && strip -s /app/target/release/gcp-queue-worker \
    && cp /app/target/release/gcp-queue-worker /tmp/gcp-queue-worker

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/gcp-queue-worker /app/queue-worker

USER 10001:10001
# Cloud Run overrides PORT; it is set here so the image is runnable as-is and so
# the value is visible in `docker inspect` rather than only in Terraform.
EXPOSE 8080
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    PORT=8080 \
    GCP_QUEUE_MAX_DELIVERY_ATTEMPTS=3 \
    GCP_QUEUE_REQUEST_TIMEOUT_SECS=3600 \
    GCP_QUEUE_CLAIM_HEARTBEAT_SECS=30 \
    GCP_QUEUE_CLAIM_STALE_AFTER_SECS=180 \
    GCP_QUEUE_CLAIM_RETENTION_SECS=86400

STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/queue-worker"]
