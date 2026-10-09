FROM rust:1.94.0-slim-bookworm@sha256:a86cada82e36ebd7a9bffed7548792c55a952fdb20718eea9278a936bcb76e62 AS rust-build
WORKDIR /work/.github/fixtures/industrial/modbus-rtu
COPY .github/fixtures/industrial/modbus-rtu/Cargo.toml .github/fixtures/industrial/modbus-rtu/Cargo.lock .github/fixtures/industrial/modbus-rtu/lib.rs ./
COPY packages/industrial/src/modbus.rs /work/packages/industrial/src/modbus.rs
COPY packages/industrial/tests/modbus_rtu_e2e.rs /work/packages/industrial/tests/modbus_rtu_e2e.rs
RUN CARGO_BUILD_JOBS=2 cargo test --locked --test modbus_rtu_e2e --no-run \
    && find target/debug/deps -maxdepth 1 -type f -name 'modbus_rtu_e2e-*' -executable -exec cp '{}' /modbus-rtu-e2e ';'

FROM python:3.13-slim@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c
RUN apt-get update -qq && apt-get install -y --no-install-recommends socat \
    && rm -rf /var/lib/apt/lists/*
COPY .github/fixtures/industrial/modbus-rtu/requirements.txt /fixture/requirements.txt
RUN pip install --no-cache-dir -r /fixture/requirements.txt
COPY --from=rust-build /modbus-rtu-e2e /usr/local/bin/modbus-rtu-e2e
COPY .github/fixtures/industrial/modbus_server.py /fixture/modbus_server.py
COPY .github/fixtures/industrial/modbus-rtu/server.py .github/fixtures/industrial/modbus-rtu/run.sh /fixture/
ENTRYPOINT ["sh", "/fixture/run.sh"]
