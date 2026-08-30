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
    --mount=type=cache,target=/app/target,id=cargo-target-gcp-executor,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package gcp-executor \
    && strip -s /app/target/release/gcp-executor \
    && cp /app/target/release/gcp-executor /tmp/gcp-executor

FROM debian:bookworm-slim AS runtime

# ca-certificates because every byte this process moves — presigned object
# URLs, the callback to the API, remote model endpoints — travels over TLS to a
# host the request names, not one baked into the image.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/gcp-executor /app/executor

USER 10001:10001
# Cloud Run overrides PORT; it is set here so the image is runnable as-is and so
# the value is visible in `docker inspect` rather than only in Terraform. It is
# the only port: Cloud Run publishes one container port per service, so
# /metrics is served from this listener and METRICS_PORT is refused at boot.
EXPOSE 8080
# EXECUTOR_SERVER_MODE is baked in because the image has no other mode; a
# deployment that sets it to anything but true is refused at boot rather than
# silently served. EXECUTOR_TIMEOUT_SECS sits below Cloud Run's 3600-second
# request ceiling so the executor's own timeout — which reports the failure
# through the callback — fires before the platform cancels the handler.
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    PORT=8080 \
    EXECUTOR_SERVER_MODE=true \
    EXECUTOR_TIMEOUT_SECS=3540

STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/executor"]
