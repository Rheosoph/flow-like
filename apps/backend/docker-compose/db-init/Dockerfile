FROM rust:1.97.1-bookworm AS seaorm_builder
RUN cargo install sea-orm-cli@1.1.16 --locked

FROM oven/bun:latest

USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
    postgresql-client ca-certificates curl git libssl3 \
 && rm -rf /var/lib/apt/lists/*

RUN bun add -g prisma@latest

COPY --from=seaorm_builder /usr/local/cargo/bin/sea-orm-cli /usr/local/bin/sea-orm-cli

WORKDIR /workspace
COPY scripts/wait-for-postgres.sh /usr/local/bin/wait-for-postgres.sh
COPY scripts/make-postgres-prisma-mirror.sh /usr/local/bin/make-postgres-prisma-mirror.sh
COPY scripts/export-database-url.sh /usr/local/bin/export-database-url.sh
RUN chmod +x /usr/local/bin/wait-for-postgres.sh /usr/local/bin/make-postgres-prisma-mirror.sh /usr/local/bin/export-database-url.sh

COPY . /workspace
