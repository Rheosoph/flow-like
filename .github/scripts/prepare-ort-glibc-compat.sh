#!/usr/bin/env bash

set -euo pipefail

# Links ort-glibc-compat.c into the Linux build when the toolchain is older than
# the one pyke used to build the prebuilt ONNX Runtime archive. See that file for
# why the shim exists.
#
# The object is appended to RUSTFLAGS rather than emitted from a build script so
# nothing in the shipped sources depends on it. An object file is fully included
# wherever it appears on the linker command line, so its position among the
# archives does not matter.

if [ "$(uname -s)" != "Linux" ]; then
  echo "Not a Linux build host; the ONNX Runtime compatibility shim is not needed."
  exit 0
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source_file="$script_dir/ort-glibc-compat.c"
object_file="${ORT_GLIBC_COMPAT_OBJECT:-${RUNNER_TEMP:-/tmp}/ort-glibc-compat.o}"
compiler="${CC:-cc}"

symbols=(
  __isoc23_strtol
  __isoc23_strtoul
  __isoc23_strtoll
  __isoc23_strtoull
  __isoc23_strtoll_l
  __isoc23_strtoull_l
  __cxa_call_terminate
)

probe_dir="$(mktemp -d)"
trap 'rm -rf "$probe_dir"' EXIT

# Declared with a dummy signature: this only has to reach the linker.
{
  for symbol in "${symbols[@]}"; do
    printf 'extern void %s(void);\n' "$symbol"
  done
  printf 'int main(void) {\n'
  for symbol in "${symbols[@]}"; do
    printf '\t%s();\n' "$symbol"
  done
  printf '\treturn 0;\n}\n'
} > "$probe_dir/probe.c"

if "$compiler" -o "$probe_dir/probe" "$probe_dir/probe.c" -lstdc++ > "$probe_dir/probe.log" 2>&1; then
  echo "The build toolchain already provides every symbol the ONNX Runtime archive references; skipping the compatibility shim."
  exit 0
fi

echo "The build toolchain is missing symbols the ONNX Runtime archive references:"
grep -o "undefined reference to \`[^']*'" "$probe_dir/probe.log" |
  sed -e "s/undefined reference to .//" -e "s/'\$//" |
  sort -u |
  sed 's/^/  /'

if ! "$compiler" -c -O2 -fPIC -o "$object_file" "$source_file"; then
  echo "::error::Failed to compile the ONNX Runtime compatibility shim from $source_file."
  exit 1
fi

for symbol in "${symbols[@]}"; do
  if ! nm --defined-only "$object_file" | grep -q " $symbol\$"; then
    echo "::error::The compiled shim does not define $symbol."
    exit 1
  fi
done

echo "Compiled the ONNX Runtime compatibility shim to $object_file."

if [ -n "${GITHUB_ENV:-}" ]; then
  printf 'RUSTFLAGS=%s\n' "${RUSTFLAGS:+$RUSTFLAGS }-Clink-arg=$object_file" >> "$GITHUB_ENV"
  echo "Appended -Clink-arg=$object_file to RUSTFLAGS."
fi
