# Flow-Like WASM SDKs

This directory contains official SDKs for building **WASM nodes** for the [Flow-Like](https://github.com/TM9657/flow-like) runtime. Each SDK targets a different language but exposes the same programming model and ABI.

## What is a WASM Node?

Flow-Like is a visual, node-based execution engine. Nodes are the building blocks of flows — each node has **input pins**, **output pins**, and executes logic when triggered. WASM nodes are **user-defined nodes compiled to WebAssembly** that the Flow-Like runtime loads and executes safely in a sandboxed environment.

This means you can write custom nodes in virtually any language that compiles to WASM, ship them as a single `.wasm` binary, and run them inside any Flow-Like deployment without native dependencies.

```
┌─────────────────────────────────────────────────────────────────┐
│  Flow-Like Runtime                                              │
│  ┌──────────┐   exec   ┌─────────────────────┐  exec  ┌──────┐ │
│  │  Trigger │ ───────► │  Your WASM Node     │ ─────► │ ...  │ │
│  └──────────┘          │  (any language)     │        └──────┘ │
│                        │  • reads inputs     │                  │
│                        │  • calls host APIs  │                  │
│                        │  • writes outputs   │                  │
│                        └─────────────────────┘                  │
└─────────────────────────────────────────────────────────────────┘
```

## Available SDKs

| Language | Package | Status |
|---|---|---|
| [TypeScript](./wasm-sdk-typescript/) | `@flow-like/wasm-sdk-typescript` on npm | ✅ Published |
| [AssemblyScript](./wasm-sdk-assemblyscript/) | `@flow-like/wasm-sdk-assemblyscript` on npm | ✅ Published |
| [Rust](./wasm-sdk-rust/) | `flow-like-wasm-sdk` on crates.io (planned) | 🚧 In progress |
| [Python](./wasm-sdk-python/) | `flow-like-wasm-sdk` on PyPI (planned) | 🚧 In progress |
| [Go](./wasm-sdk-go/) | Module import (planned) | 🚧 In progress |
| [Zig](./wasm-sdk-zig/) | Build dep (planned) | 🚧 In progress |
| [Kotlin](./wasm-sdk-kotlin/) | Gradle (planned) | 🚧 In progress |
| [C++](./wasm-sdk-cpp/) | CMake (planned) | 🚧 In progress |
| [C#](./wasm-sdk-csharp/) | NuGet (planned) | 🚧 In progress |

## Core Concepts

### Node Definition

Every WASM node declares its interface via a **NodeDefinition** — a schema describing its name, category, description, and all input/output pins. This is returned from `get_nodes()` so Flow-Like can display and wire the node in the visual editor.

### Pins

Pins are the data ports of a node. Each pin has:
- A **name** and **friendly name**
- A **data type** (`String`, `Boolean`, `Integer`, `Float`, `Json`, `Exec`, etc.)
- A **direction** (`Input` or `Output`)
- An optional **default value**
- An optional **JSON schema** for typed objects

`Exec` pins are special — they represent the execution flow (the "wire" that fires the node).

### Context

When a node runs, it receives a **Context** object. This is the primary interface for:
- Reading input pin values (`get_string`, `get_bool`, `get_i64`, `get_json`, …)
- Writing output pin values (`set_output`)
- Logging (`log_debug`, `log_info`, `log_warn`, `log_error`)
- Accessing metadata (`node_id`, `run_id`, `app_id`, `board_id`)
- Working with the board state cache

### ExecutionResult

Every node run returns an `ExecutionResult` with:
- A map of output values
- The output **exec pin name** to fire next (or `null` to stop)
- An optional error message

### Host Bridge

The runtime provides a **Host Bridge** — a set of functions the WASM module can call to interact with the Flow-Like environment. Each SDK wraps these low-level WASM imports into idiomatic high-level APIs.

## Single Node vs. Node Package

SDKs support two export modes:

**Single node** — one `.wasm` file exports exactly one node:
```
get_nodes() → NodeDefinition JSON
run(ptr, len) → ExecutionResult JSON
```

**Node package** — one `.wasm` file exports multiple nodes, dispatched by name:
```
get_nodes() → PackageNodes JSON (array of NodeDefinitions)
run(ptr, len) → ExecutionResult JSON (dispatches to correct handler)
```

## Memory ABI

Results are passed between the runtime and the WASM module via a **packed i64**:
- High 32 bits: pointer to the JSON string in WASM memory
- Low 32 bits: length of the string

Each SDK handles this packing/unpacking internally. You must also export `alloc(size) → ptr` and `dealloc(ptr, size)` so the runtime can manage WASM memory.

## Node Scores

Every node definition can include optional **NodeScores** — metadata ratings (0.0–1.0) for privacy, security, performance, governance, reliability, and cost. These are used by Flow-Like to surface quality signals in the editor and to enforce policies in controlled deployments.
