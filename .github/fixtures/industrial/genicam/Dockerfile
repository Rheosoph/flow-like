FROM rust:1.94.0-slim-bookworm@sha256:a86cada82e36ebd7a9bffed7548792c55a952fdb20718eea9278a936bcb76e62 AS build
WORKDIR /work/.github/fixtures/industrial/genicam
COPY .github/fixtures/industrial/genicam/Cargo.toml .github/fixtures/industrial/genicam/Cargo.lock .github/fixtures/industrial/genicam/lib.rs ./
COPY packages/industrial/src/genicam.rs /work/packages/industrial/src/genicam.rs
COPY packages/industrial/tests/genicam_e2e.rs /work/packages/industrial/tests/genicam_e2e.rs
COPY packages/vendor/telegenic /work/packages/vendor/telegenic
RUN cargo test --locked --test telegenic_control --test telegenic_stream \
    && cargo test --locked --test genicam_e2e --no-run \
    && find target/debug/deps -maxdepth 1 -type f -name 'genicam_e2e-*' -executable -exec cp '{}' /genicam-e2e ';'

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update -qq \
    && apt-get install -y --no-install-recommends aravis-tools=0.8.26-1 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /genicam-e2e /usr/local/bin/genicam-e2e
COPY .github/fixtures/industrial/genicam/run.sh /fixture/run.sh
ENV INDUSTRIAL_GENICAM_ADDRESS=127.0.0.1:3956
ENV INDUSTRIAL_GENICAM_INTERFACE=127.0.0.1
ENTRYPOINT ["sh", "/fixture/run.sh"]
