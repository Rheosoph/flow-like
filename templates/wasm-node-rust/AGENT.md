# Flow-Like WASM Node Development — Rust

## Project Overview

This is a **Flow-Like WASM node package** built with Rust targeting `wasm32-wasip2` (WASM Component Model). It produces `.wasm` components that run sandboxed inside the Flow-Like runtime. Each package can contain **multiple nodes**.

## Build & Test

```bash
# Build (outputs to target/wasm32-wasip2/release/)
cargo build --release

# Test (runs on native host, not WASM)
cargo test --target $(rustc -vV | grep host | awk '{print $2}')

# Or via mise
mise run build
mise run test
```

The `.cargo/config.toml` sets the default target to `wasm32-wasip2` — a plain `cargo build` produces a WASM component.

## Architecture

### Entry Point Pattern

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

wasm_main!(); // Must appear exactly once — auto-discovers all #[register_node] structs
```

- `#[register_node]` — proc macro that registers the struct via `inventory`
- `wasm_main!()` — generates the WASM Component Model exports (`get_node`, `get_nodes`, `run`, `get_abi_version`)
- Multiple `#[register_node]` structs per file/crate are supported

### Node Lifecycle

1. **`get_node()`** — called once to register metadata (pins, scores, descriptions). Never does I/O.
2. **`run(ctx)`** — called per execution. Read inputs, do work, set outputs, activate exec pins.

## Pin System

### Pin Types (DataType constants)

| Constant | Use For |
|---|---|
| `DataType::EXEC` / `"Exec"` | Flow control — connect execution order |
| `DataType::STRING` / `"String"` | Text values |
| `DataType::I64` / `"I64"` | 64-bit integers |
| `DataType::F64` / `"F64"` | 64-bit floats |
| `DataType::BOOL` / `"Bool"` | Booleans |
| `DataType::STRUCT` / `"Struct"` | Typed structs with JSON Schema |
| `DataType::PATH_BUF` / `"PathBuf"` | File paths (use `FlowPath` type) |
| `DataType::BYTES` / `"Bytes"` | Raw binary data |
| `DataType::DATE` / `"Date"` | Date/time values |
| `DataType::GENERIC` / `"Generic"` | Untyped JSON — **avoid, prefer Struct** |

### Value Types (collection wrappers)

| Constant | Meaning |
|---|---|
| `ValueType::NORMAL` | Single scalar value (default) |
| `ValueType::ARRAY` | `Vec<T>` |
| `ValueType::HASH_MAP` | `HashMap<String, T>` |
| `ValueType::HASH_SET` | `HashSet<T>` |

### CRITICAL: Prefer Struct over Generic

**Do NOT use `DataType::GENERIC` for structured data.** Always define a Rust struct with `#[derive(Serialize, Deserialize, JsonSchema)]` and use `DataType::STRUCT` with `.with_schema_type::<T>()`. This gives users schema validation, auto-complete in the UI, and type safety.

```rust
// WRONG — untyped, no schema, bad UX
PinDefinition::input("config", "Config", "Configuration", DataType::GENERIC)

// CORRECT — typed, schema-validated, good UX
#[derive(Serialize, Deserialize, JsonSchema)]
struct Config {
    threshold: f64,
    label: String,
}

PinDefinition::input("config", "Config", "Configuration", DataType::STRUCT)
    .with_schema_type::<Config>()
    .with_default(json!({"threshold": 0.5, "label": "default"}))
```

### Pin Definition API

```rust
// Input pin
PinDefinition::input("name", "Display Name", "Description", DataType::STRING)
    .with_default(json!(""))           // default value
    .with_valid_values(vec![...])      // enum dropdown in UI
    .with_range(0.0, 100.0)           // numeric slider
    .with_step(0.1)                    // slider step
    .with_schema_type::<MyStruct>()    // JSON Schema from Rust type
    .with_value_type(ValueType::ARRAY) // makes it Vec<T>
    .with_sensitive(true)              // masks value in UI (passwords, tokens)
    .with_enforce_schema(true)         // reject invalid input

// Output pin
PinDefinition::output("result", "Result", "Description", DataType::STRING)

// Exec pins (flow control) — impure nodes need these
PinDefinition::input("exec", "Exec", "Trigger", DataType::EXEC)
PinDefinition::output("exec_out", "Done", "Continue", DataType::EXEC)
```

### Pure vs Impure Nodes

