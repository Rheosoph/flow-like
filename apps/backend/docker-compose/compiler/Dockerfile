# syntax=docker/dockerfile:1.4
FROM rust:1.87-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && rustup toolchain install nightly-2025-12-15 \
    && rustup default nightly-2025-12-15

WORKDIR /app
COPY . .

RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/executor/d' Cargo.toml

ENV CARGO_BUILD_JOBS=2
ENV RUSTFLAGS="--cfg tokio_unstable"

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-compiler \
    cargo build --release --package k8s-compiler && \
    cp /app/target/release/k8s-compiler /tmp/compiler

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates libssl3 curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /tmp/compiler /app/compiler

RUN useradd -r -u 1000 flowlike && chown -R flowlike:flowlike /app
USER flowlike
EXPOSE 8081 9091
ENV RUST_LOG=info
CMD ["/app/compiler"]
