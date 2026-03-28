# Build stage
FROM rust:1.91-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

COPY . .
RUN sed -i '/apps\/desktop\/src-tauri/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/api/d' Cargo.toml && \
    sed -i '/apps\/backend\/local\/runtime/d' Cargo.toml && \
    sed -i '/dexie-tauri-adapter/d' Cargo.toml

RUN cargo build --release --package aws-event-bridge

# Runtime stage
FROM public.ecr.aws/lambda/provided:al2023

COPY --from=builder /usr/src/app/target/release/aws-event-bridge ${LAMBDA_RUNTIME_DIR}/bootstrap

CMD ["bootstrap"]
