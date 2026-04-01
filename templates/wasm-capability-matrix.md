# WASM Template Capability Matrix

This matrix tracks template/runtime parity across language targets in `templates/`.

## Runtime ABI

| Template | Runtime Format | `get_node` | `get_nodes` | `run` | `alloc/dealloc` |
|---|---|---:|---:|---:|---:|
| Rust (`wasm-node-rust`) | WASM Component Model (`wasip2`) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |
| AssemblyScript (`wasm-node-assemblyscript`) | Core WASM module | ✅ | ✅ | ✅ | ✅ |
| Go (`wasm-node-go`) | WASM Component Model (`wasip2`) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |
| C++ (`wasm-node-cpp`) | WASM Component Model (wasi-sdk + wit-bindgen-c) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |
| Kotlin (`wasm-node-kotlin`) | Core WASM module (GC/EH enabled) | ✅ | ✅ | ✅ | ✅ |
| Zig (`wasm-node-zig`) | WASM Component Model (wit-bindgen-c + wasm-tools) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |
| C# (`wasm-node-csharp`) | WASM Component Model (`wasip2`) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |
| Nim (`wasm-node-nim`) | Core WASM module (Emscripten) | ✅ | ✅ | ✅ | ✅ |
| Lua (`wasm-node-lua`) | Core WASM module (Emscripten) | ✅ | ✅ | ✅ | ✅ |
| Swift (`wasm-node-swift`) | WASM Component Model (wit-bindgen-c + SwiftWasm) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |
| Java (`wasm-node-java`) | Core WASM module (TeaVM) | ✅ | ✅ | ✅ | ✅ |
| Grain (`wasm-node-grain`) | Core WASM module | ✅ | ✅ | ✅ | ✅ |
| MoonBit (`wasm-node-moonbit`) | Core WASM module | ✅ | ✅ | ✅ | ✅ |
| Python (`wasm-node-python`) | WASM Component Model (componentize-py) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |
| TypeScript (`wasm-node-typescript`) | WASM Component Model (componentize-js) | ✅ (`get-node`) | ✅ (`get-nodes`) | ✅ | N/A (canonical ABI) |

## Host API Surface (SDK)

| SDK | log | pins | vars | cache | meta | stream | storage | models | http | auth |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Rust (`wasm-sdk-rust`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Go (`wasm-sdk-go`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| C++ (`wasm-sdk-cpp`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Kotlin (`wasm-sdk-kotlin`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Zig (`wasm-sdk-zig`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| AssemblyScript (`wasm-node-assemblyscript/assembly/sdk.ts`) | ⚠️ partial (`env` compatibility layer) | ⚠️ partial | ⚠️ partial | ❌ | ⚠️ limited | ⚠️ partial | ❌ | ❌ | ❌ | ❌ |
| C# (`wasm-sdk-csharp`) | ✅ (component host bridge) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ (no direct runtime bridge yet) | ✅ |
| Nim (`wasm-sdk-nim`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Lua (`wasm-sdk-lua`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Swift (`wasm-sdk-swift`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Java (`wasm-sdk-java`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Grain (`wasm-sdk-grain`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| MoonBit (`wasm-sdk-moonbit`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

## Notes

- **Component Model templates** (Rust, Go, C++, Zig, C#, Swift, Python, TypeScript) execute through `WasmComponent` + `WasmComponentInstance`. They support TCP, UDP, DNS via WASI sockets, plus all custom flow-like host APIs via WIT.
- **Core module templates** (AssemblyScript, Kotlin, Nim, Lua, Java, Grain, MoonBit) execute through `WasmModule` + `WasmInstance`. They only have network access through the host HTTP bridge.
- Rust uses `wit-bindgen` crate (proc macro) + `wasm32-wasip2` target + `wasm-tools component new`.
- Go uses `wit-bindgen-go` + TinyGo `wasip2` target.
- C++ uses `wit-bindgen-c` + wasi-sdk + `wasm-tools component new`.
- Zig uses `wit-bindgen-c` (via `@cImport`) + `wasm32-wasi` target + `wasm-tools component new` with WASI adapter.
- Swift uses `wit-bindgen-c` (via C target) + SwiftWasm + `wasm-tools component new` with WASI adapter.
- C# uses .NET `wasi-experimental` workload with native WIT support.
- Python uses `componentize-py` for direct WIT → WASM component.
- TypeScript uses `componentize-js` for direct WIT → WASM component.
- Kotlin requires engine support for GC + exceptions + function references. No component model path yet.
- Nim compiles to C, then uses Emscripten to produce a core WASM module.
- Lua embeds a Lua 5.4 interpreter in C, compiled with Emscripten.
- Java compiles via TeaVM, which converts Java bytecode to WebAssembly.
- Grain compiles to WASM natively; use `--no-gc` and `--use-start-section` for host ABI compatibility.
- MoonBit compiles to WASM natively; uses bump allocator for linear memory alongside MoonBit's own GC.
