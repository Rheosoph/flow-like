# syntax=docker/dockerfile:1.4
FROM --platform=linux/arm64 public.ecr.aws/amazonlinux/amazonlinux:2023 AS builder
COPY rust-toolchain.toml /rust-toolchain.toml
WORKDIR /app
RUN yum install -y gcc gcc-c++ git make openssl-devel pkgconf-pkg-config xz && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none && \
    /root/.cargo/bin/rustup toolchain install && \
    yum clean all
ENV PATH="/root/.cargo/bin:${PATH}"
ARG CARGO_BUILD_JOBS=1
COPY . .
RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-arm64-home,target=/cargo-home,sharing=shared \
    --mount=type=cache,id=flow-like-rust-beta-2026-09-27-amazonlinux2023-arm64-target,target=/app/target,sharing=locked \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    CARGO_HOME=/cargo-home cargo build --locked --release --package aws-mail-ingress && \
    cp /app/target/release/aws-mail-ingress /tmp/bootstrap

FROM --platform=linux/arm64 public.ecr.aws/lambda/provided:al2023-arm64

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
COPY --from=builder /tmp/bootstrap ${LAMBDA_RUNTIME_DIR}/bootstrap
ENV RUST_LOG=info
CMD ["bootstrap"]
