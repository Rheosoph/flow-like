# syntax=docker/dockerfile:1.7

FROM rust:1.97.1-bookworm AS builder
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

ENV CARGO_BUILD_JOBS=2

# Shared images embed the reviewed public default. Deployments replace it at
# startup with FLOW_LIKE_CONFIG_JSON, FLOW_LIKE_CONFIG_FILE, or
# FLOW_LIKE_CONFIG_SECRET_REF. No deployment config is required to build.
# Preserve the optional legacy custom default: its verified hash participates
# in the cache key, and its contents remain visible in the compiled binary.
# Never provide client secrets as a compile-time input.
ARG FLOW_LIKE_CONFIG_SHA256
RUN --mount=type=secret,id=flow_like_config,target=/tmp/flow-like.azure.config.json \
    --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-azure-api-rust-beta-2026-09-27,sharing=locked \
    if [ -e /tmp/flow-like.azure.config.json ]; then \
        test -s /tmp/flow-like.azure.config.json \
        && test -n "$FLOW_LIKE_CONFIG_SHA256" \
        && printf '%s  %s\n' "$FLOW_LIKE_CONFIG_SHA256" /tmp/flow-like.azure.config.json | sha256sum --check --status - \
        && grep -Eq '"provider"[[:space:]]*:[[:space:]]*"azure"' /tmp/flow-like.azure.config.json \
        && cp /tmp/flow-like.azure.config.json /app/flow-like.config.json; \
    else test -z "$FLOW_LIKE_CONFIG_SHA256"; fi \
    && CARGO_HOME=/cargo-home cargo build --locked --release --package azure-api \
    && strip -s /app/target/release/azure-api \
    && cp /app/target/release/azure-api /tmp/azure-api \
    && rm /app/flow-like.config.json

FROM debian:bookworm-slim AS runtime

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/azure-api /app/api

USER 10001:10001
EXPOSE 8080 9090

ENV PORT=8080 \
    METRICS_PORT=9090 \
    RUST_LOG=warn \
    RUST_BACKTRACE=0 \
    STORAGE_PROVIDER=azure \
    RUNTIME_CREDENTIALS_PROVIDER=azure

ENTRYPOINT ["/app/api"]
