# syntax=docker/dockerfile:1.4
FROM --platform=linux/arm64 public.ecr.aws/amazonlinux/amazonlinux:2023 AS builder
COPY rust-toolchain.toml /rust-toolchain.toml
WORKDIR /app
RUN yum install -y gcc gcc-c++ git make openssl-devel pkgconf-pkg-config xz && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none && \
    /root/.cargo/bin/rustup toolchain install && \
    yum clean all
ENV PATH="/root/.cargo/bin:${PATH}"
COPY . .
RUN --mount=type=cache,id=flow-like-mail-ingress-cargo-home-rust-beta-2026-09-27,target=/cargo-home,sharing=shared \
    --mount=type=cache,id=flow-like-mail-ingress-lambda-target-rust-beta-2026-09-27,target=/app/target,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package aws-mail-ingress && \
    cp /app/target/release/aws-mail-ingress /tmp/bootstrap

FROM --platform=linux/arm64 public.ecr.aws/lambda/provided:al2023-arm64
COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=builder /tmp/bootstrap ${LAMBDA_RUNTIME_DIR}/bootstrap
ENV RUST_LOG=info
CMD ["bootstrap"]
