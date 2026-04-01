---
title: Rust WASM Nodes
description: Create custom WASM nodes using Rust with the WASM Component Model
sidebar:
  order: 1
  badge:
    text: Recommended
    variant: tip
---

Rust is the **recommended language** for WASM nodes — it produces the smallest binaries and has the best WASM tooling. The Flow-Like SDK provides macros for zero-boilerplate node development.

## Prerequisites

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add WASM target
rustup target add wasm32-wasip1
```

## Quick Start with SDK

### Project Setup

```bash
cargo new --lib my-wasm-nodes
cd my-wasm-nodes
```

Update `Cargo.toml`:

```toml title="Cargo.toml"
[package]
name = "my-wasm-nodes"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
flow-like-wasm-sdk = { git = "https://github.com/TM9657/flow-like", branch = "dev" }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
```

### Single Node Example

```rust title="src/lib.rs"
use flow_like_wasm_sdk::*;

// Define the node using the macro
node! {
    name: "uppercase",
    friendly_name: "Uppercase",
    description: "Converts text to uppercase",
    category: "Custom/Text",

    inputs: {
        exec: Exec,
        text: String = "",
    },

    outputs: {
        exec_out: Exec,
        result: String,
    },
}

// Implement the run logic
run_node!(handle_run);

fn handle_run(mut ctx: Context) -> ExecutionResult {
    let text = ctx.get_string("text").unwrap_or_default();
    ctx.set_output("result", text.to_uppercase());
    ctx.success()
}
```

### Multi-Node Package

For packages with multiple related nodes:

```rust title="src/lib.rs"
use flow_like_wasm_sdk::*;

package! {
    nodes: [
        {
            name: "add",
            friendly_name: "Add",
            description: "Adds two numbers",
            category: "Custom/Math",
            inputs: { exec: Exec, a: I64 = 0, b: I64 = 0 },
            outputs: { exec_out: Exec, result: I64 },
        },
        {
            name: "subtract",
            friendly_name: "Subtract",
            description: "Subtracts two numbers",
            category: "Custom/Math",
            inputs: { exec: Exec, a: I64 = 0, b: I64 = 0 },
            outputs: { exec_out: Exec, result: I64 },
        },
        {
            name: "multiply",
            friendly_name: "Multiply",
            description: "Multiplies two numbers",
            category: "Custom/Math",
            inputs: { exec: Exec, a: I64 = 0, b: I64 = 0 },
            outputs: { exec_out: Exec, result: I64 },
        }
    ]
}

// Each node needs a run function named run_{node_name}
#[no_mangle]
pub extern "C" fn run_add(ptr: i32, len: i32) -> i64 {
    run_with_handler(ptr, len, |mut ctx| {
        let a = ctx.get_i64("a").unwrap_or(0);
        let b = ctx.get_i64("b").unwrap_or(0);
        ctx.set_output("result", a + b);
        ctx.success()
    })
}

#[no_mangle]
pub extern "C" fn run_subtract(ptr: i32, len: i32) -> i64 {
    run_with_handler(ptr, len, |mut ctx| {
        let a = ctx.get_i64("a").unwrap_or(0);
        let b = ctx.get_i64("b").unwrap_or(0);
        ctx.set_output("result", a - b);
        ctx.success()
    })
}

#[no_mangle]
pub extern "C" fn run_multiply(ptr: i32, len: i32) -> i64 {
    run_with_handler(ptr, len, |mut ctx| {
        let a = ctx.get_i64("a").unwrap_or(0);
        let b = ctx.get_i64("b").unwrap_or(0);
        ctx.set_output("result", a * b);
        ctx.success()
    })
}
```

## Package Manifest

Create `manifest.toml` alongside your code:

```toml title="manifest.toml"
manifest_version = 1
id = "com.example.math-utils"
name = "Math Utilities"
version = "1.0.0"
description = "Common math operations"

[[authors]]
name = "Your Name"

[permissions]
memory = "minimal"
timeout = "quick"

[[nodes]]
id = "add"
name = "Add"
description = "Adds two numbers"
category = "Custom/Math"

