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
# Supply that input as a BuildKit secret so ignored local configs never have to
# enter the Docker context or persist in an image layer.
RUN --mount=type=secret,id=flow_like_config,required=true,target=/tmp/flow-like.azure.config.json \
    --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-azure-api \
    test -s /tmp/flow-like.azure.config.json \
    && grep -Eq '"provider"[[:space:]]*:[[:space:]]*"azure"' /tmp/flow-like.azure.config.json \
    && cp /tmp/flow-like.azure.config.json /app/flow-like.config.json \
    && cargo build --locked --release --package azure-api \
    && cp /app/target/release/azure-api /tmp/azure-api

FROM debian:bookworm-slim AS runtime

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
