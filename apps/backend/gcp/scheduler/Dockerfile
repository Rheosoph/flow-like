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
    --mount=type=cache,target=/app/target,id=cargo-target-gcp-scheduler \
    cargo build --locked --release --package gcp-scheduler \
    && strip -s /app/target/release/gcp-scheduler \
    && cp /app/target/release/gcp-scheduler /tmp/gcp-scheduler

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/gcp-scheduler /app/scheduler

USER 10001:10001
# A Cloud Run *job*: it runs one tick and exits, so there is no listener, no
# PORT and no EXPOSE. The defaults below equal the binary's own defaults; they
# are set here so the image is runnable as-is and so the values are visible in
# `docker inspect` rather than only in Terraform. SCHEDULER_STATE_BACKEND is
# pinned because this image links only the Firestore store — the binary refuses
# any other value at startup. SINK_TRIGGER_JWT is deliberately absent: it is a
# Secret Manager reference injected by Cloud Run, never an image default.
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    SCHEDULER_STATE_BACKEND=firestore \
    FIRESTORE_SCHEDULER_COLLECTION=scheduler \
    SCHEDULER_MAX_CATCHUP_SECS=600 \
    SCHEDULER_TICK_DEADLINE_SECS=200

STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/scheduler"]
