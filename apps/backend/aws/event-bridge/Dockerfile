# Build stage
FROM rust:1.91-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

COPY Cargo.toml Cargo.lock ./
COPY packages ./packages
COPY apps/backend/aws/event-bridge ./apps/backend/aws/event-bridge

RUN cargo build --release --package aws-event-bridge

# Runtime stage
FROM public.ecr.aws/lambda/provided:al2023

COPY --from=builder /usr/src/app/target/release/aws-event-bridge ${LAMBDA_RUNTIME_DIR}/bootstrap

CMD ["bootstrap"]
