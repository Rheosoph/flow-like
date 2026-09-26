# syntax=docker/dockerfile:1.4
# Context: flow-like repository root. The root .dockerignore excludes secrets.
FROM rust:1.97.1-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
# The worker compiles the audit section of the same document as the Azure API
# image and accepts no runtime override. Pass the API build's flow_like_config
# secret and FLOW_LIKE_CONFIG_SHA256; without them the tracked public default is used.
ARG FLOW_LIKE_CONFIG_SHA256
RUN --mount=type=secret,id=flow_like_config,target=/tmp/flow-like.azure.config.json \
    --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-azure-audit-worker,sharing=locked \
    if [ -e /tmp/flow-like.azure.config.json ]; then \
        test -s /tmp/flow-like.azure.config.json \
        && test -n "$FLOW_LIKE_CONFIG_SHA256" \
        && printf '%s  %s\n' "$FLOW_LIKE_CONFIG_SHA256" /tmp/flow-like.azure.config.json | sha256sum --check --status - \
        && grep -Eq '"provider"[[:space:]]*:[[:space:]]*"azure"' /tmp/flow-like.azure.config.json \
        && cp /tmp/flow-like.azure.config.json /app/flow-like.config.json; \
    else test -z "$FLOW_LIKE_CONFIG_SHA256"; fi \
    && CARGO_HOME=/cargo-home cargo build --locked --release --package flow-like-audit-worker --features azure \
    && cp /app/target/release/flow-like-audit-worker /tmp/flow-like-audit-worker \
    && rm /app/flow-like.config.json
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 python3 && rm -rf /var/lib/apt/lists/* && useradd -r -u 1000 flowlike
COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=builder /tmp/flow-like-audit-worker /app/flow-like-audit-worker
COPY apps/backend/azure/audit-worker/entrypoint.py /app/entrypoint.py
RUN python3 -B -c "import sys; sys.path.insert(0, '/app'); import entrypoint"
USER flowlike
WORKDIR /app
ENV RUST_LOG=info
ENTRYPOINT ["python3", "/app/entrypoint.py"]
CMD ["--once"]
