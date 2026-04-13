# Flow-Like WASM Node Development — Rust

## Project Overview

This is a Flow-Like WASM node package built with Rust for `wasm32-wasip2`. It produces `.wasm` components that run sandboxed inside the Flow-Like runtime. A single package can expose multiple nodes.

## Build & Test

```bash
cargo build --release
cargo test --target $(rustc -vV | grep host | awk '{print $2}')
```

The template sets `wasm32-wasip2` as the default target in `.cargo/config.toml`, so plain `cargo build` already builds a component.

## Architecture

```rust
use flow_like_wasm_sdk::*;

#[register_node]
#[derive(Default)]
pub struct MyNode;

impl WasmNode for MyNode {
    fn get_node(&self) -> NodeDefinition { /* metadata */ }
    fn run(&self, mut ctx: Context) -> ExecutionResult { /* logic */ }
}

wasm_main!();
```

- `#[register_node]` registers a node for discovery.
- `wasm_main!()` must appear exactly once per crate.
- Put metadata only in `get_node()`. Runtime work belongs in `run()`.

## Pin Types

| Variant | Use For |
|---|---|
| `VariableType::Execution` | Flow control |
| `VariableType::String` | Text |
| `VariableType::Integer` | 64-bit integers |
| `VariableType::Float` | 64-bit floats |
| `VariableType::Boolean` | Booleans |
| `VariableType::Struct` | Typed structs with JSON Schema |
| `VariableType::PathBuf` | File paths via `FlowPath` |
| `VariableType::Byte` | Binary data |
| `VariableType::Date` | Date/time |
| `VariableType::Generic` | Untyped JSON, avoid when you can define a struct |

### Value Types

Use `ValueType::Normal`, `ValueType::Array`, `ValueType::HashMap`, and `ValueType::HashSet` to describe whether a pin holds one value or a collection.

## Critical Rules

### 1. Input and Output Pins Must Have Different Names

If a value passes through a node, the input pin name and output pin name must differ. Friendly labels can stay the same.

```rust
node.add_input_pin("input_log", "Log", "Log message input", VariableType::String);
node.add_output_pin("output_log", "Log", "Log message output", VariableType::String);
```

### 2. Prefer Struct over Generic

For structured data, define a Rust type and attach its schema.

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct Config {
    threshold: f64,
    label: String,
}

node.add_input_pin("config", "Config", "Settings", VariableType::Struct)
    .set_schema::<Config>()
    .set_enforce_schema(true);
```

### 3. Use `set_value_type()` for Collections

The schema describes one element. `ValueType` describes whether the pin holds one value, an array, a map, or a set.

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct Item {
    name: String,
    score: f64,
}

node.add_input_pin("items", "Items", "List of items", VariableType::Struct)
    .set_schema::<Item>()
    .set_value_type(ValueType::Array);
```

### 4. Use Typed Handles

- `FlowPath` for files
- `NodeImage` for images
- `Bit` for model references
- `CachedEmbeddingModel` for embedding models
- `NodeDBConnection` for vector databases

### 5. Pure vs Impure Nodes

- Pure nodes have no execution pins and no side effects.
- Impure nodes have execution pins and may perform I/O or other side effects.

### 6. Set Default Values

Give every non-execution input pin a sensible default where possible.

```rust
node.add_input_pin("count", "Count", "Number of items", VariableType::Integer)
    .set_default_value(json!(10));
```

### 7. Rate Every Node

```rust
node.set_scores(NodeScores {
    privacy: 10,
    security: 8,
    performance: 7,
    governance: 9,
    reliability: 8,
    cost: 10,
});
```

### 8. Declare Minimal Permissions in Rust

Capability permissions are declared per node with `node.add_permission(NodePermission::...)`. The manifest still owns package-wide memory and timeout tiers, but network/storage/model access is not configured in `flow-like.toml` anymore.

```toml
[permissions]
memory = "standard"
timeout = "standard"
```

```rust
node.add_permission(NodePermission::NetworkHttp);
node.add_permission(NodePermission::StorageWrite);
```

Common capability permissions include `NetworkHttp`, `NetworkWebsocket`, `NetworkTcp`, `NetworkUdp`, `NetworkDns`, `StorageRead`, `StorageWrite`, `Variables`, `Cache`, `Streaming`, `Models`, `A2ui`, `OAuth`, and `Functions`.

## Context API

```rust
ctx.get_string("name");
ctx.get_i64("name");
ctx.get_f64("name");
ctx.get_bool("name");
ctx.get_input_as::<T>("name");
ctx.require_input_as::<T>("name");

ctx.set_output("name", value);
ctx.set_output_json("name", &value);

ctx.activate_exec("exec_out");
ctx.success();
ctx.fail("error");

ctx.debug("msg");
ctx.info("msg");
ctx.warn("msg");
ctx.error("msg");

ctx.stream_text("chunk");
ctx.stream_progress(0.5, "msg");
```

## Pin Configuration Helpers

```rust
.set_default_value(json!(""))
.set_valid_values(vec![...])
.set_range(0.0, 100.0)
.set_step(0.1)
.set_schema::<T>()
.set_value_type(ValueType::Array)
.set_sensitive(true)
.set_enforce_schema(true)
.set_enforce_generic_value_type(true)
```

## Testing

Tests run natively. The SDK provides host stubs during `#[cfg(test)]`.

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
