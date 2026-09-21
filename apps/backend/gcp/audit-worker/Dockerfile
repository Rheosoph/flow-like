# syntax=docker/dockerfile:1.4
# Build from the flow-like repository root using its reviewed .dockerignore.
FROM rust:1.97.1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-audit-worker-gcp,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package flow-like-audit-worker --features gcp && \
    cp /app/target/release/flow-like-audit-worker /tmp/flow-like-audit-worker

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 python3 && \
    rm -rf /var/lib/apt/lists/* && useradd -r -u 1000 flowlike
COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=builder /tmp/flow-like-audit-worker /app/flow-like-audit-worker
COPY apps/backend/gcp/audit-worker/entrypoint.py /app/entrypoint.py
USER 1000:1000
WORKDIR /app
ENV RUST_LOG=info
STOPSIGNAL SIGTERM
ENTRYPOINT ["python3", "/app/entrypoint.py"]
CMD ["--once"]
