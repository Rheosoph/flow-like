#!/usr/bin/env bash

set -euo pipefail
export LC_ALL=C

# The `libspa` crate (pulled in by xcap -> pipewire, for Wayland screen capture)
# unconditionally reads `spa_video_info_raw.flags`, a field PipeWire added in
# 0.3.65 alongside widening `modifier` to uint64_t. Ubuntu 22.04 ships PipeWire
# 0.3.48, so bindgen emits the older struct and the crate fails to compile.
#
# libspa-sys declares only `libpipewire-0.3 >= 0.3`, so pkg-config accepts the
# stale headers and the mismatch surfaces as a Rust type error instead.
#
# libspa-0.2 is a headers-only pkg-config module (no Libs:, the SPA plugins are
# dlopened), so refreshing its headers is a build-time change only: libpipewire
# itself stays the distribution's, and the artifact's glibc floor is unaffected.

spa_version="${SPA_HEADERS_VERSION:-1.0.5}"

if [ "$(uname -s)" != "Linux" ]; then
  exit 0
fi

if ! command -v pkg-config >/dev/null 2>&1; then
  echo "::error::pkg-config is required to locate the SPA headers."
  exit 1
fi

if ! pkg-config --exists libspa-0.2; then
  echo "::error::libspa-0.2 not found. Install libpipewire-0.3-dev first."
  exit 1
fi

spa_include_dir="$(
  pkg-config --cflags-only-I libspa-0.2 |
    tr ' ' '\n' |
    sed -n 's/^-I//p' |
    grep 'spa-0\.2$' |
    head -n 1
)"

if [ -z "$spa_include_dir" ] || [ ! -d "$spa_include_dir/spa" ]; then
  echo "::error::Could not locate the spa-0.2 include directory via pkg-config."
  exit 1
fi

raw_header="$spa_include_dir/spa/param/video/raw.h"

has_video_flags() {
  [ -f "$raw_header" ] &&
    sed -n '/^struct spa_video_info_raw/,/^};/p' "$raw_header" |
    grep -q 'uint32_t[[:space:]]\+flags'
}

if has_video_flags; then
  echo "SPA headers at $spa_include_dir already expose spa_video_info_raw.flags."
  exit 0
fi

echo "Refreshing SPA headers in $spa_include_dir to PipeWire $spa_version" \
  "(installed libpipewire-0.3: $(pkg-config --modversion libpipewire-0.3))."

sudo_cmd=()
if [ "$(id -u)" -ne 0 ]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "::error::Refreshing $spa_include_dir requires root or sudo."
    exit 1
  fi
  sudo_cmd=(sudo)
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

archive_url="https://gitlab.freedesktop.org/pipewire/pipewire/-/archive/$spa_version/pipewire-$spa_version.tar.gz"

if ! curl -sSfL --retry 3 --retry-delay 5 "$archive_url" -o "$work_dir/pipewire.tar.gz"; then
  echo "::error::Failed to download PipeWire $spa_version from $archive_url."
  exit 1
fi

tar -xzf "$work_dir/pipewire.tar.gz" -C "$work_dir"

upstream_spa="$work_dir/pipewire-$spa_version/spa/include/spa"
if [ ! -d "$upstream_spa" ]; then
  echo "::error::PipeWire $spa_version archive did not contain spa/include/spa."
  exit 1
fi

"${sudo_cmd[@]}" cp -r "$upstream_spa/." "$spa_include_dir/spa/"

if ! has_video_flags; then
  echo "::error::spa_video_info_raw.flags still missing after refreshing headers."
  exit 1
fi

echo "SPA headers refreshed to PipeWire $spa_version."
