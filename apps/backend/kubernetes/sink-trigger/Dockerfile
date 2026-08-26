# syntax=docker/dockerfile:1.7

# Build stage
FROM rust:1.97.1-bookworm AS builder

WORKDIR /app

# Cargo resolves every member listed by the root virtual workspace before it
# selects this package. Keep the complete (secret-filtered) workspace context
# available, matching the other backend builders; `.dockerignore` excludes
# target directories, credentials, deployment configs, and frontend outputs.
COPY . .

# Build release binary. Cache mounts keep repeated local and BuildKit builds
# warm without copying compiler artifacts into the runtime image.
RUN --mount=type=cache,target=/cargo-home,id=cargo-home,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-k8s-sink-trigger,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package k8s-sink-trigger \
    && cp /app/target/release/k8s-sink-trigger /tmp/k8s-sink-trigger

# Runtime stage - distroless for minimal attack surface
FROM gcr.io/distroless/static:nonroot

COPY --from=builder /tmp/k8s-sink-trigger /sink-trigger

USER nonroot:nonroot

ENTRYPOINT ["/sink-trigger"]
