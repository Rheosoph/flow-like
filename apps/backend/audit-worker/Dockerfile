# syntax=docker/dockerfile:1.4
FROM rust:1.97.1-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
# The worker compiles the audit section of this document and accepts no runtime
# override. Build it with the same FLOW_LIKE_CONFIG as the sibling API image;
# the Compose and Kubernetes examples carry the same audit policy.
ARG FLOW_LIKE_CONFIG=apps/backend/docker-compose/flow-like.config.example.json
RUN cp "$FLOW_LIKE_CONFIG" /app/flow-like.config.json
ARG AUDIT_FEATURES=aws,azure,gcp
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-audit-worker,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package flow-like-audit-worker --features "$AUDIT_FEATURES" && \
    cp /app/target/release/flow-like-audit-worker /tmp/flow-like-audit-worker

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/* && \
    useradd -r -u 1000 flowlike
COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=builder /tmp/flow-like-audit-worker /app/flow-like-audit-worker
USER flowlike
WORKDIR /app
ENV RUST_LOG=info
ENTRYPOINT ["/app/flow-like-audit-worker"]
