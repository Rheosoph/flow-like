# Build with --platform linux/arm64 to match the bundled tools and Lambda runtime.
FROM public.ecr.aws/amazonlinux/amazonlinux:2023 AS build

WORKDIR /app

RUN yum update -y && \
    yum install -y \
        clang \
        git \
        pkgconf-pkg-config \
        openssl-devel \
        glib2-devel \
        gtk3-devel \
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
    source "$HOME"/.cargo/env && \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    cargo build --locked --profile lambda --manifest-path=apps/backend/aws/api/Cargo.toml 2>&1 && \
    cp target/lambda/aws-api /app/aws-api && \
    strip --strip-all /app/aws-api

ARG CRDB_CA_URL
RUN if [ -n "$CRDB_CA_URL" ]; then \
      curl -fsSL "$CRDB_CA_URL" -o /app/db-root.crt; \
    else \
      touch /app/db-root.crt; \
    fi

FROM public.ecr.aws/lambda/provided:al2023-arm64

COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=build /app/aws-api ${LAMBDA_RUNTIME_DIR}/bootstrap

RUN chmod +x ${LAMBDA_RUNTIME_DIR}/bootstrap
COPY --from=build /app/db-root.crt /var/runtime/certs/root.crt

CMD ["bootstrap"]
