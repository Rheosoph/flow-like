# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.97.1-bookworm AS build-deps
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install

RUN apt-get update && apt-get install -y \
    pkg-config \
    cmake \
    libssl-dev \
    unixodbc-dev \
    protobuf-compiler \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# The prebuilt ONNX archive uses glibc/GCC symbols newer than Bookworm provides.
COPY .github/scripts/ort-glibc-compat.c /tmp/ort-glibc-compat.c
RUN cc -c -O2 -fPIC /tmp/ort-glibc-compat.c -o /tmp/ort-glibc-compat.o
ENV RUSTFLAGS="-Clink-arg=/tmp/ort-glibc-compat.o"

FROM build-deps AS builder
WORKDIR /app

COPY . .

ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

RUN --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-query-service-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --profile server --package query-service && \
    cp /app/target/server/query-service /tmp/query-service && \
    strip --strip-all /tmp/query-service

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim AS runtime-deps

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libodbc2 \
    libodbcinst2 \
    curl \
    libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

FROM runtime-deps AS runtime
COPY LICENSE /usr/share/licenses/flow-like/LICENSE

WORKDIR /app

COPY --from=builder /tmp/query-service /app/query-service

RUN useradd -r -u 1000 query && \
    mkdir -p /var/lib/flow-like/query-spill /var/lib/flow-like/query-models && \
    chown -R query:query /app /var/lib/flow-like/query-spill /var/lib/flow-like/query-models

USER query

EXPOSE 8090

ENV RUST_LOG=info \
    PORT=8090 \
    QUERY_SPILL_DIR=/var/lib/flow-like/query-spill \
    QUERY_MODEL_CACHE_DIR=/var/lib/flow-like/query-models

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${PORT}/health" || exit 1

CMD ["/app/query-service"]
