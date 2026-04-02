FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_TARGET_DIR=/tmp/cargo-target
ENV RUSTFLAGS="--cfg tokio_unstable"

# Linux native deps + Windows cross-compilation toolchain (MinGW)
RUN apt-get update -qq && apt-get install -y -qq \
    curl build-essential pkg-config protobuf-compiler \
    libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
    libclang-dev libxcb1-dev libxrandr-dev libdbus-1-dev \
    libpipewire-0.3-dev libwayland-dev libegl-dev \
    libgtk-3-dev libglib2.0-dev libsoup-3.0-dev \
    libjavascriptcoregtk-4.1-dev libssl-dev \
    libxdo-dev libinput-dev libxkbcommon-dev \
    gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable -q

ENV PATH="/root/.cargo/bin:${PATH}"

# Add Windows GNU target for cross-checking
RUN rustup target add x86_64-pc-windows-gnu

WORKDIR /workspace
