#!/usr/bin/env bash
set -euo pipefail

# Runs only in the digest-pinned Ubuntu container in standalone-release.yml.
: "${RUNTIME_PACK:?}" "${UBUNTU_SNAPSHOT:?}" "${SOURCE_DATE_EPOCH:?}"
case "$RUNTIME_PACK" in
  llamacpp-linux-x64-cpu|llamacpp-linux-x64-vulkan|llamacpp-linux-arm64-cpu) ;;
  *) echo "Unsupported Linux runtime pack: $RUNTIME_PACK" >&2; exit 1 ;;
esac
[[ "$UBUNTU_SNAPSHOT" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]
# The minimal image has no CA bundle. Borrow the runner's public trust roots until
# the frozen ca-certificates package is installed. Ubuntu's archive keys verify APT's indices.
printf 'Acquire::https::CaInfo "/bootstrap-ca.crt";\n' > /etc/apt/apt.conf.d/99-runtime-bootstrap-ca
rm -f /etc/apt/sources.list.d/*
cat > /etc/apt/sources.list <<EOF
deb [check-valid-until=no] https://snapshot.ubuntu.com/ubuntu/$UBUNTU_SNAPSHOT jammy main universe
deb [check-valid-until=no] https://snapshot.ubuntu.com/ubuntu/$UBUNTU_SNAPSHOT jammy-updates main universe
deb [check-valid-until=no] https://snapshot.ubuntu.com/ubuntu/$UBUNTU_SNAPSHOT jammy-security main universe
EOF
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  ca-certificates python3 binutils patchelf libgomp1 libstdc++6 libgcc-s1 libssl3 libvulkan1
rm /etc/apt/apt.conf.d/99-runtime-bootstrap-ca
cd /work
if [[ "$RUNTIME_PACK" == llamacpp-linux-arm64-cpu ]]; then
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gcc-12 g++-12 libc6-dev cmake make git libssl-dev xz-utils bzip2 flex bison texinfo zlib1g-dev
  python3 .github/scripts/runtime_toolchain.py build
  export RUNTIME_TOOLCHAIN_ROOT=/toolchain
  export PATH="/toolchain/bin:$PATH"
  LD_LIBRARY_PATH="$(python3 .github/scripts/runtime_toolchain.py library-path)"
  export LD_LIBRARY_PATH
  # The packing smoke checks both ordinary host lookup and the pack's fallback copies.
  # Load this before Ubuntu's aarch64-linux-gnu.conf so the pinned C++ runtime wins.
  printf '%s\n' "$LD_LIBRARY_PATH" | tr ':' '\n' > /etc/ld.so.conf.d/00-flow-runtime-gcc.conf
  ldconfig
  LLAMACPP_COMMIT="$(python3 .github/scripts/runtime_packs.py pin | sed -n 's/^commit=//p')"
  LLAMACPP_NUMBER="$(python3 .github/scripts/runtime_packs.py pin | sed -n 's/^number=//p')"
  git init -q /scratch/llama.cpp
  git -C /scratch/llama.cpp fetch -q --depth 1 https://github.com/ggml-org/llama.cpp "$LLAMACPP_COMMIT"
  git -C /scratch/llama.cpp checkout -q --detach FETCH_HEAD
  test "$(git -C /scratch/llama.cpp rev-parse HEAD)" = "$LLAMACPP_COMMIT"
  cmake -S /scratch/llama.cpp -B /scratch/llama-build -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER=/toolchain/bin/gcc -DCMAKE_CXX_COMPILER=/toolchain/bin/g++ \
    -DCMAKE_INSTALL_RPATH='$ORIGIN' -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
    -DGGML_BACKEND_DL=ON -DGGML_NATIVE=OFF -DGGML_CPU_ALL_VARIANTS=ON -DGGML_RPC=OFF \
    -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_TOOLS=ON -DLLAMA_BUILD_SERVER=ON \
    -DLLAMA_BUILD_UI=OFF -DLLAMA_USE_PREBUILT_UI=OFF \
    -DLLAMA_BUILD_NUMBER="$LLAMACPP_NUMBER" -DLLAMA_BUILD_COMMIT="${LLAMACPP_COMMIT:0:9}"
  cmake --build /scratch/llama-build --config Release -j "$(nproc)"
  cp /scratch/llama.cpp/LICENSE /scratch/llama-build/bin/LICENSE
  source=/scratch/llama-build/bin
else
  source="$(python3 .github/scripts/runtime_packs.py fetch --pack "$RUNTIME_PACK" --output /scratch/upstream)"
fi
python3 .github/scripts/runtime_packs.py llamacpp --pack "$RUNTIME_PACK" --source "$source" \
  --epoch "$SOURCE_DATE_EPOCH" --output /scratch/runtime-packs
