---
title: Why Rust?
description: Understand Why We Chose Rust for Flow-Like
sidebar:
    order: 25
---

Flow-Like's core is built in Rust, providing the performance, safety, and reliability needed for a workflow automation platform.

## Why Rust for Flow-Like?

### Type Safety at Every Layer

Rust's type system enables Flow-Like's fully-typed workflows:

- **Compile-time guarantees**: Catch errors before runtime
- **Explicit absence and failure**: `Option<T>` and `Result<T, E>` make callers handle those states
- **Trait-based abstractions**: Nodes, pins, and storage backends share common interfaces

### Performance

Workflow execution benefits from:

- **Zero-cost abstractions**: High-level code compiles to efficient machine code
- **No garbage collector**: Predictable latency for real-time workflows
- **Parallel execution**: Safe concurrency with `async/await` and Rayon

### Memory Safety

For a platform handling user-defined workflows:

- **Safe ownership and borrowing**: Safe Rust prevents many use-after-free,
  aliasing, and data-race bugs; indexing remains bounds-checked
- **Visible native boundaries**: ONNX, LanceDB, and other native integrations
  keep their `unsafe`/FFI boundaries reviewable
- **Controlled concurrency**: Shared state has to satisfy Rust's thread-safety
  contracts before it can cross task and thread boundaries

## Rust in the Codebase

### Core Packages

Key Rust crates in the workspace include:

| Path | Crate | Responsibility |
|------|-------|----------------|
| `packages/core/` | `flow-like` | Public runtime and editor compatibility API |
| `packages/core/runtime/` | `flow-like-runtime` | Boards, execution, and host services |
| `packages/core/editor/` | `flow-like-editor` | FlowScript editing and copilot services |
| `packages/types/` | `flow-like-types` | Shared domain types |
| `packages/storage/` | `flow-like-storage` | Storage abstraction |
| `packages/model-provider/` | `flow-like-model-provider` | AI and ML providers |
| `packages/api/` | `flow-like-api` | REST API |
| `packages/executor/` | `flow-like-executor` | Execution runtime |
| `packages/catalog/` | `flow-like-catalog` | Built-in node implementations |
| `packages/catalog-macros/` | `flow-like-catalog-macros` | Procedural macros for the catalog |

Dependency-light boundaries live below their owning package so the top-level
`packages/` directory stays navigable:

| Path | Crate | Boundary |
|------|-------|----------|
| `packages/core/contracts/` | `flow-like-core-contracts` | Serde-only core/copilot contracts |
| `packages/core/a2ui-schema/` | `flow-like-a2ui-schema` | A2UI schemas and protobuf conversions |
| `packages/core/editor-contracts/` | `flow-like-editor-contracts` | Editing commands, metadata, and layer cache settings |
| `packages/core/dev-check/` | `flow-like-dev-check` | Internal lightweight target for bare Cargo commands |
| `packages/types/contracts/` | `flow-like-types-contracts` | Cache, dispatch, and maintenance wire types |
| `packages/types/proto/` | `flow-like-types-proto` | Protobuf schemas and generated types |
| `packages/types/data-url/` | `flow-like-types-data-url` | Byte, file, and HTTP data URL helpers |
| `packages/storage/contracts/` | `flow-like-storage-contracts` | Graph and vector storage traits |
| `packages/storage/files/` | `flow-like-storage-files` | Object/file stores without the database engine |
| `packages/model-provider/protocol/` | `flow-like-model-protocol` | Model message/response protocol |
| `packages/wasm/schema/` | `flow-like-wasm-schema` | Runtime-independent WASM manifests, node DTOs, widgets, bundles, and compatibility version |
| `packages/api/entity/` | `flow-like-api-entity` | SeaORM entity boundary |

The workspace also contains supporting crates. Treat the root `Cargo.toml`
member list as the authoritative inventory.

Internal dependencies inherit their locations from the root
`[workspace.dependencies]` table through `workspace = true`. Runtime consumers
use the canonical `flow-like-runtime` dependency and select the features they
need:

```toml
[dependencies]
flow-like-runtime = { workspace = true, features = ["flow-metadata"] }
```

