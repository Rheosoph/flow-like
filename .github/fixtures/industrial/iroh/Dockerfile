FROM rust:1.94.0-slim-bookworm@sha256:a86cada82e36ebd7a9bffed7548792c55a952fdb20718eea9278a936bcb76e62 AS build
COPY rust-toolchain.toml /rust-toolchain.toml
RUN rustup toolchain install
WORKDIR /work/.github/fixtures/industrial/iroh
COPY .github/fixtures/industrial/iroh/Cargo.toml .github/fixtures/industrial/iroh/Cargo.lock .github/fixtures/industrial/iroh/lib.rs .github/fixtures/industrial/iroh/reference.rs ./
COPY packages/industrial/src/iroh.rs /work/packages/industrial/src/iroh.rs
COPY packages/industrial/tests/iroh_e2e.rs /work/packages/industrial/tests/iroh_e2e.rs
RUN cargo build --locked --bin iroh-reference \
    && cargo test --locked --test iroh_e2e --no-run \
    && find target/debug/deps -maxdepth 1 -type f -name 'iroh_e2e-*' -executable -exec cp '{}' /iroh-e2e ';'

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
COPY --from=build /work/.github/fixtures/industrial/iroh/target/debug/iroh-reference /usr/local/bin/iroh-reference
COPY --from=build /iroh-e2e /usr/local/bin/iroh-e2e
ENV FLOW_LIKE_IROH_REFERENCE=/usr/local/bin/iroh-reference
USER 65534:65534
ENTRYPOINT ["timeout", "180", "iroh-e2e", "--ignored", "--test-threads=1", "--nocapture"]
