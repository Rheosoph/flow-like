FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_TARGET_DIR=/tmp/cargo-target
ENV RUSTFLAGS="--cfg tokio_unstable"

# Linux native deps + Windows cross-compilation toolchain (MinGW)
RUN apt-get update -qq && apt-get install -y -qq \
    curl build-essential pkg-config protobuf-compiler \
    libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
    libclang-dev libxcb1-dev libxrandr-dev libdbus-1-dev \
    libpipewire-0.3-dev libwayland-dev libegl-dev libgbm-dev ocl-icd-libopencl1 ocl-icd-opencl-dev \
    libgtk-3-dev libglib2.0-dev libsoup-3.0-dev \
    libjavascriptcoregtk-4.1-dev libssl-dev \
    libxdo-dev libinput-dev libxkbcommon-dev \
    gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/*

RUN lib_dir="/usr/lib/$(dpkg-architecture -qDEB_HOST_MULTIARCH)" \
    && if [ ! -e "$lib_dir/libOpenCL.so" ] && [ -e "$lib_dir/libOpenCL.so.1" ]; then ln -sf "$lib_dir/libOpenCL.so.1" "$lib_dir/libOpenCL.so"; fi \
    && test -e "$lib_dir/libOpenCL.so"

RUN curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain 1.93.0 -q

ENV PATH="/root/.cargo/bin:${PATH}"

# Add Windows GNU target for cross-checking
RUN rustup target add x86_64-pc-windows-gnu

WORKDIR /workspace
