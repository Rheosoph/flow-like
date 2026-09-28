FROM rust:1.97.1-bookworm AS build
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install
WORKDIR /app

ARG CARGO_BUILD_JOBS

RUN apt-get update && apt-get install -y \
        clang \
        cmake \
        pkg-config \
        libssl-dev \
        libclang-dev \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-bookworm-arm64-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-beta-2026-09-27-bookworm-arm64-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-beta-2026-09-27-bookworm-arm64-target,target=/app/target,sharing=locked \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    cargo build --locked --release --manifest-path=apps/backend/aws/compiler-ecs/Cargo.toml

RUN --mount=type=cache,id=flow-like-rust-beta-2026-09-27-bookworm-arm64-target,target=/app/target/,sharing=locked \
    cp target/release/aws-compiler-ecs /app/aws-compiler-ecs

RUN strip --strip-all /app/aws-compiler-ecs

# distroless/cc includes libc, libgcc, libstdc++, and ca-certificates
FROM gcr.io/distroless/cc-debian12:latest

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

COPY --from=build /app/aws-compiler-ecs /usr/local/bin/compiler

ENTRYPOINT ["/usr/local/bin/compiler"]
