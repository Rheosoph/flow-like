# Flow-Like WASM SDKs

This directory contains SDKs for building **WASM nodes** for the [Flow-Like](https://github.com/Rheosoph/flow-like) runtime. Each SDK targets a different language. They share the node programming model, with different ABI bindings and host API coverage.

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

## State between nodes and calls

A run owns its live Wasm state. For packages with reusable `run` exports, the
runtime retains an instance within the run and its security domain, so guest
globals and heap objects can survive calls from nodes in that package. Each
invocation receives fresh execution data and permissions. The package's host
cache and socket handles are shared within that same runtime scope. A different
security domain uses a separate instance and resource registry.

When the run completes or is cancelled, its instances, cache and sockets are
released. A later run always starts with fresh live state. Persist application
data through storage if needed; stored pointer values and socket handles cannot
restore a client or connection.

The Rust SDK provides a typed `resources` registry for arbitrary guest objects.
One node calls `resources::insert(value)` and outputs the returned string
handle. Another calls `resources::with::<T, _>` or `with_mut::<T, _>` to access
the object, while `remove::<T>` returns ownership and `close::<T>` drops it.
The object stays in guest memory and needs no serialization or `Send`/`Sync`
implementation. Handles are valid only for the owning package instance in the
current run and security domain. See the
[custom buffer example](../../templates/wasm-node-rust/src/package_objects.rs)
and [Rust API reference](wasm-sdk-rust/README.md#store-arbitrary-objects-within-a-run).
Other languages can retain objects in package globals using their own registries.

Run teardown reclaims guest memory and host resources without executing guest
object destructors. Perform graceful client shutdown during a node call before
the run ends. A retained socket client still needs target-compatible networking
APIs and grants; the registry does not drive its guest event loop between calls.

Command-style components executed through `wasi:cli/run` still start with fresh
guest memory on every command. The in-process command path can share the run's
host cache and sockets. The external CLI fallback is rejected during run-scoped
execution because its separate process cannot access these resources. Do not
infer guest-state persistence from the source language alone. It depends on the
compiled artifact's exports and execution path.

The Rust SDK exposes WebSocket listen, accept, connect, send, receive and close
methods. Other SDKs do not currently provide equivalent WebSocket convenience
methods. Their component bindings can import the shared WIT interface directly.
See the [Rust server example](../../templates/wasm-node-rust/src/websocket_server.rs)
and [template lifecycle matrix](../../templates/wasm-capability-matrix.md#state-and-resource-lifetime).

## Memory ABI

Results are passed between the runtime and the WASM module via a **packed i64**:
- High 32 bits: pointer to the JSON string in WASM memory
- Low 32 bits: length of the string

Each SDK handles this packing/unpacking internally. You must also export `alloc(size) → ptr` and `dealloc(ptr, size)` so the runtime can manage WASM memory.

## Node Scores

Every node definition can include optional **NodeScores** — metadata ratings (0.0–1.0) for privacy, security, performance, governance, reliability, and cost. These are used by Flow-Like to surface quality signals in the editor and to enforce policies in controlled deployments.
