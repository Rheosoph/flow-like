# syntax=docker/dockerfile:1.4
FROM rust:1.97.1-bookworm AS builder
WORKDIR /workspace
COPY . .
RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/workspace/target,id=cargo-target-execution-manager,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release -p flow-like-execution-manager \
      --bin execution-manager --bin execution-gateway && \
    mkdir /out && cp target/release/execution-manager target/release/execution-gateway /out/

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates coreutils \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /out/ /app/
# The supervisor accesses the Docker socket. Each gateway drops privileges
# after preparing its private socket volume and before starting Tokio.
CMD ["/app/execution-manager", "docker"]
