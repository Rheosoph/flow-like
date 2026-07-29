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
| `packages/core/` | `flow-like` | Core workflow library |
| `packages/types/` | `flow-like-types` | Shared domain types |
| `packages/storage/` | `flow-like-storage` | Storage abstraction |
| `packages/bits/` | `flow-like-bits` | Reusable components |
| `packages/model-provider/` | `flow-like-model-provider` | AI and ML providers |
| `packages/api/` | `flow-like-api` | REST API |
| `packages/executor/` | `flow-like-executor` | Execution runtime |
| `packages/catalog/` | `flow-like-catalog` | Built-in node implementations |
| `packages/catalog-macros/` | `flow-like-catalog-macros` | Procedural macros for the catalog |

The workspace also contains supporting crates. Treat the root `Cargo.toml` member list as the authoritative inventory.

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

Recommended tools for working with the Rust codebase:

```bash
# Format code
cargo fmt

# Lint with Clippy
cargo clippy

# Run tests
cargo test

# Check compilation without building
cargo check

# Run benchmarks
cargo bench -p flow-like-catalog
```

## Next Steps

- [Building from Source](/dev/build/) — Set up your development environment
- [Writing Nodes](/dev/writing-nodes/) — Create custom workflow nodes
- [Architecture](/dev/architecture/) — Understand the full system
