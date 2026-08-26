#!/usr/bin/env bash

set -euo pipefail
export LC_ALL=C

# Linux release artifacts must run on the oldest distribution we support. A
# newer build host can raise the required GLIBC symbol versions in the desktop
# executable and in libraries copied into the AppImage. glibc itself is supplied
# by the user's system, so packaging those newer files produces an artifact that
# cannot start on the declared baseline.

bundle_dir="${RELEASE_BUNDLE_DIR:-target/release/bundle}"
max_glibc_version="${MAX_GLIBC_VERSION:-2.35}"

if [ ! -d "$bundle_dir" ]; then
  echo "::error::No Linux bundle directory found at $bundle_dir."
  exit 1
fi

if ! command -v readelf >/dev/null 2>&1; then
  echo "::error::readelf is required to verify Linux glibc compatibility."
  exit 1
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

version_is_newer() {
  local version="$1"
  local ceiling="$2"

  [ "$version" != "$ceiling" ] &&
    [ "$(printf '%s\n%s\n' "$version" "$ceiling" | sort -V | tail -n 1)" = "$version" ]
}

glibc_requirements() {
  local elf="$1"

  readelf --version-info --wide "$elf" 2>/dev/null |
    awk '
      /^Version needs section/ {
        in_needs = 1
        next
      }
      /^Version (symbols|definition) section/ {
        in_needs = 0
      }
      in_needs {
        for (field = 1; field <= NF; field++) {
          if ($field == "Name:" && $(field + 1) ~ /^GLIBC_/) {
            print $(field + 1)
          }
        }
      }
    '
}

requirement_is_incompatible() {
  local requirement="$1"
  local version

  if [[ ! "$requirement" =~ ^GLIBC_[0-9]+(\.[0-9]+)+$ ]]; then
    # Special ABI tags such as GLIBC_ABI_DT_RELR can introduce a newer loader
    # requirement without carrying a numeric version. Reject unknown tags so
    # they cannot silently bypass the declared baseline.
    return 0
  fi

  version="${requirement#GLIBC_}"
  version_is_newer "$version" "$max_glibc_version"
}

status=0
scan_count=0

scan_tree() {
  local label="$1"
  local root="$2"
  local elf_count=0
  local requirement_count=0
  local tree_status=0
  local inspection_errors
  local offenders
  local requirements

  scan_count=$((scan_count + 1))
  offenders="$work_dir/offenders-$scan_count"
  inspection_errors="$work_dir/inspection-errors-$scan_count"
  : > "$offenders"
  : > "$inspection_errors"

  while IFS= read -r -d '' candidate; do
    if ! readelf --file-header "$candidate" >/dev/null 2>&1; then
      continue
    fi

    elf_count=$((elf_count + 1))
    requirements="$work_dir/requirements-$scan_count-$elf_count"

    if ! glibc_requirements "$candidate" | sort -Vu > "$requirements"; then
      printf '%s\n' "${candidate#"$root"/}" >> "$inspection_errors"
      continue
    fi

    while IFS= read -r requirement; do
      requirement_count=$((requirement_count + 1))
      if requirement_is_incompatible "$requirement"; then
        printf '%s\t%s\n' "${candidate#"$root"/}" "$requirement" >> "$offenders"
      fi
    done < "$requirements"
  done < <(find "$root" -type f -print0)

  echo
  echo "Checking glibc compatibility: $label"
  echo "  ELF files checked: $elf_count"
  echo "  GLIBC requirements checked: $requirement_count"

  if [ "$elf_count" -eq 0 ]; then
    status=1
    tree_status=1
    echo "::error::$label contains no ELF files to verify."
  fi

  if [ -s "$inspection_errors" ]; then
    status=1
    tree_status=1
    echo "::error::readelf could not inspect every ELF file in $label:"
    while IFS= read -r file; do
      echo "::error::  $file"
    done < "$inspection_errors"
  elif [ "$elf_count" -gt 0 ] && [ "$requirement_count" -eq 0 ]; then
    status=1
    tree_status=1
    echo "::error::$label contains no GLIBC requirements to verify."
  fi

  if [ -s "$offenders" ]; then
    status=1
    tree_status=1
    echo "::error::$label contains ELF files that are incompatible with the GLIBC_$max_glibc_version baseline:"
    while IFS=$'\t' read -r file requirement; do
      echo "::error::  $file requires $requirement"
    done < <(sort -u "$offenders")
  fi

  if [ "$tree_status" -eq 0 ]; then
    echo "  all GLIBC requirements are <= $max_glibc_version"
  fi
}

# Tauri builds the AppImage, Debian package and RPM from the same release
# executable and resources. The AppDir scan therefore covers the RPM payload as
# well as the additional shared libraries bundled only with the AppImage.
appdir_count=0
appimage_dir="$bundle_dir/appimage"

if [ -d "$appimage_dir" ]; then
  while IFS= read -r -d '' appdir; do
    appdir_count=$((appdir_count + 1))
    scan_tree "AppDir $appdir" "$appdir"
  done < <(find "$appimage_dir" -maxdepth 1 -type d -name '*.AppDir' -print0)
fi

if [ "$appdir_count" -eq 0 ]; then
  echo "::error::No .AppDir found under $appimage_dir."
  exit 1
fi

deb_count=0
deb_dir="$bundle_dir/deb"

if [ -d "$deb_dir" ]; then
  while IFS= read -r -d '' deb; do
    deb_count=$((deb_count + 1))

    if ! command -v dpkg-deb >/dev/null 2>&1; then
      echo "::error::dpkg-deb is required to verify $deb."
      exit 1
    fi

    extracted="$work_dir/deb-$deb_count"
    mkdir -p "$extracted"
    dpkg-deb --extract "$deb" "$extracted"
    scan_tree "Debian package $deb" "$extracted"
  done < <(find "$deb_dir" -maxdepth 1 -type f -name '*.deb' -print0)
fi

if [ "$deb_count" -eq 0 ]; then
  echo "::error::No .deb package found under $deb_dir."
  exit 1
fi

echo
if [ "$status" -ne 0 ]; then
  echo "::error::Linux glibc compatibility verification failed."
  exit 1
fi

echo "Linux glibc compatibility passed at GLIBC_$max_glibc_version ($appdir_count AppDir(s), $deb_count Debian package(s))."
