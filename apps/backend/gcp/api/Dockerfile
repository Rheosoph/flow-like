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

# The API embeds reviewed deployment and OAuth-provider metadata at compile time.
# Supply that input as a BuildKit secret so it never enters the Docker context.
# BuildKit deliberately excludes secret contents from cache keys, so callers
# also pass the non-secret SHA-256 digest. Referencing and verifying it here
# prevents a changed config from reusing a binary compiled with stale metadata.
# The copy placed at /app/flow-like.config.json (over the tracked public
# default) is removed in this same RUN so no standalone config file persists in
# a layer. Its reviewed non-secret contents are intentionally embedded in the
# compiled API binary; never put client secrets in this input.
ARG FLOW_LIKE_CONFIG_SHA256
RUN --mount=type=secret,id=flow_like_config,required=true,target=/tmp/flow-like.gcp.config.json \
    --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-gcp-api,sharing=locked \
    test -s /tmp/flow-like.gcp.config.json \
    && test -n "$FLOW_LIKE_CONFIG_SHA256" \
    && printf '%s  %s\n' "$FLOW_LIKE_CONFIG_SHA256" /tmp/flow-like.gcp.config.json | sha256sum --check --status - \
    && grep -Eq '"provider"[[:space:]]*:[[:space:]]*"gcp"' /tmp/flow-like.gcp.config.json \
    && cp /tmp/flow-like.gcp.config.json /app/flow-like.config.json \
    && CARGO_HOME=/cargo-home cargo build --locked --release --package gcp-api \
    && strip -s /app/target/release/gcp-api \
    && cp /app/target/release/gcp-api /tmp/gcp-api \
    && rm /app/flow-like.config.json

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/gcp-api /app/api

USER 10001:10001
# Cloud Run publishes exactly one container port, so Prometheus is served from
# the same listener as the API rather than from a second METRICS_PORT socket.
EXPOSE 8080

ENV PORT=8080 \
    RUST_LOG=warn \
    RUST_BACKTRACE=0 \
    STORAGE_PROVIDER=gcp \
    RUNTIME_CREDENTIALS_PROVIDER=gcp

ENTRYPOINT ["/app/api"]
