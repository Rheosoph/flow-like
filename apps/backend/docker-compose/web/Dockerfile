# syntax=docker/dockerfile:1.4
# Official Node LTS binary, with native amd64 and arm64 variants.
FROM node:24.20.0-bookworm-slim@sha256:ba849c60be29959425b8734d57b8b4b7d56f98edd9504c9af091d5281095a71e AS node

# ============================================================
# Stage 1: Device cryptography WebAssembly bundle
# ============================================================
# The browser loads every device-management primitive from /device-crypto.
# The output is generated (and gitignored), so the image builds it the same way
# the published web build does, with the toolchain pinned in rust-toolchain.toml.
FROM rust:1.97.1-bookworm AS device-crypto
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install

# ring compiles C for wasm32, which needs clang with a WebAssembly backend.
RUN apt-get update && apt-get install -y --no-install-recommends clang llvm && \
    rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/bin/node /usr/local/bin/node

WORKDIR /app
COPY . .
RUN rm -rf apps/web/public/device-crypto apps/desktop/public/device-crypto
RUN --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-device-crypto-web-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home node packages/device-crypto/build-web.mjs && \
    test -s apps/web/public/device-crypto/flow_like_device_crypto_bg.wasm && \
    test -s apps/web/public/device-crypto/flow_like_device_crypto.js

# ============================================================
# Stage 2: Builder
# ============================================================
FROM oven/bun:1.3.8-debian AS builder

# Bun's image includes a `node` fallback symlink to Bun. Next's build workers
# require actual Node, including subprocesses that resolve it through PATH.
COPY --from=node /usr/local/bin/node /opt/node/bin/node
ENV PATH="/opt/node/bin:${PATH}"
RUN apt-get update && apt-get install -y --no-install-recommends libatomic1 libstdc++6 && \
    apt-get clean && rm -rf /var/lib/apt/lists/* && \
    node -e 'if (process.versions.bun || process.versions.node.split(".")[0] !== "24") throw new Error("Next builds require Node.js 24")'

WORKDIR /app

# Use the reviewed root context so the frozen workspace graph includes locales,
# widget SDK, root build scripts, and patched dependencies.
COPY . .
RUN rm -rf apps/web/public/device-crypto
RUN bun install --frozen-lockfile

# Deployment URLs are public runtime settings, never image build arguments.
ENV NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG=1
ENV NEXT_TELEMETRY_DISABLED=1

# Build the static export
WORKDIR /app/apps/web
RUN bun ../../scripts/sync-node-icons.ts
COPY --from=device-crypto /app/apps/web/public/device-crypto /app/apps/web/public/device-crypto
ARG TARGETARCH
# TypeScript needs more than 4 GiB for the shared UI workspace. Keep the
# limit configurable and reserve additional builder memory for native tooling.
ARG NODE_MAX_OLD_SPACE_SIZE=8192
ENV NODE_OPTIONS="--max-old-space-size=${NODE_MAX_OLD_SPACE_SIZE}"
# Local/persistent builders retain compilation work across retries. The exported
# /app/apps/web/out directory is outside the mount and remains in the image layer.
RUN --mount=type=cache,id=flow-like-next-node24-next16-${TARGETARCH},target=/app/apps/web/.next,sharing=locked \
    /opt/node/bin/node /app/node_modules/next/dist/bin/next build --turbopack && \
    test -s out/device-crypto/flow_like_device_crypto_bg.wasm

# ============================================================
# Stage 3: Runtime (nginx to serve static files)
# ============================================================
FROM nginx:1.30.4-alpine@sha256:dc5069ad14f19660b141b21236140b91656bf89bbc3e2417c70ae650cd66104c

# Copy custom nginx configuration
COPY apps/backend/kubernetes/web/nginx-main.conf /etc/nginx/nginx.conf
COPY apps/backend/docker-compose/web/nginx.conf /etc/nginx/conf.d/default.conf
COPY apps/backend/docker-compose/web/runtime-config.py /usr/local/bin/flow-like-web-runtime-config.py
COPY apps/backend/docker-compose/web/entrypoint.sh /usr/local/bin/flow-like-web-entrypoint.sh
COPY LICENSE /usr/share/licenses/flow-like/LICENSE

# Copy static files from builder
COPY --from=builder /app/apps/web/out /usr/share/nginx/html

# Python's JSON encoder and URL parser avoid unsafe shell interpolation.
# Static assets stay root-owned; only /tmp needs to be writable at runtime.
RUN apk add --no-cache python3 && adduser -D -u 1000 flowlike

USER flowlike

EXPOSE 3001

ENTRYPOINT ["/bin/sh", "/usr/local/bin/flow-like-web-entrypoint.sh"]
CMD ["nginx", "-g", "daemon off;"]
