# syntax=docker/dockerfile:1.7

FROM rust:1.97.1-bookworm AS builder
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    cmake \
    libssl-dev \
    unixodbc-dev \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

ENV CARGO_BUILD_JOBS=2

RUN --mount=type=cache,target=/cargo-home,id=cargo-home-rust-beta-2026-09-27,sharing=shared \
    --mount=type=cache,target=/app/target,id=cargo-target-azure-executor-rust-beta-2026-09-27,sharing=locked \
    CARGO_HOME=/cargo-home cargo build --locked --release --package azure-executor \
    && strip -s /app/target/release/azure-executor \
    && cp /app/target/release/azure-executor /tmp/azure-executor

FROM debian:bookworm-slim AS runtime

COPY LICENSE /usr/share/licenses/flow-like/LICENSE

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libodbc2 \
    libodbcinst2 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin flowlike

# ODBC drivers for the database nodes; a connection string names one, e.g.
# Driver={ODBC Driver 18 for SQL Server}, Driver={PostgreSQL} or Driver={MariaDB}.
# SQL Server comes from Microsoft's Debian 12 repository (ACCEPT_EULA accepts the Microsoft ODBC
# Driver 18 licence); curl and gnupg are only needed to add that repository.
RUN apt-get update && apt-get install -y --no-install-recommends curl gnupg \
    && curl -fsSL https://packages.microsoft.com/keys/microsoft.asc | gpg --dearmor -o /usr/share/keyrings/microsoft-prod.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/microsoft-prod.gpg] https://packages.microsoft.com/debian/12/prod bookworm main" > /etc/apt/sources.list.d/mssql-release.list \
    && apt-get update \
    && ACCEPT_EULA=Y apt-get install -y --no-install-recommends msodbcsql18 odbc-postgresql odbc-mariadb \
    && printf '\n[PostgreSQL]\nDescription=PostgreSQL Unicode (same name as on the AWS images)\nDriver=%s\n\n[MariaDB]\nDescription=MariaDB Unicode\nDriver=%s\n' \
        "$(dpkg -L odbc-postgresql | grep '/psqlodbcw.so$')" "$(dpkg -L odbc-mariadb | grep '/libmaodbc.so$')" >> /etc/odbcinst.ini \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/* /etc/apt/sources.list.d/mssql-release.list

WORKDIR /app
COPY --from=builder --chown=flowlike:flowlike /tmp/azure-executor /app/executor

USER 10001:10001
EXPOSE 8080 9090

# Server-only image: the binary refuses any other EXECUTOR_SERVER_MODE, so the
# opt-in is baked in and only an explicit override can reach the refusal.
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    PORT=8080 \
    METRICS_PORT=9090 \
    EXECUTOR_SERVER_MODE=true

STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/executor"]
