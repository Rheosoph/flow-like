FROM rust:1.94.0-slim-bookworm@sha256:a86cada82e36ebd7a9bffed7548792c55a952fdb20718eea9278a936bcb76e62 AS rust-build
WORKDIR /work/.github/fixtures/industrial/ethercat
COPY .github/fixtures/industrial/ethercat/Cargo.toml .github/fixtures/industrial/ethercat/Cargo.lock .github/fixtures/industrial/ethercat/lib.rs ./
COPY packages/industrial/src/ethercat.rs /work/packages/industrial/src/ethercat.rs
COPY packages/industrial/tests/ethercat_e2e.rs /work/packages/industrial/tests/ethercat_e2e.rs
RUN CARGO_BUILD_JOBS=2 cargo test --locked --lib ethercat::runtime::cancellation_tests \
    && CARGO_BUILD_JOBS=2 cargo test --locked --test ethercat_e2e --no-run \
    && find target/debug/deps -maxdepth 1 -type f -name 'ethercat_e2e-*' -executable -exec cp '{}' /ethercat-e2e ';'

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS peer-build
RUN apt-get update -qq && apt-get install -y --no-install-recommends g++ cmake make ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ADD --checksum=sha256:ddc2c35cc5327a22e6bdc0ff85d8444e4e9248319b0a87865c8cb79b75be243f https://codeload.github.com/leducp/KickCAT/tar.gz/cb2e1a192ac11ffaa58f08945053b8129abf94cb /tmp/kickcat.tar.gz
ADD --checksum=sha256:9dcb3d8ce0a41b2a48ac8baa54b51a9f1b6a2c52dd374e28cc713bab0568ec98 https://codeload.github.com/p-ranav/argparse/tar.gz/refs/tags/v3.2 /tmp/argparse.tar.gz
ADD --checksum=sha256:4b92eb0c06d10683f7447ce9406cb97cd4b453be18d7279320f7b2f025c10187 https://codeload.github.com/nlohmann/json/tar.gz/refs/tags/v3.12.0 /tmp/json.tar.gz
ADD --checksum=sha256:5556deb5081fb246ee92afae73efd943c889cef0cafea92b0b82422d6a18f289 https://codeload.github.com/leethomason/tinyxml2/tar.gz/refs/tags/11.0.0 /tmp/tinyxml2.tar.gz
RUN mkdir /opt/kickcat /opt/argparse /opt/json /opt/tinyxml2 \
    && tar -xzf /tmp/kickcat.tar.gz --strip-components=1 -C /opt/kickcat \
    && tar -xzf /tmp/argparse.tar.gz --strip-components=1 -C /opt/argparse \
    && tar -xzf /tmp/json.tar.gz --strip-components=1 -C /opt/json \
    && tar -xzf /tmp/tinyxml2.tar.gz --strip-components=1 -C /opt/tinyxml2 \
    && cmake -S /opt/argparse -B /tmp/argparse -DARGPARSE_BUILD_TESTS=OFF \
    && cmake --install /tmp/argparse \
    && cmake -S /opt/json -B /tmp/json -DJSON_BuildTests=OFF \
    && cmake --install /tmp/json \
    && cmake -S /opt/tinyxml2 -B /tmp/tinyxml2 -Dtinyxml2_BUILD_TESTING=OFF -DBUILD_SHARED_LIBS=OFF \
    && cmake --build /tmp/tinyxml2 -j2 && cmake --install /tmp/tinyxml2
COPY .github/fixtures/industrial/ethercat/CMakeLists.txt .github/fixtures/industrial/ethercat/peer.cc /fixture/
RUN cmake -S /fixture -B /tmp/peer -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /tmp/peer --target ethercat-peer -j2

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update -qq && apt-get install -y --no-install-recommends iproute2 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=peer-build /tmp/peer/ethercat-peer /usr/local/bin/ethercat-peer
COPY --from=peer-build /opt/kickcat /opt/kickcat
COPY --from=peer-build /opt/json/LICENSE.MIT /usr/share/licenses/flow-ethercat/json.MIT
COPY --from=peer-build /opt/tinyxml2/LICENSE.txt /usr/share/licenses/flow-ethercat/tinyxml2.txt
COPY --from=rust-build /ethercat-e2e /usr/local/bin/ethercat-e2e
COPY .github/fixtures/industrial/ethercat/device.json .github/fixtures/industrial/ethercat/run.sh /fixture/
ENTRYPOINT ["sh", "/fixture/run.sh"]
