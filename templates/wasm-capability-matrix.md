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

## State and resource lifetime

Live state belongs to a run. Reusable export-based packages retain guest memory
between calls within the same package and security domain. Calls to an instance
are serialized. A node's inputs, outputs, logs and permissions are refreshed for
each invocation, while its package's host cache and sockets remain available to
later calls in that scope. Different security domains have separate instances
and resource registries, even within the same package and run.

| Execution path | Guest globals between calls | Package host cache and sockets |
|---|---|---|
| Core module with `run` export | Retained within the run and security domain | Shared within the package, run and security domain |
| Component with `run` export | Retained within the run and security domain | Shared within the package, run and security domain |
| In-process `wasi:cli/run` command fallback | Fresh for each command | Shared through the command's host context |
| External Wasmtime CLI fallback | Rejected during run-scoped execution | Separate process cannot access the run's resources |

Completion and cancellation release all of this live state. A later run starts
fresh, even for the same package. A stored connection handle or pointer is not
valid in the next run. This lifetime is determined by exports and runtime path,
so a language's Component Model support alone does not establish reuse.

Kotlin, MoonBit and Grain packages should be rebuilt with the updated SDK and
the optional `reset_scratch` export. This lets the runtime reuse transient ABI
buffers between calls while preserving guest objects. Input and result pointers
remain borrowed for one invocation; retain parsed values instead. Older binaries
cannot gain this export through a runtime update and can keep accumulating ABI
allocations within the run. Guest heap allocations remain subject to the memory
budget, including Grain builds that disable garbage collection.

The Rust SDK wraps the host WebSocket listener and connection APIs. The
[Rust template example](wasm-node-rust/src/websocket_server.rs) passes handles
through Start Server, Accept Connection and Send Text nodes. Each node declares
`NetworkWebsocket`, and the runtime's network policy applies. Other SDKs do not
yet wrap these WebSocket APIs; custom component bindings can use the shared WIT
interface. Core-module users need the corresponding `flowlike_ws` host imports.
String connection handles use `connect_ref`, `send_ref`, `receive_ref` and
`close_ref`; listener handles use `listen`, `accept` and `local_address`.
The original connection imports use numeric handles and are separate from this
string-handle API.

## Notes

- **Component Model templates** (Rust, Go, C++, Zig, C#, Swift, Python, TypeScript) execute through `WasmComponent` + `WasmComponentInstance`. They support TCP, UDP, DNS via WASI sockets, plus all custom flow-like host APIs via WIT.
- **Core module templates** (AssemblyScript, Kotlin, Nim, Lua, Java, Grain, MoonBit) execute through `WasmModule` + `WasmInstance`. Network access uses host bridges; these SDKs do not currently wrap the WebSocket imports.
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
