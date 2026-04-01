# Package
version       = "0.1.0"
author        = "Your Name"
description   = "Flow-Like WASM node built with Nim"
license       = "MIT"
srcDir        = "src"
bin           = @["node"]

# Dependencies
requires "nim >= 2.0.0"

# Build task — compile to WASM via Emscripten
task build, "Build WASM node":
  exec "mkdir -p build"
  exec """nim c \
    --cpu:wasm32 \
    --cc:clang \
    --clang.exe:emcc \
    --clang.linkerexe:emcc \
    -d:emscripten \
    -d:release \
    --noMain:on \
    --mm:arc \
    --passC:"-fno-exceptions -msimd128 -mrelaxed-simd -O2 -I../wasm-sdk-nim/src" \
    --passL:"-s STANDALONE_WASM -s EXPORTED_FUNCTIONS=['_get_node','_get_nodes','_run','_alloc','_dealloc','_get_abi_version'] -s ERROR_ON_UNDEFINED_SYMBOLS=0 -s ALLOW_MEMORY_GROWTH=1 --no-entry -O2" \
    -o:build/node.wasm \
    src/node.nim"""

task test, "Run tests (native, not WASM)":
  exec "nim c -r tests/test_node.nim"