[[nodes]]
id = "subtract"
name = "Subtract"
description = "Subtracts two numbers"
category = "Custom/Math"

[[nodes]]
id = "multiply"
name = "Multiply"
description = "Multiplies two numbers"
category = "Custom/Math"
```

## SDK API Reference

### Context Methods

```rust
// Get input values
ctx.get_string("pin_name") -> Option<String>
ctx.get_i64("pin_name") -> Option<i64>
ctx.get_f64("pin_name") -> Option<f64>
ctx.get_bool("pin_name") -> Option<bool>
ctx.get_json("pin_name") -> Option<serde_json::Value>
ctx.get_bytes("pin_name") -> Option<Vec<u8>>

// Set output values
ctx.set_output("pin_name", value)

// Execution control
ctx.success() -> ExecutionResult
ctx.error(message) -> ExecutionResult
```

### Logging

```rust
use flow_like_wasm_sdk::log;

log::debug("Debug message");
log::info("Info message");
log::warn("Warning message");
log::error("Error message");
```

### Variables

```rust
use flow_like_wasm_sdk::var;

// Get/set execution variables
let value = var::get_variable("my_var");
var::set_variable("my_var", json!({"key": "value"}));
```

### Streaming Output

```rust
use flow_like_wasm_sdk::stream;

// Stream progress updates
stream::stream_progress(50, "Processing...");

// Stream text
stream::stream_text("Partial output...");

// Stream JSON
stream::stream_json(json!({"status": "working"}));
```

## Pin Types

| Type | Rust Type | Default |
|------|-----------|---------|
| `Exec` | `()` | - |
| `String` | `String` | `""` |
| `I64` | `i64` | `0` |
| `F64` | `f64` | `0.0` |
| `Bool` | `bool` | `false` |
| `Json` | `serde_json::Value` | `null` |
| `Bytes` | `Vec<u8>` | `[]` |

## Build

```bash
cargo build --release --target wasm32-wasip1
```

Output: `target/wasm32-wasip1/release/my_wasm_nodes.wasm`

## Optimize (Optional)

Install `wasm-opt` for smaller binaries:

```bash
# macOS
brew install binaryen

# Linux
apt install binaryen

# Optimize (can reduce size by 20-40%)
wasm-opt -Os -o optimized.wasm target/wasm32-wasip1/release/my_wasm_nodes.wasm
```

## Install & Test

### Local Testing

```bash
# Copy to Flow-Like nodes directory
cp target/wasm32-wasip1/release/my_wasm_nodes.wasm ~/.flow-like/nodes/
cp manifest.toml ~/.flow-like/nodes/my_wasm_nodes.toml
```

### Publishing

```bash
# Publish to registry (requires API key)
flow-like publish ./target/wasm32-wasip1/release/my_wasm_nodes.wasm
```

## Advanced: HTTP Requests

For nodes that need network access, declare it in the manifest:

```toml
[permissions.network]
http_enabled = true
allowed_hosts = ["api.example.com"]
```

Then use the host functions:

```rust
use flow_like_wasm_sdk::http;

fn handle_run(mut ctx: Context) -> ExecutionResult {
    let response = http::get("https://api.example.com/data")?;
    ctx.set_output("result", response);
    ctx.success()
}
```

## Advanced: OAuth Access

For nodes requiring OAuth:

```toml
[[permissions.oauth_scopes]]
provider = "google"
scopes = ["https://www.googleapis.com/auth/drive.readonly"]
reason = "Access Google Drive files"
required = true

[[nodes]]
id = "list_files"
oauth_providers = ["google"]
```

```rust
use flow_like_wasm_sdk::auth;

fn handle_run(mut ctx: Context) -> ExecutionResult {
    let token = auth::get_oauth_access_token("google")?;
    // Use token for API calls...
    ctx.success()
}
```

## Related

→ [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
→ [WASM Nodes Overview](/dev/wasm-nodes/overview/)
→ [Writing Native Rust Nodes](/dev/writing-nodes/)
