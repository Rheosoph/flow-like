# ADOT Lambda 0.49.0 (2026-08-25 security release), built from immutable
# AWS source and its pinned upstream submodule. Both archives are SHA256 checked.
# This independently buildable stage includes the official awsxray exporter.
# The local patch adds upstream memory_limiter/decouple factories, adapts AWS's
# factory-call patch to its upstream context, and locks the ADOT module overlay.
# Source: github.com/aws-observability/aws-otel-lambda (CHANGELOG 2026-08-25).
FROM golang:1.26.4-bookworm AS otel-extension
ARG FLOW_LIKE_OTEL_ENABLED=true
COPY apps/backend/aws/api/otel-collector-processors.patch /tmp/processors.patch
RUN --mount=type=cache,id=flow-like-adot-0.49.0-go-mod,target=/go/pkg/mod,sharing=locked \
    --mount=type=cache,id=flow-like-adot-0.49.0-arm64-go-build,target=/root/.cache/go-build,sharing=locked \
    mkdir -p /otel/extensions /otel/licenses && \
    if [ "$FLOW_LIKE_OTEL_ENABLED" = "true" ]; then \
      curl --proto '=https' --tlsv1.2 -fsSL \
        https://codeload.github.com/aws-observability/aws-otel-lambda/tar.gz/f906c6eeb37b71d9ae7604ae64fbf3e769f04526 -o /tmp/adot.tgz && \
      echo '1a9999838d4f0ee5e281258161bf2ec42ea5b9f593705a6b35e1dadaaf3918ce  /tmp/adot.tgz' | sha256sum -c - && \
      curl --proto '=https' --tlsv1.2 -fsSL \
        https://codeload.github.com/open-telemetry/opentelemetry-lambda/tar.gz/4ec82de147d609400dc034611570cee1fa85c868 -o /tmp/upstream.tgz && \
      echo '9e9d201353d761b9a8027483c5372ad0e478a9bc72b2991ca7bdb46252520d21  /tmp/upstream.tgz' | sha256sum -c - && \
      mkdir -p /src/adot /src/upstream && \
      tar -xzf /tmp/adot.tgz --strip-components=1 -C /src/adot && \
      tar -xzf /tmp/upstream.tgz --strip-components=1 -C /src/upstream && \
      cp /src/adot/LICENSE /otel/licenses/LICENSE && cp /src/adot/NOTICE /otel/licenses/NOTICE && \
      cp -r /src/adot/adot/collector/. /src/upstream/collector/ && \
      cd /src/upstream/collector && \
      git apply -p2 /src/adot/collector.patch && \
      git apply -p1 /tmp/processors.patch && \
      cat /src/adot/adot/collector/lambdacomponents/go.sum >> go.sum && \
      CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build -mod=readonly -trimpath \
        -ldflags '-s -w -X main.Version=v0.156.0-adot0.49.0 -X main.GitHash=f906c6eeb37b71d9ae7604ae64fbf3e769f04526 -X github.com/open-telemetry/opentelemetry-collector-contrib/exporter/awsxrayexporter.collectorDistribution=opentelemetry-collector-lambda' \
        -o /otel/extensions/collector .; \
    elif [ "$FLOW_LIKE_OTEL_ENABLED" != "false" ]; then \
      echo 'FLOW_LIKE_OTEL_ENABLED must be true or false' >&2; exit 1; \
    fi

# Test client uses the collector's pinned protobuf/gRPC graph and stays out of Lambda.
FROM otel-extension AS otel-extension-test-client
COPY tools/api-collector-smoke-client.go /tmp/otel-client.go
RUN --mount=type=cache,id=flow-like-adot-0.49.0-go-mod,target=/go/pkg/mod,sharing=locked \
    --mount=type=cache,id=flow-like-adot-0.49.0-arm64-go-build,target=/root/.cache/go-build,sharing=locked \
    cd /src/upstream/collector && \
    CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build -mod=readonly -trimpath -o /otel-client /tmp/otel-client.go

# Production config/lifecycle plus gRPC-to-X-Ray transport, with networking disabled.
FROM python:3.12-slim AS otel-extension-smoke
COPY --from=otel-extension-test-client /otel-client /otel-client
COPY --from=otel-extension /otel/extensions/collector /collector
COPY apps/backend/aws/api/otel-collector.yaml /otel-collector.yaml
COPY tools/check-api-collector.py /check-api-collector.py
RUN --network=none python /check-api-collector.py

# Build with --platform linux/arm64 to match the bundled tools and Lambda runtime.
FROM public.ecr.aws/amazonlinux/amazonlinux:2023 AS build

WORKDIR /app

RUN yum update -y && \
    yum install -y \
        clang \
        git \
        pkgconf-pkg-config \
        openssl-devel \
        glib2-devel \
        gtk3-devel \
        unzip \
        xz && \
    curl -LO https://github.com/protocolbuffers/protobuf/releases/download/v25.3/protoc-25.3-linux-aarch_64.zip && \
    unzip protoc-25.3-linux-aarch_64.zip -d /usr/local && \
    rm protoc-25.3-linux-aarch_64.zip && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.97.1 && \
    yum remove -y xz && \
    yum clean all

ENV PATH="/root/.cargo/bin:${PATH}"
ARG CARGO_BUILD_JOBS

COPY . .

# Copy the binary before releasing the shared cache lock; caches may be evicted.
RUN --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-git,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,id=flow-like-rust-1.97.1-amazonlinux2023-arm64-target,target=/app/target,sharing=locked \
    source "$HOME"/.cargo/env && \
    if [ -n "${CARGO_BUILD_JOBS:-}" ]; then export CARGO_BUILD_JOBS; fi && \
    cargo build --locked --profile lambda --manifest-path=apps/backend/aws/api/Cargo.toml 2>&1 && \
    cp target/lambda/aws-api /app/aws-api && \
    strip --strip-all /app/aws-api

ARG CRDB_CA_URL
RUN if [ -n "$CRDB_CA_URL" ]; then \
      curl -fsSL "$CRDB_CA_URL" -o /app/db-root.crt; \
    else \
      touch /app/db-root.crt; \
    fi

FROM public.ecr.aws/lambda/provided:al2023-arm64

COPY LICENSE /usr/share/licenses/flow-like/LICENSE
COPY --from=otel-extension /otel/licenses/ /usr/share/licenses/aws-otel-lambda/
COPY --from=otel-extension /otel/extensions/ /opt/extensions/
COPY apps/backend/aws/api/otel-collector.yaml /var/task/otel-collector.yaml
COPY --from=build /app/aws-api ${LAMBDA_RUNTIME_DIR}/bootstrap

RUN chmod +x ${LAMBDA_RUNTIME_DIR}/bootstrap
COPY --from=build /app/db-root.crt /var/runtime/certs/root.crt

CMD ["bootstrap"]
