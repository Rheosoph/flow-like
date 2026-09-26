# syntax=docker/dockerfile:1.7
# ECS/Fargate API image for linux/amd64 and linux/arm64, each built on a native
# runner. The runtime is distroless: glibc, libssl3, CA roots and a nonroot
# user, with no shell or package manager. The binary answers its own container
# health check (`/app/api healthcheck`).
FROM rust:1.97.1-bookworm AS build

ARG TARGETARCH
# Graviton2 (Neoverse N1) is the oldest arm64 CPU Fargate runs on, and
# x86-64-v2 covers every x86 family ECS supports. Pass a lower target to run on
# older EC2 capacity, e.g. TARGET_CPU_ARM64=generic for Graviton1 (A1).
ARG TARGET_CPU_ARM64=neoverse-n1
ARG TARGET_CPU_AMD64=x86-64-v2
ARG CARGO_BUILD_JOBS

RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl-dev \
        lld \
        pkg-config \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

# LLD links the large API graph faster than ld.bfd and emits AArch64 range
# thunks, which ld.bfd stops doing once .text outgrows a 128 MB branch.
# Copy the binary out before the target cache mount is released.
RUN --mount=type=cache,id=flow-like-rust-1.97.1-bookworm-${TARGETARCH}-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-bookworm-${TARGETARCH}-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-bookworm-${TARGETARCH}-aws-api-ecs-target,target=/app/target,sharing=locked \
    set -eu && \
    case "$TARGETARCH" in \
        arm64) target_cpu="$TARGET_CPU_ARM64" ;; \
        amd64) target_cpu="$TARGET_CPU_AMD64" ;; \
        *) echo "Unsupported TARGETARCH: ${TARGETARCH:-unset}" >&2; exit 1 ;; \
    esac && \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    export RUSTFLAGS="-C target-cpu=${target_cpu} -C link-arg=-fuse-ld=lld" && \
    cargo build --locked --profile server --package aws-api-ecs && \
    install -D -m 0755 target/server/aws-api-ecs /out/api && \
    strip --strip-all /out/api

FROM gcr.io/distroless/cc-debian12:nonroot

COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=build /out/api /app/api

USER 65532:65532
ENV PORT=8080
EXPOSE 8080

# ECS ignores this; its task definition carries the same command.
HEALTHCHECK --interval=15s --timeout=3s --start-period=60s --retries=3 \
    CMD ["/app/api", "healthcheck"]

ENTRYPOINT ["/app/api"]
