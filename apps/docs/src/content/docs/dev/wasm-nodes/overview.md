---
title: Custom WASM Nodes
description: Create custom workflow nodes using WebAssembly
sidebar:
  order: 25
  badge:
    text: Beta
    variant: tip
---

Flow-Like supports custom nodes written in **any language that compiles to WebAssembly**. This allows you to extend Flow-Like with your own logic without modifying the core Rust codebase.

## Why WASM?

| Benefit | Description |
|---------|-------------|
| **Language Freedom** | Write nodes in Rust, Go, TypeScript, Python, C++, or any WASM-compatible language |
| **Sandboxed Execution** | WASM runs in a secure sandbox with controlled memory and capabilities |
| **Portable** | Same WASM module works on desktop, server, and browser |
| **Performance** | Near-native execution speed |
| **Hot Reload** | Load new nodes without restarting Flow-Like |
| **Package Registry** | Share and discover community nodes |

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Flow-Like Runtime                         │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
│  │ Native Node │    │ Native Node │    │  WASM Node  │     │
│  │   (Rust)    │    │   (Rust)    │    │  (Any Lang) │     │
│  └─────────────┘    └─────────────┘    └──────┬──────┘     │
│                                               │             │
│                                        ┌──────▼──────┐     │
│                                        │ WASM Runtime │     │
│                                        │  (Wasmtime)  │     │
│                                        └─────────────┘     │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## Packages vs Single Nodes

WASM nodes are distributed as **packages** that can contain one or more nodes:

| Type | Use Case |
|------|----------|
| **Single Node** | Simple, focused functionality |
| **Multi-Node Package** | Related nodes that share code (e.g., math operators) |

## Package Manifest

Every package requires a `manifest.toml` that declares:

- Package metadata (name, version, author)
- Permission requirements
- OAuth scope requirements

Nodes are automatically extracted from the WASM binary — they are not declared in the manifest.

```toml title="manifest.toml"
manifest_version = 1
id = "com.example.math-utils"
name = "Math Utilities"
version = "1.0.0"
description = "Common math operations"

[permissions]
memory = "standard"     # 64 MB
timeout = "standard"    # 30 seconds
variables = true
cache = true
```

See [Package Manifest](/dev/wasm-nodes/manifest/) for full documentation.

## Permission System

Packages must declare their required permissions upfront. Users can review these before installing.

### Memory Tiers

| Tier | Memory | Use Case |
|------|--------|----------|
| `minimal` | 16 MB | Simple string processing |
| `light` | 32 MB | Basic data manipulation |
| `standard` | 64 MB | Most nodes (default) |
| `heavy` | 128 MB | Data processing |
| `intensive` | 256 MB | ML inference, large datasets |
| `large` | 512 MB | Large model inference |
| `huge` | 1 GB | Very large datasets |
| `extreme` | 2 GB | Heavy computation |
| `maximum` | 4 GB | Maximum allocation |

### Timeout Tiers

| Tier | Duration | Use Case |
|------|----------|----------|
| `quick` | 5 seconds | Fast operations |
| `standard` | 30 seconds | Most nodes (default) |
| `extended` | 60 seconds | API calls |
| `long_running` | 5 minutes | ML inference |
| `very_long` | 10 minutes | Heavy processing |
| `maximum` | 30 minutes | Maximum duration |

### OAuth Scopes

Packages can request OAuth access per-provider:

```toml
[[permissions.oauth_scopes]]
provider = "google"
scopes = ["https://www.googleapis.com/auth/drive.readonly"]
reason = "Read files from Google Drive"
required = true
```

## Data Types

WASM nodes can use these pin data types:

| Type | Description | JSON Representation |
|------|-------------|---------------------|
| `Execution` | Flow control trigger | `null` |
| `String` | Text value | `"hello"` |
| `Integer` | 64-bit signed integer | `42` |
| `Float` | 64-bit floating point | `3.14` |
| `Boolean` | True/false | `true` |
| `Date` | ISO 8601 timestamp | `"2025-01-01T00:00:00Z"` |
| `PathBuf` | Storage path | `"uploads/file.txt"` |
| `Struct` | JSON object | `{"key": "value"}` |
| `Byte` | Raw bytes (base64) | `"SGVsbG8gV29ybGQ="` |
| `Generic` | Any type (dynamic) | varies |

