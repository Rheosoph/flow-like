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
    --mount=type=cache,target=/app/target,id=cargo-target-azure-executor,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package azure-executor \
    && strip -s /app/target/release/azure-executor \
    && cp /app/target/release/azure-executor /tmp/azure-executor

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/azure-executor /app/executor

USER 10001:10001
EXPOSE 8080 9090

# Server-only image: the binary refuses any other EXECUTOR_SERVER_MODE, so the
# opt-in is baked in and only an explicit override can reach the refusal.
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    PORT=8080 \
    METRICS_PORT=9090 \
    EXECUTOR_SERVER_MODE=true

STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/executor"]
