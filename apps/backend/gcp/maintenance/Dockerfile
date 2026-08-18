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

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-gcp-maintenance \
    cargo build --locked --release --package gcp-maintenance \
    && strip -s /app/target/release/gcp-maintenance \
    && cp /app/target/release/gcp-maintenance /tmp/gcp-maintenance

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/gcp-maintenance /app/maintenance

USER 10001:10001
# A Cloud Run *job*: no port, no listener. It runs once per execution, calls the
# API and exits; the exit code is the result. MAINTENANCE_JOB is set here so the
# deployed default is visible in `docker inspect` rather than only in the code.
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    MAINTENANCE_JOB=all

STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/maintenance"]
