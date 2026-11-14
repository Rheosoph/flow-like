#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
platform="${1:-linux/amd64,linux/arm64}"
context="$(mktemp -d)"
trap 'rm -rf "$context"' EXIT

# Exercise the real desktop stage before compiling the agent. BuildKit skips
# the agent and runtime stages, so this context needs no release artifacts.
cp "$repo_root/apps/standalone/packaging/Dockerfile.playground" "$context/Dockerfile"
cp "$repo_root/.github/scripts/tests/container-native-deps.c" "$context/native-deps-check.c"
cat >> "$context/Dockerfile" <<'DOCKERFILE'

FROM ubuntu:24.04 AS native-deps-check-build
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential pkg-config libwayland-dev libpipewire-0.3-dev libegl-dev libgbm-dev \
    libxcb-randr0-dev libxcb-render0-dev libx11-dev libxi-dev libxtst-dev libinput-dev \
    libudev-dev libxkbcommon-dev ocl-icd-opencl-dev \
 && rm -rf /var/lib/apt/lists/*
COPY native-deps-check.c /tmp/native-deps-check.c
RUN libraries="wayland-client wayland-server libpipewire-0.3 egl gbm xcb-randr xcb-render x11 xi xtst libinput libudev xkbcommon OpenCL" \
 && cc -O2 /tmp/native-deps-check.c -o /tmp/native-deps-check \
    $(pkg-config --cflags --libs $libraries) -ldl -Wl,--no-as-needed -lstdc++

FROM desktop AS native-deps-check-runtime
COPY --from=native-deps-check-build /tmp/native-deps-check /tmp/native-deps-check
USER 65532:65532
RUN /tmp/native-deps-check
DOCKERFILE

docker buildx build --platform "$platform" --target native-deps-check-runtime \
  --output type=cacheonly -f "$context/Dockerfile" "$context"
