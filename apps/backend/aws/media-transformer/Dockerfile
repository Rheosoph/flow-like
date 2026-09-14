FROM public.ecr.aws/amazonlinux/amazonlinux:2023 AS build

WORKDIR /app

ARG CARGO_BUILD_JOBS

RUN yum update -y && \
    yum install -y \
        binutils \
        clang \
        cmake \
        gcc \
        gcc-c++ \
        git \
        make \
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

COPY . .

# Copy the binary before releasing the shared cache lock; caches may be evicted.
RUN --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-git,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-target,target=/app/target,sharing=locked \
    source "$HOME"/.cargo/env && \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    cargo build --locked --release --manifest-path=apps/backend/aws/media-transformer/Cargo.toml 2>&1 && \
    cp target/release/media-transformer /app/media-transformer && \
    strip --strip-all /app/media-transformer

FROM public.ecr.aws/lambda/provided:al2023-arm64

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

COPY --from=build /app/media-transformer ${LAMBDA_RUNTIME_DIR}/bootstrap

RUN chmod +x ${LAMBDA_RUNTIME_DIR}/bootstrap && \
    test -x ${LAMBDA_RUNTIME_DIR}/bootstrap

ENV RUST_LOG=info

CMD ["bootstrap"]
