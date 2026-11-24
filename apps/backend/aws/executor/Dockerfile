# Builds for linux/amd64 and linux/arm64. The ONNX Runtime and protoc archives
# are selected from TARGETARCH; Microsoft's Linux builds are manylinux and stay
# under the Amazon Linux 2023 glibc, so no compatibility shim is needed.
FROM public.ecr.aws/amazonlinux/amazonlinux:2023 AS build
COPY rust-toolchain.toml /rust-toolchain.toml

ARG TARGETARCH
ARG ONNXRUNTIME_VERSION=1.24.2
ARG ONNXRUNTIME_NUPKG_SHA256=782564d3d68e87269ea1812cc94710ad06fe014faf093f01a6f89d0bd58719d3
ARG PROTOC_VERSION=25.3

WORKDIR /app

# A Dockerfile RUN is `/bin/sh -c`, so `set -eu` has to be explicit for an
# unresolved architecture to fail loudly instead of downloading nothing.
# Vendored OpenSSL needs make and full Perl core modules; librdkafka uses CMake.
# ONNX Runtime comes from Microsoft's signed NuGet package: the GitHub release
# assets 404 since 2026-11-17. The package ships a bare libonnxruntime.so, so the
# SONAME chain the linker and loader expect is recreated here.
RUN set -eu && \
    case "$TARGETARCH" in \
        amd64) onnx_arch=x64; protoc_arch=x86_64 ;; \
        arm64) onnx_arch=arm64; protoc_arch=aarch_64 ;; \
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
        "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/${ONNXRUNTIME_VERSION}/microsoft.ml.onnxruntime.${ONNXRUNTIME_VERSION}.nupkg" \
        -o /tmp/onnxruntime.nupkg && \
    echo "${ONNXRUNTIME_NUPKG_SHA256}  /tmp/onnxruntime.nupkg" | sha256sum -c - && \
    mkdir -p /opt/onnxruntime/lib && \
    unzip -q /tmp/onnxruntime.nupkg LICENSE ThirdPartyNotices.txt -d /opt/onnxruntime && \
    unzip -qj /tmp/onnxruntime.nupkg "runtimes/linux-${onnx_arch}/native/*" -d /opt/onnxruntime/lib && \
    mv /opt/onnxruntime/lib/libonnxruntime.so "/opt/onnxruntime/lib/libonnxruntime.so.${ONNXRUNTIME_VERSION}" && \
    ln -s "libonnxruntime.so.${ONNXRUNTIME_VERSION}" /opt/onnxruntime/lib/libonnxruntime.so.1 && \
    ln -s libonnxruntime.so.1 /opt/onnxruntime/lib/libonnxruntime.so && \
    rm /tmp/onnxruntime.nupkg && \
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
    cargo build --locked --release --manifest-path=apps/backend/aws/executor/Cargo.toml 2>&1

RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-${TARGETARCH}-target,target=/app/target/,sharing=locked cp target/release/aws-executor /app/aws-executor

RUN strip --strip-all /app/aws-executor

FROM public.ecr.aws/lambda/provided:al2023

# Patch runtime packages separately from the builder (ALAS2023-2026-3430).
ARG AL2023_SECURITY_RELEASE=2023.12.20261112
RUN set -eu; \
    dnf upgrade -y --releasever="${AL2023_SECURITY_RELEASE}" rpm rpm-libs; \
    for package in rpm rpm-libs; do \
        installed_version="$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$package")"; \
        test "$(rpm --eval "%{lua:print(rpm.vercmp('${installed_version}', '4.16.1.3-29.amzn2023.0.10'))}")" -ge 0; \
    done; \
    dnf clean all

COPY LICENSE /usr/share/licenses/flow-like/LICENSE
RUN dnf install -y unixODBC && dnf clean all
COPY --from=build /opt/onnxruntime/lib/ /usr/lib64/
COPY --from=build /opt/onnxruntime/LICENSE /usr/share/licenses/onnxruntime/LICENSE
COPY --from=build /opt/onnxruntime/ThirdPartyNotices.txt /usr/share/licenses/onnxruntime/ThirdPartyNotices.txt
COPY --from=build /app/aws-executor ${LAMBDA_RUNTIME_DIR}/bootstrap

RUN chmod +x ${LAMBDA_RUNTIME_DIR}/bootstrap

CMD ["bootstrap"]
