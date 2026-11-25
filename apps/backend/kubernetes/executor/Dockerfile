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
    cmake \
    libssl-dev \
    unixodbc-dev \
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
    --mount=type=cache,target=/app/target,id=cargo-target-k8s-executor-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package k8s-executor \
      --package flow-like-execution-manager --bin k8s-executor --bin execution-slot && \
    cp /app/target/release/k8s-executor /tmp/k8s-executor && \
    cp /app/target/release/execution-slot /tmp/execution-slot

# ============================================================
# Stage 4: Runtime - minimal for sandboxed execution
# ============================================================
FROM debian:bookworm-slim AS runtime-deps

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libodbc2 \
    libodbcinst2 \
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
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 1000 executor

# ODBC drivers for the database nodes; a connection string names one, e.g.
# Driver={ODBC Driver 18 for SQL Server}, Driver={PostgreSQL} or Driver={MariaDB}.
# SQL Server comes from Microsoft's Debian 12 repository (ACCEPT_EULA accepts the Microsoft ODBC
# Driver 18 licence); curl and gnupg are only needed to add that repository.
RUN apt-get update && apt-get install -y --no-install-recommends curl gnupg \
    && curl -fsSL https://packages.microsoft.com/keys/microsoft.asc | gpg --dearmor -o /usr/share/keyrings/microsoft-prod.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/microsoft-prod.gpg] https://packages.microsoft.com/debian/12/prod bookworm main" > /etc/apt/sources.list.d/mssql-release.list \
    && apt-get update \
    && ACCEPT_EULA=Y apt-get install -y --no-install-recommends msodbcsql18 odbc-postgresql odbc-mariadb \
    && printf '\n[PostgreSQL]\nDescription=PostgreSQL Unicode (same name as on the AWS images)\nDriver=%s\n\n[MariaDB]\nDescription=MariaDB Unicode\nDriver=%s\n' \
        "$(dpkg -L odbc-postgresql | grep '/psqlodbcw.so$')" "$(dpkg -L odbc-mariadb | grep '/libmaodbc.so$')" >> /etc/odbcinst.ini \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/* /etc/apt/sources.list.d/mssql-release.list

FROM runtime-deps AS runtime
COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY packages/browser/NOTICE* packages/browser/LICENSE-* /usr/share/licenses/flow-like/flow-like-browser/

WORKDIR /app

COPY --from=builder /tmp/k8s-executor /app/executor
COPY --from=builder /tmp/execution-slot /app/execution-slot

RUN chown -R executor:executor /app

USER executor

ENV RUST_LOG=info

ENTRYPOINT ["/app/executor"]
