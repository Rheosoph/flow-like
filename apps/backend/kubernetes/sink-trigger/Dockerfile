# Build stage
FROM rust:1.93-bookworm AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY packages packages
COPY apps/backend/kubernetes/sink-trigger apps/backend/kubernetes/sink-trigger

# Build release binary
RUN cargo build --release --package k8s-sink-trigger

# Runtime stage - distroless for minimal attack surface
FROM gcr.io/distroless/static:nonroot

COPY --from=builder /app/target/release/k8s-sink-trigger /sink-trigger

USER nonroot:nonroot

ENTRYPOINT ["/sink-trigger"]
