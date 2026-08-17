#!/usr/bin/env bash

set -euo pipefail

# Asserts the two Linux packaging properties that silently regressed in the past:
#
#   1. The AppDir must not bundle any library on the AppImage excludelist.
#      Bundling libwayland-client.so.0 shipped a blank window to every
#      Arch-family user in 0.1.6 and 0.1.7 (see prepare-appimage-tools.sh).
#   2. The produced AppImage must use the static type2-runtime, not the legacy
#      AppImageKit runtime that dlopens libfuse.so.2. tauri-bundler falls back to
#      the legacy runtime, with only a log line, when its plugin download fails.
#      0.1.7 amd64 shipped that way and cannot mount without fuse2 installed.

bundle_dir="${RELEASE_BUNDLE_DIR:-target/release/bundle}"
appimage_dir="$bundle_dir/appimage"
excludelist_url="${APPIMAGE_EXCLUDELIST_URL:-https://raw.githubusercontent.com/AppImageCommunity/pkg2appimage/master/excludelist}"

if [ ! -d "$appimage_dir" ]; then
  echo "::error::No AppImage bundle directory found at $appimage_dir."
  exit 1
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

excludelist="$work_dir/excludelist"

if curl --fail --location --silent --show-error \
  --retry 3 --retry-delay 5 --retry-all-errors \
  --connect-timeout 20 --max-time 120 \
  --output "$work_dir/excludelist.raw" "$excludelist_url"; then
  sed -e 's/#.*//' -e 's/[[:space:]]*$//' "$work_dir/excludelist.raw" |
    grep -E '^lib.*\.so' | sort -u > "$excludelist"
else
  # Never let a network failure turn this gate into a no-op. This subset is the
  # graphics and IPC core of the upstream list, which is what actually breaks.
  echo "::warning::Could not fetch the AppImage excludelist; falling back to the built-in critical subset."
  cat > "$excludelist" <<'EOF'
libEGL.so.1
libGL.so.1
libGLX.so.0
libGLdispatch.so.0
libdrm.so.2
libgbm.so.1
libglapi.so.0
libwayland-client.so.0
EOF
fi

echo "Excludelist entries checked: $(wc -l < "$excludelist" | tr -d ' ')"

status=0
appdir_count=0

while IFS= read -r -d '' appdir; do
  appdir_count=$((appdir_count + 1))
  echo
  echo "Checking AppDir: $appdir"

  bundled="$work_dir/bundled"
  find "$appdir" -type f -name '*.so*' -exec basename {} \; | sort -u > "$bundled"
  echo "  bundled shared objects: $(wc -l < "$bundled" | tr -d ' ')"

  offenders="$work_dir/offenders"
  comm -12 "$excludelist" "$bundled" > "$offenders"

  if [ -s "$offenders" ]; then
    status=1
    echo "::error::$appdir bundles libraries that must come from the host system:"
    while IFS= read -r lib; do
      echo "::error::  $lib"
      find "$appdir" -type f -name "$lib" | sed 's/^/    /'
    done < "$offenders"
    echo "::error::Bundling these shadows the user's own graphics stack via AppRun's LD_LIBRARY_PATH and breaks EGL on distributions with a newer Mesa."
  else
    echo "  no excludelisted libraries bundled"
  fi
done < <(find "$appimage_dir" -maxdepth 1 -type d -name '*.AppDir' -print0)

if [ "$appdir_count" -eq 0 ]; then
  echo "::error::No .AppDir found under $appimage_dir."
  exit 1
fi

appimage_count=0

while IFS= read -r -d '' appimage; do
  appimage_count=$((appimage_count + 1))
  echo
  echo "Checking runtime: $appimage"

  head -c 1048576 "$appimage" > "$work_dir/runtime.bin"

  if grep -qa 'type2-runtime' "$work_dir/runtime.bin"; then
    echo "  static type2-runtime detected"
  else
    status=1
    echo "::error::$appimage was not built with the static type2-runtime."
  fi

  if grep -qa 'error loading libfuse.so.2' "$work_dir/runtime.bin"; then
    status=1
    echo "::error::$appimage uses the legacy AppImageKit runtime that dlopens libfuse.so.2, so it cannot mount on distributions without fuse2."
    echo "::error::This means tauri-bundler fell back to its built-in AppImage plugin; check the build log for 'Download of AppImage plugin failed'."
  fi
done < <(find "$appimage_dir" -maxdepth 1 -type f -name '*.AppImage' -print0)

if [ "$appimage_count" -eq 0 ]; then
  echo "::error::No .AppImage found under $appimage_dir."
  exit 1
fi

echo
if [ "$status" -ne 0 ]; then
  echo "::error::AppImage verification failed."
  exit 1
fi

echo "AppImage verification passed ($appdir_count AppDir(s), $appimage_count AppImage(s))."
