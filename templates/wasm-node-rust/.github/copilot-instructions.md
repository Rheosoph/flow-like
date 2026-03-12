# Flow-Like WASM Node Development — Rust

## Project Overview

This is a **Flow-Like WASM node package** built with Rust targeting `wasm32-wasip2` (WASM Component Model). It produces `.wasm` components that run sandboxed inside the Flow-Like runtime. Each package can contain **multiple nodes**.

## Build & Test

```bash
cargo build --release    # WASM component → target/wasm32-wasip2/release/
cargo test --target $(rustc -vV | grep host | awk '{print $2}')   # native tests
```

## Architecture

Every WASM node package follows this structure:

```rust
use flow_like_wasm_sdk::*;

#[register_node]
#[derive(Default)]
pub struct MyNode;

impl WasmNode for MyNode {
    fn get_node(&self) -> NodeDefinition { /* metadata */ }
    fn run(&self, mut ctx: Context) -> ExecutionResult { /* logic */ }
}

wasm_main!(); // exactly once — auto-discovers all #[register_node] structs
```

## Pin Types

| Constant | Use For |
|---|---|
| `DataType::EXEC` | Flow control |
| `DataType::STRING` | Text |
| `DataType::I64` | 64-bit integer |
| `DataType::F64` | 64-bit float |
| `DataType::BOOL` | Boolean |
| `DataType::STRUCT` | Typed structs with JSON Schema — **prefer this** |
| `DataType::PATH_BUF` | File paths via `FlowPath` |
| `DataType::BYTES` | Binary data |
| `DataType::DATE` | Date/time |
| `DataType::GENERIC` | Untyped JSON — **avoid, use Struct instead** |

### Value Types

`ValueType::NORMAL` (scalar), `ValueType::ARRAY`, `ValueType::HASH_MAP`, `ValueType::HASH_SET`

## CRITICAL Rules

### 1. Prefer Struct over Generic

**Never use `DataType::GENERIC` for structured data.** Define a struct with `#[derive(Serialize, Deserialize, JsonSchema)]` and use `DataType::STRUCT` + `.with_schema_type::<T>()`.

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct Config { threshold: f64, label: String }

PinDefinition::input("config", "Config", "Settings", DataType::STRUCT)
    .with_schema_type::<Config>()
```

### 2. Use FlowPath for Files

**Never use raw `String` for file paths.** Use `FlowPath` + `DataType::PATH_BUF`. The runtime resolves it to the actual storage backend.

```rust
let file: FlowPath = ctx.require_input_as("file")?;
let bytes = file.read(&ctx);
file.write(&ctx, b"data");
```

### 3. Use Typed Handles

- `FlowPath` for files
- `NodeImage` for images
- `Bit` for LLM/model references
- `CachedEmbeddingModel` for embedding models
- `NodeDBConnection` for vector databases

### 4. Pure vs Impure Nodes

- **Pure**: no exec pins, deterministic, no side effects
- **Impure**: has exec input/output pins, may do I/O

### 5. Set Default Values

Every non-exec input pin should have `.with_default(json!(...))`.

### 6. Rate Every Node

```rust
node.set_scores(NodeScores {
    privacy: 10, security: 8, performance: 7,
    governance: 9, reliability: 8, cost: 10,
});
```

### 7. Minimal Permissions in flow-like.toml

Only declare what the node needs. Principle of least privilege.

```toml
[permissions]
memory = "standard"
timeout = "standard"

[permissions.network]
http_enabled = false
allowed_hosts = []

[permissions.filesystem]
node_storage = false
user_storage = false
```

## Context API

```rust
// Read inputs
ctx.get_string("name")             // Option<String>
ctx.get_i64("name")                // Option<i64>
ctx.get_f64("name")                // Option<f64>
ctx.get_bool("name")               // Option<bool>
ctx.require_input_as::<T>("name")  // Result<T, String>

// Write outputs
ctx.set_output("name", value);
ctx.set_output_json("name", &value);

// Execution
ctx.activate_exec("exec_out");
ctx.success()      // finalize + auto-activate exec_out
ctx.fail("error")  // finalize with error

// Logging
ctx.debug("msg"); ctx.info("msg"); ctx.warn("msg"); ctx.error("msg");

// Streaming
ctx.stream_text("chunk"); ctx.stream_progress(0.5, "msg");
```

## Pin Definition Helpers

```rust
.with_default(json!(""))              // default value
.with_valid_values(vec![...])         // enum dropdown
.with_range(0.0, 100.0)              // numeric slider
.with_step(0.1)                       // slider step
.with_schema_type::<T>()              // JSON Schema from type
.with_value_type(ValueType::ARRAY)    // Vec<T>
.with_sensitive(true)                  // masked in UI
.with_enforce_schema(true)             // reject invalid input
```

## Testing

Tests run on native host. SDK provides mock stubs during `#[cfg(test)]`.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_is_valid() {
        let node = MyNode.get_node();
        assert!(!node.pins.is_empty());
    }
}
```
