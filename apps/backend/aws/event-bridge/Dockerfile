# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
FROM --platform=linux/arm64 public.ecr.aws/amazonlinux/amazonlinux:2023 AS builder

WORKDIR /app

RUN yum update -y && \
    yum install -y \
        binutils \
        clang \
        gcc \
        gcc-c++ \
        make \
        openssl-devel \
        pkgconf-pkg-config \
        unzip \
        xz && \
    curl -LO https://github.com/protocolbuffers/protobuf/releases/download/v25.3/protoc-25.3-linux-aarch_64.zip && \
    unzip protoc-25.3-linux-aarch_64.zip -d /usr/local && \
    rm protoc-25.3-linux-aarch_64.zip && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.93.0 && \
    yum remove -y xz && \
    yum clean all

ENV PATH="/root/.cargo/bin:${PATH}"
ENV RUSTFLAGS="--cfg tokio_unstable"
ARG CARGO_BUILD_JOBS

COPY . .

RUN --mount=type=cache,id=flow-like-rust-1.93-amazonlinux2023-arm64-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.93-amazonlinux2023-arm64-git,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.93-amazonlinux2023-arm64-target,target=/app/target,sharing=locked \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    cargo build --locked --release --manifest-path apps/backend/aws/event-bridge/Cargo.toml && \
    cp /app/target/release/aws-event-bridge /tmp/bootstrap && \
    strip --strip-all /tmp/bootstrap

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM --platform=linux/arm64 public.ecr.aws/lambda/provided:al2023-arm64

COPY --from=builder /tmp/bootstrap ${LAMBDA_RUNTIME_DIR}/bootstrap

RUN chmod 755 ${LAMBDA_RUNTIME_DIR}/bootstrap && \
    test -x ${LAMBDA_RUNTIME_DIR}/bootstrap

ENV RUST_LOG=info

CMD ["bootstrap"]
