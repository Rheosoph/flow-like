# syntax=docker/dockerfile:1.7

# Build stage
FROM rust:1.97.1-bookworm AS builder
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install

WORKDIR /app

# Cargo resolves every member listed by the root virtual workspace before it
# selects this package. Keep the complete (secret-filtered) workspace context
# available, matching the other backend builders; `.dockerignore` excludes
# target directories, credentials, deployment configs, and frontend outputs.
COPY . .

# Build release binary. Cache mounts keep repeated local and BuildKit builds
# warm without copying compiler artifacts into the runtime image.
RUN --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-k8s-sink-trigger-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package k8s-sink-trigger \
    && cp /app/target/release/k8s-sink-trigger /tmp/k8s-sink-trigger

# The GNU Rust target needs glibc and its loader from the matching Debian runtime.
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

COPY --from=builder /tmp/k8s-sink-trigger /sink-trigger

USER nonroot:nonroot

ENTRYPOINT ["/sink-trigger"]