Crates that retain `flow_like::...` imports declare
`extern crate flow_like_runtime as flow_like;` at their Rust crate root. The
source alias preserves those imports; Cargo dependency declarations and feature
forwarding use `flow-like-runtime`. The workspace dependency named `flow-like`
continues to provide the public runtime and editor facade. Declare inter-crate
dependency paths in the root workspace manifest. A `path` under `[lib]` names
that crate's own source target and is not an inter-crate dependency.

### Key Dependencies

| Dependency | Purpose |
|------------|---------|
| `tokio` | Async runtime |
| `axum` | HTTP framework for API |
| `serde` | Serialization/deserialization |
| `object_store` | Cloud storage abstraction |
| `lancedb` | Vector database for embeddings |
| `rig-core` | LLM integrations |
| `ort` | ONNX runtime for local ML |
| `tauri` | Desktop app framework |

### Edition 2024

Flow-Like uses Rust Edition 2024 for most packages. Some executor, compiler,
and WASM crates remain on Edition 2021, so check the crate's own
`Cargo.toml` before relying on edition-specific syntax.

- Latest language features
- Improved async ergonomics
- Better compile-time optimizations

## Async Architecture

Flow-Like uses async Rust extensively:

```rust
#[async_trait]
impl NodeLogic for HttpRequestNode {
    async fn run(&self, context: &mut ExecutionContext) -> anyhow::Result<()> {
        let url: String = context.evaluate_pin("url").await?;
        let response = reqwest::get(&url).await?;
        context.set_pin_value("body", json!(response.text().await?)).await?;
        Ok(())
    }
}
```

The `async_trait` crate enables async trait methods, and `tokio` provides the runtime.

## Feature Flags

Conditional compilation selects deployment and runtime capabilities. These
examples come from different workspace crates rather than one shared feature
table:

```toml
[features]
# Enable local ML inference (adds ~100MB to binary)
local-ml = ["flow-like-model-provider/local-ml"]

# Enable Tauri-specific APIs
tauri = ["flow-like-storage/tauri"]

# Enable Kubernetes execution backend
kubernetes = ["kube", "k8s-openapi"]
```

## Error Handling

Flow-Like uses `anyhow` for error handling in application code and `thiserror` for library errors:

```rust
use anyhow::{Result, Context};

async fn load_board(id: &str) -> Result<Board> {
    let bytes = storage
        .get(path)
        .await
        .context("Failed to load board from storage")?;

    serde_json::from_slice(&bytes)
        .context("Failed to deserialize board")
}
```

## Cross-Compilation

The Rust backend compiles for multiple targets:

- **macOS**: `aarch64-apple-darwin`, `x86_64-apple-darwin`
- **Windows**: `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`
- **Linux**: `x86_64-unknown-linux-gnu`
- **iOS**: `aarch64-apple-ios` (with special ONNX handling)

## Development Tools

The tracked Cargo configuration provides short commands for the common loops. A
plain `cargo check` goes through the internal `flow-like-dev-check` facade and
checks the shared AST, contract types, and lightweight core surface; it does not
compile the database runtime, every cloud backend, or every platform target.
Bare `cargo test` and `cargo clippy` cover the selected AST/contracts targets;
use the equally short core aliases below when changing core itself. The facade
exists because Cargo cannot assign features directly to `default-members`, and
it lets the public `flow-like` default remain backward compatible.

```bash
# Shared engine work
cargo check-core
cargo check-core-runtime
cargo check-runtime
cargo check-editor
cargo test-core
cargo test-runtime
cargo test-editor
cargo clippy-core

# File/query storage or the complete database runtime
cargo check-storage
cargo check-storage-runtime

# Desktop and API work
cargo check-desktop
cargo check-api

# Runtime host work without compiling unrelated products
cargo check-wasm
cargo check-executor

# Catalog metadata, desktop execution, or server execution
cargo check-catalog
cargo check-catalog-desktop
cargo check-catalog-server

# Run the desktop app with dependencies optimized enough for usable runtime speed
cargo run-desktop

# Explicit whole-workspace validation (also used by CI)
cargo check-all
cargo test-all
cargo clippy-all
```

