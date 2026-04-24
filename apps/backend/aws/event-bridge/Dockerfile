# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.87-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && rustup toolchain install nightly-2025-12-15 \
    && rustup default nightly-2025-12-15

WORKDIR /app

COPY . .

RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/runtime/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/docker-compose\/runtime/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/kubernetes\/executor/d' Cargo.toml && \
    sed -i '/dexie-tauri-adapter/d' Cargo.toml

# Limit build parallelism to reduce memory pressure during heavy workspace builds.
ENV CARGO_BUILD_JOBS=2
ENV RUSTFLAGS="--cfg tokio_unstable"

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-aws-event-bridge \
    cargo build --release --package aws-event-bridge && \
    cp /app/target/release/aws-event-bridge /tmp/bootstrap

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM public.ecr.aws/lambda/provided:al2023

# The AL2023 custom-runtime entrypoint executes /var/runtime/bootstrap.
COPY --from=builder /tmp/bootstrap /var/runtime/bootstrap

RUN chmod 755 /var/runtime/bootstrap && \
    test -x /var/runtime/bootstrap

ENV RUST_LOG=info

CMD ["bootstrap"]
