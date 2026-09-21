# syntax=docker/dockerfile:1.4
# ============================================================
# Stage 1: Builder
# ============================================================
# Links the API crate like the Lambda API, so it needs the same build packages.
FROM --platform=linux/arm64 public.ecr.aws/amazonlinux/amazonlinux:2023 AS builder

WORKDIR /app

RUN yum update -y && \
    yum install -y \
        binutils \
        clang \
        git \
        glib2-devel \
        gtk3-devel \
        openssl-devel \
        pkgconf-pkg-config \
        unzip \
        xz && \
    curl -LO https://github.com/protocolbuffers/protobuf/releases/download/v25.3/protoc-25.3-linux-aarch_64.zip && \
    unzip protoc-25.3-linux-aarch_64.zip -d /usr/local && \
    rm protoc-25.3-linux-aarch_64.zip && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.97.1 && \
    yum remove -y xz && \
    yum clean all

ENV PATH="/root/.cargo/bin:${PATH}"
ARG CARGO_BUILD_JOBS

COPY . .

# Copy the binary before releasing the shared cache lock; caches may be evicted.
RUN --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-git,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-target,target=/app/target,sharing=locked \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    cargo build --locked --profile lambda --manifest-path apps/backend/aws/audit-worker/Cargo.toml && \
    cp target/lambda/aws-audit-worker /tmp/bootstrap && \
    strip --strip-all /tmp/bootstrap

ARG CRDB_CA_URL
RUN if [ -n "$CRDB_CA_URL" ]; then \
      curl -fsSL "$CRDB_CA_URL" -o /tmp/db-root.crt; \
    else \
      touch /tmp/db-root.crt; \
    fi

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM --platform=linux/arm64 public.ecr.aws/lambda/provided:al2023-arm64

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

COPY --from=builder /tmp/bootstrap ${LAMBDA_RUNTIME_DIR}/bootstrap
COPY --from=builder /tmp/db-root.crt /var/runtime/certs/root.crt

RUN chmod 755 ${LAMBDA_RUNTIME_DIR}/bootstrap && \
    test -x ${LAMBDA_RUNTIME_DIR}/bootstrap

ENV RUST_LOG=info

CMD ["bootstrap"]
