# Builds for linux/amd64 and linux/arm64. The ONNX Runtime and protoc archives
# are selected from TARGETARCH; Microsoft's Linux builds are manylinux and stay
# under the Amazon Linux 2023 glibc, so no compatibility shim is needed.
FROM public.ecr.aws/amazonlinux/amazonlinux:2023 AS build
COPY rust-toolchain.toml /rust-toolchain.toml

ARG TARGETARCH
ARG ONNXRUNTIME_VERSION=1.24.2
ARG ONNXRUNTIME_SHA256_AMD64=43725474ba5663642e17684717946693850e2005efbd724ac72da278fead25e6
ARG ONNXRUNTIME_SHA256_ARM64=6715b3d19965a2a6981e78ed4ba24f17a8c30d2d26420dbed10aac7ceca0085e
ARG PROTOC_VERSION=25.3

WORKDIR /app

# A Dockerfile RUN is `/bin/sh -c`, so `set -eu` has to be explicit for an
# unresolved architecture to fail loudly instead of downloading nothing.
RUN set -eu && \
    case "$TARGETARCH" in \
        amd64) onnx_arch=x64; onnx_sha="$ONNXRUNTIME_SHA256_AMD64"; protoc_arch=x86_64 ;; \
        arm64) onnx_arch=aarch64; onnx_sha="$ONNXRUNTIME_SHA256_ARM64"; protoc_arch=aarch_64 ;; \
        *) echo "Unsupported TARGETARCH: ${TARGETARCH:-unset}" >&2; exit 1 ;; \
    esac && \
    yum update -y && \
    yum install -y \
        clang \
        git \
        lld \
        pkgconf-pkg-config \
        openssl-devel \
        glib2-devel \
        gtk3-devel \
        tar \
        unzip \
        xz && \
    curl -fsSL --retry 3 -O \
        "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip" && \
    unzip "protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip" -d /usr/local && \
    rm "protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip" && \
    curl -fsSL --retry 3 \
        "https://github.com/microsoft/onnxruntime/releases/download/v${ONNXRUNTIME_VERSION}/onnxruntime-linux-${onnx_arch}-${ONNXRUNTIME_VERSION}.tgz" \
        -o /tmp/onnxruntime.tgz && \
    echo "${onnx_sha}  /tmp/onnxruntime.tgz" | sha256sum -c - && \
    mkdir -p /opt/onnxruntime && \
    tar -xzf /tmp/onnxruntime.tgz --strip-components=1 -C /opt/onnxruntime && \
    rm /tmp/onnxruntime.tgz && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none && \
    /root/.cargo/bin/rustup toolchain install && \
    yum remove -y xz && \
    yum clean all

ENV PATH="/root/.cargo/bin:${PATH}"
ENV CC=clang
ENV ORT_LIB_PATH=/opt/onnxruntime/lib
ENV ORT_PREFER_DYNAMIC_LINK=1
ENV LD_LIBRARY_PATH=/opt/onnxruntime/lib
RUN ln -sf /usr/bin/clang /usr/local/bin/cc
ARG CARGO_BUILD_JOBS

COPY . .

# An AArch64 `BL` reaches 128 MB. Once .text crosses that, ld.bfd stops emitting
# range-extension veneers and the link dies with R_AARCH64_CALL26 truncation, so
# AArch64 links with LLD, which emits thunks. x86-64 keeps the default linker;
# its call displacement is 2 GB.
RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-git,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-target,target=/app/target,sharing=locked \
    source "$HOME"/.cargo/env && \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    if [ "$TARGETARCH" = "arm64" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-Clink-arg=-fuse-ld=lld"; fi && \
    cargo build --locked --release --manifest-path=apps/backend/aws/executor-ecs/Cargo.toml 2>&1

RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-target,target=/app/target/,sharing=locked \
    cp target/release/aws-executor-ecs /app/aws-executor-ecs

RUN strip --strip-all /app/aws-executor-ecs

FROM public.ecr.aws/amazonlinux/amazonlinux:2023-minimal

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

COPY --from=build /opt/onnxruntime/lib/ /usr/lib64/
COPY --from=build /opt/onnxruntime/LICENSE /usr/share/licenses/onnxruntime/LICENSE
COPY --from=build /opt/onnxruntime/ThirdPartyNotices.txt /usr/share/licenses/onnxruntime/ThirdPartyNotices.txt
COPY --from=build /app/aws-executor-ecs /usr/local/bin/executor-async

ENTRYPOINT ["/usr/local/bin/executor-async"]