- **Pure nodes**: no exec pins, no side effects, deterministic (e.g., string manipulation, math)
- **Impure nodes**: have exec pins, may do I/O or have side effects (e.g., HTTP calls, file writes)

For impure nodes, always add `exec` input and `exec_out` output pins. Add `exec_error` output only when leaving the workflow system (API calls, DB, etc.) — the runtime handles errors via `ctx.fail()` in most cases.

## Built-in Types — Use These

The SDK re-exports typed handles for interacting with the host runtime. **Always use these over raw strings or generic JSON.**

### FlowPath — File System Access

`FlowPath` is a handle to a file in the runtime's object store. **Never use raw `String` paths for file I/O.** The runtime resolves `FlowPath` to the actual storage backend (local, S3, Azure, etc.).

```rust
use flow_like_wasm_sdk::FlowPath;

// Pin definition — use DataType::PATH_BUF
PinDefinition::input("file", "File", "Input file", DataType::PATH_BUF)

// In run():
let file: FlowPath = ctx.require_input_as("file")?;
let bytes = file.read(&ctx);              // read bytes
file.write(&ctx, b"hello");              // write bytes
let children = file.list(&ctx);          // list directory

// Get platform directories
let storage = ctx.storage_dir(true);     // node-scoped storage
let uploads = ctx.upload_dir();          // user uploads
let cache = ctx.cache_dir(true, false);  // cache directory
```

### NodeImage — Image Handles

```rust
use flow_like_wasm_sdk::NodeImage;

// Create from bytes
let img = NodeImage::from_bytes(&ctx, &png_bytes, "png");
// Convert back
let bytes = img.to_bytes(&ctx, "png");
```

### Bit — LLM/Model References

```rust
use flow_like_wasm_sdk::{Bit, ChatMessage};

// Receive a model reference from upstream node
let model: Bit = ctx.require_input_as("model")?;
let response = model.prompt(&ctx, &[
    ChatMessage::system("You are helpful."),
    ChatMessage::user("Hello!"),
]);
```

### NodeDBConnection — Vector Database

```rust
use flow_like_wasm_sdk::{NodeDBConnection, VectorSearchQuery};

let db: NodeDBConnection = ctx.require_input_as("db")?;
let results = db.vector_search(&ctx, &VectorSearchQuery {
    vector: embedding,
    limit: 10,
    ..Default::default()
});
```

## Context API Reference

### Reading Inputs

```rust
ctx.get_string("name")                // Option<String>
ctx.get_i64("name")                   // Option<i64>
ctx.get_f64("name")                   // Option<f64>
ctx.get_bool("name")                  // Option<bool>
ctx.get_input("name")                 // Option<&Value>
ctx.get_input_as::<T>("name")         // Option<T> — deserialize from JSON
ctx.require_input("name")             // Result<&Value, String>
ctx.require_input_as::<T>("name")     // Result<T, String>
```

### Writing Outputs

```rust
ctx.set_output("name", value)          // any impl Into<Value>
ctx.set_output_json("name", &struct)   // serialize Rust struct
```

### Execution Control

```rust
ctx.activate_exec("exec_out")   // fire an exec output pin
ctx.success()                   // finalize, auto-activates "exec_out"
ctx.fail("reason")              // finalize with error
ctx.finish()                    // finalize without auto-exec
ctx.set_pending(true)           // mark as long-running
```

### Logging

```rust
ctx.debug("msg");
ctx.info("msg");
ctx.warn("msg");
ctx.error("msg");
```

### Streaming

```rust
ctx.stream_text("partial output");
ctx.stream_progress(0.5, "Halfway");
ctx.stream_json(&json!({ "status": "ok" }));
```

### HTTP (requires `network.http_enabled` permission)

```rust
use flow_like_wasm_sdk::http_ns;
let response = http_ns::http_request(0, "https://api.example.com/data", "{}", &[]);
```

### OAuth (requires `oauth_scopes` permission)

```rust
use flow_like_wasm_sdk::auth_ns;
if auth_ns::has_oauth_token("google") {
    let token = auth_ns::get_oauth_token("google");
}
```

### Variables & Cache

```rust
use flow_like_wasm_sdk::{var, cache_ns};
var::set_variable("key", &json!("value"));
let val = var::get_variable("key");

cache_ns::cache_set("key", &json!(42));
let cached = cache_ns::cache_get("key");
```

## Node Scores

