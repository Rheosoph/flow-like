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
ENV RUSTFLAGS="--cfg tokio_unstable"

# The API embeds reviewed deployment metadata and OpenID JWKS at compile time.
# Supply that input as a BuildKit secret so it never enters the Docker context.
# The copy placed at /app/flow-like.config.json (over the tracked public
# default) is removed in this same RUN so it never persists in a builder layer
# or a pushed build cache.
RUN --mount=type=secret,id=flow_like_config,required=true,target=/tmp/flow-like.gcp.config.json \
    --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-gcp-api \
    test -s /tmp/flow-like.gcp.config.json \
    && grep -Eq '"provider"[[:space:]]*:[[:space:]]*"gcp"' /tmp/flow-like.gcp.config.json \
    && cp /tmp/flow-like.gcp.config.json /app/flow-like.config.json \
    && cargo build --locked --release --package gcp-api \
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