Workspace crates inherit a lightweight `flow-like` surface (`flow-metadata`) and storage
surface (`files,query-parser`) from the root manifest. API/desktop products opt
into `app-runtime`, while executor products select `flow-runtime,model` and
catalog execution bundles turn on the database implementation transitively.
The Cargo aliases hide those details in normal use. This keeps ordinary
metadata/editor work off the Arrow, Lance, and DataFusion compile path without
making developers maintain long feature lists.

`flow-like-runtime` owns execution, board models, and host services.
`flow-like-editor` adds FlowScript reconciliation and copilot services. The public
`flow-like` crate combines both through re-exports. Catalog implementations use
the runtime directly, so editor compilation can proceed alongside node compilation.

Catalog implementations compile in domain crates. The standard catalog combines
UI, values, numbers, text, and runtime nodes; data integrations, web connectors,
and media processors have their own crates. The existing catalog facades keep
their public module paths and node order, so application imports do not change.
When editing one domain, check its implementation directly:

```bash
cargo check -p flow-like-catalog-std-text --features execute
cargo check -p flow-like-catalog-data-github --features execute
cargo check -p flow-like-catalog-media-document --features execute
```

Shared cache, query-session, and embedding types live below the node collections.
Keep a shared helper there when another domain needs it. Depending on an entire
catalog to obtain one helper makes that catalog a prerequisite for compilation.
See [the catalog crate guide](https://github.com/Rheosoph/flow-like/blob/dev/packages/catalog/README.md)
for the boundaries and registration requirements.

Feature boundaries are checked independently so feature unification from an
unrelated workspace member cannot hide a missing declaration:

```bash
./tools/check-feature-boundaries.sh --offline --no-rustc-wrapper
```

For reproducible cold, warm, and one-file incremental measurements, use the
non-destructive timing runner. It creates timestamped target directories and
Cargo HTML timing reports; it never runs `cargo clean` or deletes an older run.

```bash
# Lightweight core only (the default scenario)
./tools/compile-times.sh

# Compare lightweight and runtime core explicitly
./tools/compile-times.sh core core-runtime

# Include code generation and linking, then measure a std node edit in desktop
./tools/compile-times.sh --command build desktop-std-string

# Match CI's profile without enabling Rust incremental compilation
./tools/compile-times.sh --command build --profile ci --incremental 0 backend-executor

# Measure the cost of opting a headless server into local ONNX inference
./tools/compile-times.sh catalog-server catalog-server-local-ml

# Every listed scenario; allow several gigabytes per fresh scenario
./tools/compile-times.sh --all-scenarios
```

The runner defaults to `cargo check`. Select `--command build` to include code
generation and linking. Incremental compilation follows the selected profile
unless `--incremental 0` or `--incremental 1` overrides it; the runner clears any
ambient `CARGO_INCREMENTAL` value. The incremental phase touches a source file
and restores its timestamp. Use a representative code edit separately when
measuring how much compiler work an implementation change invalidates.

When Clang and LLD are installed on Linux, or when using Rust's LLD linker on
Windows, opt into the faster linker configuration without changing the shared
portable defaults:

```bash
cargo --config .cargo/fast-compile.toml check-desktop
```

Backend Docker builders share the complete Cargo home between services, which
keeps Cargo's root package-cache locks available while registry and Git data are
reused concurrently. Each service keeps a separate `target/` cache with a
BuildKit lock. Do not split the shared home back into independent `registry/`
and `git/` mounts: Cargo's coordinating lock files live above those folders.

Use the normal Cargo commands for formatting, Clippy, focused tests, and
benchmarks (`cargo fmt`, `cargo clippy -p <crate>`, `cargo test -p <crate>`, and
`cargo bench -p flow-like-catalog`).

## Next Steps

- [Building from Source](/dev/build/) — Set up your development environment
- [Writing Nodes](/dev/writing-nodes/) — Create custom workflow nodes
- [Architecture](/dev/architecture/) — Understand the full system