Rate every node on these dimensions (0–10, 0 = bad, 10 = good):

```rust
node.set_scores(NodeScores {
    privacy: 10,       // Does it leak data externally?
    security: 8,       // Attack surface? Input validation?
    performance: 7,    // CPU/memory efficiency?
    governance: 9,     // Audit trail? Compliance?
    reliability: 8,    // Error handling? Determinism?
    cost: 10,          // External API costs?
});
```

## Permissions & Security (flow-like.toml)

The `flow-like.toml` manifest declares the package's security sandbox. The runtime **enforces** these — a node cannot access capabilities not declared here.

```toml
[permissions]
memory = "standard"        # Memory tier: minimal|light|standard|heavy|intensive|large|huge|extreme|maximum
timeout = "standard"       # Timeout tier: quick|standard|extended|long_running|very_long|maximum

[permissions.network]
http_enabled = false       # Outbound HTTP
allowed_hosts = []         # Empty = all hosts (if http_enabled)
websocket_enabled = false
tcp_enabled = false
udp_enabled = false
dns_enabled = false

[permissions.filesystem]
node_storage = false       # Node-scoped persistent storage
user_storage = false       # User-scoped persistent storage
```

**Principle of least privilege** — only request what you need. Packages requesting `http_enabled = true` with empty `allowed_hosts` get extra scrutiny during review.

## Common Patterns

### Impure Node with Error Handling

```rust
fn run(&self, mut ctx: Context) -> ExecutionResult {
    let url = ctx.get_string("url").unwrap_or_default();

    let response = match http_ns::http_request(0, &url, "{}", &[]) {
        Some(r) => r,
        None => return ctx.fail("HTTP request failed"),
    };

    ctx.set_output("response", response);
    ctx.success()
}
```

### Struct-Typed Pins with Schema

```rust
use schemars::JsonSchema;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct EmailConfig {
    to: String,
    subject: String,
    body: String,
}

// In get_node():
node.add_pin(
    PinDefinition::input("config", "Email Config", "Email parameters", DataType::STRUCT)
        .with_schema_type::<EmailConfig>()
);

// In run():
let config: EmailConfig = ctx.require_input_as("config")?;
```

### Enum Dropdown via valid_values

```rust
PinDefinition::input("format", "Format", "Output format", DataType::STRING)
    .with_default(json!("json"))
    .with_valid_values(vec!["json".into(), "csv".into(), "xml".into()])
```

### Multiple Pins with Same Name

Pins with the same `name` allow the user to add more instances of that pin in the UI:

```rust
node.add_pin(PinDefinition::input("item", "Item", "Input item", DataType::STRING));
node.add_pin(PinDefinition::input("item", "Item", "Input item", DataType::STRING));
// User can add more "item" pins in the editor
```

## Testing

Tests run on the native host (not WASM). The SDK provides mock stubs for all host functions during `#[cfg(test)]`.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_definition_is_valid() {
        let node = MyNode.get_node();
        assert_eq!(node.name, "my_node");
        assert!(!node.pins.is_empty());
    }
}
```

## Key Conventions

1. **Always provide descriptions** — node description, pin descriptions. Users see these in the visual editor.
2. **Set default values** — every non-exec input pin should have a sensible default via `.with_default(json!(...))`.
3. **Use `DataType::STRUCT` + `with_schema_type`** — never `DataType::GENERIC` for structured data.
4. **Use `FlowPath`** for file I/O — never raw `String` paths.
5. **Use `NodeImage`** for images — never raw bytes in Generic pins.
6. **Use `Bit`/`CachedEmbeddingModel`** for AI models — never pass model configs as JSON blobs.
7. **Log meaningfully** — `ctx.debug()` for tracing, `ctx.warn()` / `ctx.error()` for issues.
8. **Rate your node** — always call `node.set_scores(...)` with honest ratings.
9. **Declare minimal permissions** — only request what the node actually needs in `flow-like.toml`.
10. **Version the manifest** — bump `version` in `flow-like.toml` when changing pin interfaces.

## File Structure

```
├── .cargo/config.toml     # Sets wasm32-wasip2 as default target
├── .github/workflows/     # CI: build + release
├── Cargo.toml             # Dependencies (flow-like-wasm-sdk)
├── flow-like.toml         # Package manifest (permissions, metadata)
├── mise.toml              # Task runner config
├── src/
│   └── lib.rs             # Node implementations
└── AGENT.md               # This file
```
