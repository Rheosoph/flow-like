# syntax=docker/dockerfile:1.4
FROM rust:1.97.1-bookworm AS builder
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=/cargo-home,id=mail-ingress-cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=mail-ingress-target-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package flow-like-mail-ingress && \
    cp /app/target/release/flow-like-mail-ingress /tmp/flow-like-mail-ingress

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && useradd -r -u 1000 flowlike
COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=builder /tmp/flow-like-mail-ingress /app/flow-like-mail-ingress
USER flowlike
ENV RUST_LOG=info
ENTRYPOINT ["/app/flow-like-mail-ingress"]

