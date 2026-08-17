#!/usr/bin/env bash

set -euo pipefail

# Seeds tauri-bundler's tool cache with current linuxdeploy binaries.
#
# tauri-bundler pins linuxdeploy to a fork built 2024-07-29. Upstream added
# libwayland-client.so.0 to the AppImage excludelist on 2024-11-03 ("New version
# of Mesa has some dependency issues with libwayland-client if it is bundled"),
# and linuxdeploy compiles that list in at build time. The pinned binary
# therefore copies the build host's libwayland-client into the AppDir, where
# AppRun's LD_LIBRARY_PATH puts it ahead of the user's own copy. On distributions
# shipping a newer Mesa (Arch, Manjaro) the host libEGL_mesa then fails to
# resolve wl_display_create_queue_with_name, libglvnd ends up with an empty
# vendor list, and eglGetDisplay returns EGL_BAD_PARAMETER. WebKitGTK >= 2.46
# treats that as fatal and aborts the web process, leaving a blank window.
#
# tauri-bundler also downloads linuxdeploy-plugin-appimage without retries and,
# on failure, silently falls back to the legacy AppImageKit runtime that dlopens
# libfuse.so.2 - unmountable on any distribution without fuse2 installed.
#
# Both tools are only fetched when the cached file is absent, so seeding the
# cache overrides them. verify-appimage-bundle.sh asserts the outcome.

arch="${APPIMAGE_TOOLS_ARCH:-$(uname -m)}"

case "$arch" in
  x86_64 | aarch64 | armhf | i686) ;;
  arm64) arch="aarch64" ;;
  *)
    echo "::error::Unsupported AppImage tools architecture: $arch"
    exit 1
    ;;
esac

# Mirrors dirs::cache_dir() as used by tauri-bundler's appimage bundler.
tools_dir="${XDG_CACHE_HOME:-$HOME/.cache}/tauri"
mkdir -p "$tools_dir"

fetch_tool() {
  local url="$1"
  local dest="$2"
  local min_size="$3"
  local staging="$dest.download"

  echo "Fetching $(basename "$dest") from $url"
  if ! curl --fail --location --silent --show-error \
    --retry 5 --retry-delay 5 --retry-all-errors \
    --connect-timeout 30 --max-time 900 \
    --output "$staging" "$url"; then
    rm -f "$staging"
    echo "::error::Failed to download $url"
    return 1
  fi

  local size
  size="$(wc -c < "$staging" | tr -d ' ')"
  if [ "$size" -lt "$min_size" ]; then
    rm -f "$staging"
    echo "::error::$url returned $size bytes, expected at least $min_size."
    return 1
  fi

  # Both tools are AppImages, i.e. ELF executables with an appended payload.
  if [ "$(head -c 4 "$staging" | od -An -tx1 | tr -d ' \n')" != "7f454c46" ]; then
    rm -f "$staging"
    echo "::error::$url did not return an ELF binary."
    return 1
  fi

  chmod +x "$staging"
  mv "$staging" "$dest"
  echo "  $(sha256sum "$dest" 2>/dev/null || shasum -a 256 "$dest")"
}

linuxdeploy="$tools_dir/linuxdeploy-$arch.AppImage"
appimage_plugin="$tools_dir/linuxdeploy-plugin-appimage.AppImage"

fetch_tool \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$arch.AppImage" \
  "$linuxdeploy" \
  4000000

# linuxdeploy resolves plugins from its own directory, and tauri-bundler expects
# this exact arch-less filename in the cache.
fetch_tool \
  "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-$arch.AppImage" \
  "$appimage_plugin" \
  4000000

echo "Seeded AppImage tooling in $tools_dir:"
ls -l "$tools_dir"
