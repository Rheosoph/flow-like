---
title: "Component Model vs Core Modules"
description: "Understand the two WASM runtime models in Flow-Like, their capabilities, and which languages support each"
sidebar:
  order: 26
  badge:
    text: Important
    variant: caution
---

Flow-Like supports two distinct WebAssembly runtime models: the **WASM Component Model** and **Core WASM Modules**. Both allow you to write custom nodes in many languages, but they differ significantly in capabilities, networking, and how they communicate with the host.

## Quick Comparison

| | Component Model | Core Module |
|---|---|---|
| **Interface** | [WIT](https://component-model.bytecodealliance.org/design/wit.html) (WebAssembly Interface Types) | Raw exported functions with manual memory management |
| **Data exchange** | Typed function signatures via canonical ABI | JSON strings passed through linear memory with `alloc`/`dealloc` |
| **WASI version** | Preview 2 (wasip2) | Preview 1 (wasip1) |
| **TCP/UDP sockets** | ✅ Available (permission-gated) | ❌ Not available |
| **DNS resolution** | ✅ Available (permission-gated) | ❌ Not available |
| **HTTP** | ✅ Via WASI HTTP + host bridge | ✅ Via host HTTP bridge only |
| **Host APIs** | Full SDK (logging, pins, variables, cache, streaming, metadata, storage, models, auth) | Full SDK (same APIs, different binding layer) |
| **Binary size** | Slightly larger (includes component metadata) | Smaller raw binaries |
| **Tooling maturity** | Rapidly improving | Stable and well-established |

## How They Work

### Component Model

Component Model nodes use [WIT (WebAssembly Interface Types)](https://component-model.bytecodealliance.org/design/wit.html) to define a typed contract between the node and the Flow-Like runtime. The interface is defined once in a `.wit` file and bindings are auto-generated for your language.

```
┌──────────────────────────────────────────────┐
│              Flow-Like Runtime               │
│                                              │
│  ┌────────────────────────────────────────┐  │
│  │         Wasmtime Component Model       │  │
│  │                                        │  │
│  │  Exports (your code implements):       │  │
│  │    get-node()    → node definition     │  │
│  │    get-nodes()   → all definitions     │  │
│  │    run()         → execute logic       │  │
│  │    get-abi-version() → "0.1.0"        │  │
│  │                                        │  │
│  │  Imports (runtime provides):           │  │
│  │    flow-like:node/logging              │  │
│  │    flow-like:node/pins                 │  │
│  │    flow-like:node/variables            │  │
│  │    flow-like:node/cache                │  │
│  │    flow-like:node/streaming            │  │
│  │    flow-like:node/storage              │  │
│  │    flow-like:node/models               │  │
│  │    flow-like:node/auth                 │  │
│  │    flow-like:node/http                 │  │
│  │    wasi:sockets/tcp (optional)         │  │
│  │    wasi:sockets/udp (optional)         │  │
│  │    wasi:sockets/ip-name-lookup (DNS)   │  │
│  └────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘
```

Key properties:

- Bindings are **auto-generated** from the WIT file — no manual memory management
- Data is exchanged via **typed function parameters**, not raw pointers
- The canonical ABI handles serialization/deserialization transparently
- Full access to **WASI Preview 2** interfaces including sockets

### Core Module

Core Module nodes export raw functions and manage memory manually. The runtime passes JSON strings through WASM linear memory using `alloc`/`dealloc` helper functions.

```
┌──────────────────────────────────────────────┐
│              Flow-Like Runtime               │
│                                              │
│  ┌────────────────────────────────────────┐  │
│  │           Wasmtime Core Module         │  │
│  │                                        │  │
│  │  Exports (your code implements):       │  │
│  │    get_node()    → ptr to JSON         │  │
│  │    get_nodes()   → ptr to JSON         │  │
│  │    run(ptr,len)  → packed ptr+len      │  │
│  │    alloc(size)   → memory ptr          │  │
│  │    dealloc(ptr,size)                   │  │
│  │                                        │  │
│  │  Imports (runtime provides):           │  │
│  │    env::log(ptr, len, level)           │  │
│  │    env::get_input(name_ptr, ...)       │  │
│  │    env::set_output(name_ptr, ...)      │  │
│  │    env::http_request(...)              │  │
│  │    ... (flat C-style host functions)   │  │
│  └────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘
```

Key properties:

- You must implement `alloc` and `dealloc` for memory management
- Data is exchanged as **JSON strings** through linear memory
- Host functions use a flat C-style ABI with raw pointers
- Network access is limited to the **host HTTP bridge** — no direct socket access

---

## Networking Capabilities

This is the most impactful difference between the two models.

### Component Model

Component Model nodes can request access to **WASI sockets**, which provides:

- **TCP** — open connections, listen on ports, stream data
- **UDP** — send and receive datagrams
- **DNS** — resolve hostnames programmatically

These capabilities are **permission-gated** — the user must grant network access before the node can use sockets. This enables use cases like:

- Direct database connections (PostgreSQL, MySQL, Redis)
- Custom protocol implementations
- Low-level network tools
- Real-time data feeds over UDP

### Core Module

Core Module nodes can only access the network through the **host HTTP bridge**, which supports:

- HTTP/HTTPS `GET`, `POST`, `PUT`, `DELETE` requests
- Custom headers and request bodies

This is sufficient for REST API calls and webhook integrations, but does not support raw TCP/UDP connections or custom protocols.

---

## Language Support

### Component Model Languages

These languages produce WASM Component Model binaries with full WIT-based type safety:

| Language | Toolchain | Binding Generator | Target |
|---|---|---|---|
| **Rust** | `cargo` + `wasm32-wasip2` | `wit-bindgen` (proc macro) | Direct component output |
| **Go** | TinyGo + `wasip2` target | `wit-bindgen-go` | Direct component output |
| **C++** | wasi-sdk + CMake | `wit-bindgen c` | Core → `wasm-tools component new` |
| **Zig** | Zig + `wasm32-wasi` | `wit-bindgen c` (via `@cImport`) | Core → `wasm-tools component new` |
| **Swift** | SwiftWasm + wasi SDK | `wit-bindgen c` (via C target) | Core → `wasm-tools component new` |
| **C#** | .NET `wasi-experimental` | Native WIT support | Direct component output |
| **Python** | `componentize-py` | Built-in | Direct component output |
| **TypeScript** | `componentize-js` | Built-in | Direct component output |

:::tip
Rust and Go produce components directly from a single build command. C++, Zig, and Swift first compile to a core WASM module, then use `wasm-tools component new` to wrap it into a component.
:::

### Core Module Languages

These languages produce traditional WASM core modules with manual host function bindings:

| Language | Toolchain | Notes |
|---|---|---|
| **AssemblyScript** | `asc` compiler | TypeScript-like syntax, small binaries |
| **Kotlin** | Kotlin/Wasm (GC + EH) | Requires engine GC and exception handling support |
| **Java** | TeaVM | Converts Java bytecode to WASM |
| **Nim** | Emscripten backend | Compiles via C to WASM |
| **Lua** | Embedded interpreter (C) | Lua 5.4 interpreter compiled to WASM |
| **Grain** | Native WASM target | `--no-gc` + `--use-start-section` for compatibility |
| **MoonBit** | Native WASM target | Bump allocator for linear memory |

---

## Which Should I Choose?

### Use the Component Model when you need:

- **Direct socket access** (TCP, UDP, DNS)
- **Type-safe host bindings** generated from WIT
- **Future-proof architecture** — the Component Model is the direction WASM is heading
- **Interoperability** with other WASM components

### Use Core Modules when:

- Your language **doesn't support the Component Model** yet (e.g., Kotlin, Java, Grain)
- You need the **smallest possible binary size**
- You're building **simple computation-only nodes** that don't need networking

### Decision Flowchart

```
Need TCP/UDP/DNS?
  ├─ Yes → Component Model (Rust, Go, C++, Zig, Swift, C#, Python, TS)
  └─ No
      ├─ Language supports CM? → Prefer Component Model
      └─ Language is CM-unsupported? → Core Module is fine
```

---

## Migration Path

If you have existing Core Module nodes and want to upgrade to the Component Model, the process depends on your language:

1. **Add the WIT file** — Copy `flow-like-node.wit` into your project's `wit/` directory
2. **Generate bindings** — Run the appropriate binding generator for your language
3. **Replace exports** — Swap manual `alloc`/`dealloc`/`get_node`/`run` exports for WIT-generated implementations
4. **Remove memory management** — The canonical ABI handles this for you
5. **Build as component** — Use the Component Model build pipeline for your language

Each [language template](/dev/wasm-nodes/overview/#language-templates) in the repository includes a working `mise.toml` with build tasks you can use as reference.

---

## Runtime Behavior

Both models run inside the same Wasmtime runtime with identical sandboxing. The differences are in the **ABI layer**, not the security model:

| Aspect | Component Model | Core Module |
|---|---|---|
| Sandbox isolation | ✅ Full | ✅ Full |
| Memory caps | ✅ Enforced | ✅ Enforced |
| CPU time limits | ✅ Enforced | ✅ Enforced |
| Permission system | ✅ Per-node | ✅ Per-node |
| Hot reload | ✅ Supported | ✅ Supported |
| Caching | ✅ Compiled modules cached | ✅ Compiled modules cached |

---

## Related

- [WASM Nodes Overview](/dev/wasm-nodes/overview/) — General introduction
- [Sandboxing & Permissions](/dev/wasm-nodes/sandboxing/) — Security model
- [Rust Template](/dev/wasm-nodes/rust/) — Recommended Component Model language
- [Go Template](/dev/wasm-nodes/go/) — Component Model with TinyGo
- [C++ Template](/dev/wasm-nodes/cpp/) — Component Model with wasi-sdk
