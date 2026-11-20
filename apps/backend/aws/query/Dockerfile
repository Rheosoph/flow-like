# Builds for linux/amd64 and linux/arm64 on Amazon Linux 2023.
FROM public.ecr.aws/amazonlinux/amazonlinux:2023 AS build
COPY rust-toolchain.toml /rust-toolchain.toml

ARG TARGETARCH
ARG PROTOC_VERSION=25.3

WORKDIR /app

RUN set -eu && \
    case "$TARGETARCH" in \
        amd64) protoc_arch=x86_64 ;; \
        arm64) protoc_arch=aarch_64 ;; \
        *) echo "Unsupported TARGETARCH: ${TARGETARCH:-unset}" >&2; exit 1 ;; \
    esac && \
    yum update -y && \
    yum install -y \
        clang \
        cmake \
        git \
        lld \
        make \
        perl \
        pkgconf-pkg-config \
        openssl-devel \
        libcurl-devel \
        unixODBC-devel \
        tar \
        unzip \
        xz && \
    curl -fsSL --retry 3 -O \
        "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip" && \
    unzip "protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip" -d /usr/local && \
    rm "protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip" && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none && \
    /root/.cargo/bin/rustup toolchain install && \
    yum remove -y xz && \
    yum clean all

ENV PATH="/root/.cargo/bin:${PATH}"
ENV CC=clang
RUN ln -sf /usr/bin/clang /usr/local/bin/cc
ARG CARGO_BUILD_JOBS

COPY . .

# AArch64 links with LLD: ld.bfd stops emitting range-extension veneers once .text passes 128 MB.
RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-git,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-target,target=/app/target,sharing=locked \
    source "$HOME"/.cargo/env && \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    if [ "$TARGETARCH" = "arm64" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-Clink-arg=-fuse-ld=lld"; fi && \
    cargo build --locked --release --manifest-path=apps/backend/aws/query/Cargo.toml 2>&1

RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-target,target=/app/target/,sharing=locked cp target/release/aws-query /app/aws-query

RUN strip --strip-all /app/aws-query

FROM public.ecr.aws/lambda/provided:al2023

COPY LICENSE /usr/share/licenses/flow-like/LICENSE
# ca-certificates: BigQuery token requests trust the system roots.
RUN dnf install -y unixODBC ca-certificates && dnf clean all
COPY --from=build /app/aws-query ${LAMBDA_RUNTIME_DIR}/bootstrap

RUN chmod +x ${LAMBDA_RUNTIME_DIR}/bootstrap

CMD ["bootstrap"]
