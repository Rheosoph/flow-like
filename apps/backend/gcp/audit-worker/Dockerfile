# syntax=docker/dockerfile:1.4
# Build from the flow-like repository root using its reviewed .dockerignore.
FROM rust:1.97.1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
# The worker compiles the audit section of the same document as the GCP API
# image and accepts no runtime override. Pass the API build's flow_like_config
# secret and FLOW_LIKE_CONFIG_SHA256; without them the tracked public default is used.
ARG FLOW_LIKE_CONFIG_SHA256
RUN --mount=type=secret,id=flow_like_config,target=/tmp/flow-like.gcp.config.json \
    --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-audit-worker-gcp,sharing=locked \
    if [ -e /tmp/flow-like.gcp.config.json ]; then \
        test -s /tmp/flow-like.gcp.config.json \
        && test -n "$FLOW_LIKE_CONFIG_SHA256" \
        && printf '%s  %s\n' "$FLOW_LIKE_CONFIG_SHA256" /tmp/flow-like.gcp.config.json | sha256sum --check --status - \
        && grep -Eq '"provider"[[:space:]]*:[[:space:]]*"gcp"' /tmp/flow-like.gcp.config.json \
        && cp /tmp/flow-like.gcp.config.json /app/flow-like.config.json; \
    else test -z "$FLOW_LIKE_CONFIG_SHA256"; fi \
    && CARGO_HOME=/cargo-home cargo build --locked --release --package flow-like-audit-worker --features gcp \
    && cp /app/target/release/flow-like-audit-worker /tmp/flow-like-audit-worker \
    && rm /app/flow-like.config.json

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