## Value Types

Pins can hold single values or collections:

| ValueType | Description |
|-----------|-------------|
| `Normal` | Single value |
| `Array` | Ordered list `[...]` |
| `HashMap` | Key-value map `{...}` |
| `HashSet` | Unique values set |

## Quality Scores

Set quality metrics (0-10 scale) to help users understand node trade-offs:

| Score | Meaning |
|-------|---------|
| `privacy` | Data protection level (10 = very private) |
| `security` | Attack resistance (10 = very secure) |
| `performance` | Execution speed (10 = very slow, expensive) |
| `governance` | Compliance level (10 = highly auditable) |
| `reliability` | Stability (10 = may fail often) |
| `cost` | Resource usage (10 = expensive) |

## Language Templates

Choose your preferred language to get started. See [Component Model vs Core Modules](/dev/wasm-nodes/runtime-models/) for a detailed comparison of what each runtime model supports.

### Component Model (Recommended)

These languages produce WASM Component Model binaries with typed WIT bindings, WASI Preview 2 support, and optional TCP/UDP/DNS networking:

| Language | Template | Notes |
|----------|----------|-------|
| [Rust](/dev/wasm-nodes/rust/) | `wasm-node-rust` | Recommended — smallest binaries, best tooling |
| [Go](/dev/wasm-nodes/go/) | `wasm-node-go` | TinyGo with `wasip2` target |
| [C++](/dev/wasm-nodes/cpp/) | `wasm-node-cpp` | wasi-sdk + wit-bindgen |
| [Zig](/dev/wasm-nodes/zig/) | `wasm-node-zig` | wit-bindgen via `@cImport` |
| [Swift](/dev/wasm-nodes/swift/) | `wasm-node-swift` | SwiftWasm + wit-bindgen |
| [C#](/dev/wasm-nodes/csharp/) | `wasm-node-csharp` | .NET `wasi-experimental` workload |
| [Python](/dev/wasm-nodes/python/) | `wasm-node-python` | componentize-py |
| [TypeScript](/dev/wasm-nodes/typescript/) | `wasm-node-typescript` | componentize-js |

### Core Modules

These languages produce traditional WASM core modules. They have access to all host APIs via the HTTP bridge, but cannot use TCP/UDP/DNS sockets:

| Language | Template | Notes |
|----------|----------|-------|
| AssemblyScript | `wasm-node-assemblyscript` | TypeScript-like syntax, small binaries |
| Kotlin | `wasm-node-kotlin` | Requires engine GC + exception handling support |
| Java | `wasm-node-java` | Via TeaVM |
| Nim | `wasm-node-nim` | Compiles via Emscripten |
| Lua | `wasm-node-lua` | Embedded Lua 5.4 interpreter |
| Grain | `wasm-node-grain` | Native WASM target |
| MoonBit | `wasm-node-moonbit` | Native WASM target |

## Installation & Registry

### Installing Packages

Packages can be installed from multiple sources:

1. **Registry** — Browse and install from the Flow-Like registry
2. **Local file** — Load `.wasm` files from disk
3. **URL** — Install from a direct download URL

```bash
# From registry (coming soon)
flow-like install com.example.math-utils

# From local file
flow-like install ./my-package.wasm
```

### Offline Support

Installed packages are cached locally, enabling:

- Full offline functionality
- Fast startup without network
- Automatic background updates when online

### Publishing

Share your packages with the community:

```bash
# Build your package
cargo build --release --target wasm32-wasip1

# Publish to registry (requires API key)
flow-like publish ./target/wasm32-wasip1/release/my_package.wasm
```

See the [Package Registry](/dev/wasm-nodes/registry/) documentation for details on the publishing process and governance.

## Security Considerations

WASM nodes run in a sandboxed environment with:

- **Declared permissions only** — Packages can only use permissions they declare
- **Per-node OAuth** — Nodes only get OAuth tokens they specifically request
- **Memory limits** — Enforced per the declared memory tier
- **Execution timeout** — Prevents infinite loops
- **No arbitrary filesystem** — Must use Flow-Like's storage API
- **No arbitrary network** — Must use Flow-Like's HTTP capabilities

## Next Steps

→ [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
→ [Package Registry](/dev/wasm-nodes/registry/) — Publishing and governance
