# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.97.1-bookworm AS build-deps
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install

# Automation execution links the Linux capture and input libraries.
# ocl-icd-opencl-dev provides libOpenCL.so: rustautogui's `dev` feature implies
# `opencl`, and cl-sys emits an unconditional -lOpenCL on Linux.
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    libclang-dev \
    libwayland-dev \
    libpipewire-0.3-dev \
    libegl-dev \
    libgbm-dev \
    libxcb-randr0-dev \
    libxcb-render0-dev \
    libx11-dev \
    libxi-dev \
    libxtst-dev \
    libinput-dev \
    libudev-dev \
    libxkbcommon-dev \
    ocl-icd-opencl-dev \
    && rm -rf /var/lib/apt/lists/*

# The prebuilt ONNX archive uses glibc/GCC symbols newer than Bookworm provides.
COPY .github/scripts/ort-glibc-compat.c /tmp/ort-glibc-compat.c
RUN cc -c -O2 -fPIC /tmp/ort-glibc-compat.c -o /tmp/ort-glibc-compat.o
ENV RUSTFLAGS="-Clink-arg=/tmp/ort-glibc-compat.o"

FROM build-deps AS builder
WORKDIR /app

COPY . .

# Limit build parallelism to reduce memory pressure during lance compilation
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

# Use BuildKit cache mounts for faster rebuilds
RUN --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-runtime-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package docker-compose-runtime && \
    cp /app/target/release/docker-compose-runtime /tmp/docker-compose-runtime

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim AS runtime-deps

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    libstdc++6 \
    libwayland-client0 \
    libwayland-server0 \
    libpipewire-0.3-0 \
    libegl1 \
    libgbm1 \
    libxcb-randr0 \
    libxcb-render0 \
    libx11-6 \
    libxi6 \
    libxtst6 \
    libinput10 \
    libudev1 \
    libxkbcommon0 \
    ocl-icd-libopencl1 \
    && rm -rf /var/lib/apt/lists/*

FROM runtime-deps AS runtime
COPY LICENSE /usr/share/licenses/flow-like/LICENSE

WORKDIR /app

COPY --from=builder /tmp/docker-compose-runtime /app/runtime

RUN useradd -r -u 1000 flowlike && \
    chown -R flowlike:flowlike /app

USER flowlike

EXPOSE 9000

ENV RUST_LOG=info

CMD ["/app/runtime"]
