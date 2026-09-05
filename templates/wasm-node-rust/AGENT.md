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

The runtime retains an export-based package instance within one run when its
security configuration permits reuse. Guest globals and heap objects survive
calls to that instance; the registration macro still constructs the node type
for each call. Use the SDK's `resources` registry for guest-owned clients. Each call
receives fresh inputs, outputs, logs and permissions. Changing the security
domain selects a separate instance and resource registry. Guest state and host
resource handles cannot cross that boundary.

Guest memory, host cache and open sockets are scoped to the run. They are
discarded when it completes, fails or is cancelled and are never restored into
another run. The [package object example](src/package_objects.rs) uses
`resources::insert`, `with`, `with_mut` and `close` to share a custom `TextBuffer`
through string handle pins. Objects need `'static` ownership and can omit
`Serialize`, `Send` and `Sync`. Type mismatches and unavailable handles return
`ResourceError`; convert it to a node error with `ctx.fail(error.to_string())`.
Connect execution pins to establish the order of stateful operations.

Use `resources::remove::<T>` to take ownership for explicit shutdown, or
`resources::close::<T>` to remove an object and run its destructor during the
node call. Run teardown reclaims memory and host resources but does not execute
guest Rust destructors for retained objects. Retaining a client does not drive
its guest event loop between calls or grant additional networking permissions.
The native thread-local registry is a test stub; a different `run_id` in a
native `Context` does not create a new registry. Guest object preservation
requires reusable exports and is unavailable across `wasi:cli/run` commands.

The [cursor example](src/resource_cursor.rs) retains an owned iterator and uses
`resources::remove` to consume its remaining items. The
[TCP example](src/resource_tcp.rs) stores a standard-library `TcpListener` and
a custom object containing a `TcpStream` plus pending bytes. Every TCP node
declares `NodePermission::NetworkTcp` so operations share one permission domain.
Numeric bind addresses avoid a DNS dependency. Accept, send, and poll are
nonblocking; route retries through a delay and check `ready` or `drained` before
continuing. Repeating Queue TCP Text appends more bytes, while Poll TCP Send
resumes existing queued bytes. Keep accepted connections and listener cleanup
separate. New resource types belong in package code and use the existing
registry rather than adding host imports for each type.

## Pin System

The WASM SDK API mirrors the native catalog API. Pin types use proper Rust enums, schemas are derived from typed structs.

### VariableType (data types)

| Variant | Use For |
|---|---|
| `VariableType::Execution` | Flow control — connect execution order |
| `VariableType::String` | Text values |
| `VariableType::Integer` | 64-bit integers |
| `VariableType::Float` | 64-bit floats |
| `VariableType::Boolean` | Booleans |
| `VariableType::Struct` | Typed structs with JSON Schema |
| `VariableType::PathBuf` | File paths (use `FlowPath` type) |
| `VariableType::Byte` | Raw binary data |
| `VariableType::Date` | Date/time values |
| `VariableType::Generic` | Untyped JSON — **avoid, prefer Struct** |

### ValueType (collection wrappers)

| Variant | Meaning |
|---|---|
| `ValueType::Normal` | Single scalar value (default) |
| `ValueType::Array` | `Vec<T>` |
| `ValueType::HashMap` | `HashMap<String, T>` |
| `ValueType::HashSet` | `HashSet<T>` |

### CRITICAL: Input and Output Pins Must Have Different Names

**When a value passes through a node (input → processed → output), the input pin and output pin MUST have different `name` values (first argument).** The friendly name (second argument) CAN be the same. Pin names are used by `ctx.get_*()` and `ctx.set_output()` to identify which pin to read/write — if an input and output share the same name, get/set operations will collide.

```rust
// WRONG — input and output share the name "log", get/set will conflict
node.add_input_pin("log", "Log", "Log message", VariableType::String);
node.add_output_pin("log", "Log", "Log message", VariableType::String);

// CORRECT — different names, friendly names can match
node.add_input_pin("input_log", "Log", "Log message input", VariableType::String);
node.add_output_pin("output_log", "Log", "Log message output", VariableType::String);
```

Common prefixing conventions: `input_` / `output_`, or use semantically distinct names like `source_text` / `result_text`.

### CRITICAL: Prefer Struct over Generic

**Do NOT use `VariableType::Generic` for structured data.** Always define a Rust struct with `#[derive(Serialize, Deserialize, JsonSchema)]` and use `VariableType::Struct` with `.set_schema::<T>()`. This gives users schema validation, auto-complete in the UI, and type safety.

**The Generic pin type should only be used as a last resort** when the data shape is truly unknown at design time. If you can define a struct for it, always do so.

```rust
// WRONG — untyped, no schema, bad UX
node.add_input_pin("config", "Config", "Configuration", VariableType::Generic);

// CORRECT — typed, schema-validated, good UX
#[derive(Serialize, Deserialize, JsonSchema)]
struct Config {
    threshold: f64,
    label: String,
}

node.add_input_pin("config", "Config", "Configuration", VariableType::Struct)
    .set_schema::<Config>()
    .set_default_value(json!({"threshold": 0.5, "label": "default"}));
```

### CRITICAL: Use `with_value_type` / `set_value_type` for Collections

When a pin represents an **array**, **set**, or **map** of elements, use `.set_value_type()` (or `.with_value_type()`) to declare the collection kind. The **schema should describe the single element type**, not the collection itself.

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct Item {
    name: String,
    score: f64,
}

// WRONG — schema describes Vec<Item>, loses per-element validation
node.add_input_pin("items", "Items", "List of items", VariableType::Generic);

// CORRECT — schema is for a single Item, ValueType::Array wraps it as a list
node.add_input_pin("items", "Items", "List of items", VariableType::Struct)
    .set_schema::<Item>()
    .set_value_type(ValueType::Array);

// HashMap<String, Item>
node.add_input_pin("item_map", "Item Map", "Map of items", VariableType::Struct)
    .set_schema::<Item>()
    .set_value_type(ValueType::HashMap);

// HashSet<Item>
node.add_input_pin("item_set", "Item Set", "Unique items", VariableType::Struct)
    .set_schema::<Item>()
    .set_value_type(ValueType::HashSet);

// Array of strings (no struct needed for primitives)
node.add_input_pin("tags", "Tags", "List of tags", VariableType::String)
    .set_value_type(ValueType::Array);

// Using the consuming builder pattern:
node.add_pin(
    PinDefinition::input("items", "Items", "List of items", VariableType::Struct)
        .with_schema_type::<Item>()
        .with_value_type(ValueType::Array)
);
```

**Key rule:** The schema (`set_schema::<T>()` / `with_schema_type::<T>()`) always describes **one element**. The `ValueType` declares how many of those elements the pin holds.

### Pin Definition API — Native-style Builder Pattern (preferred)

This mirrors the native catalog API. `add_input_pin` / `add_output_pin` return `&mut PinDefinition` for chained configuration:

```rust
// Input pin with builder pattern
node.add_input_pin("session", "Session", "Copilot session", VariableType::Struct)
    .set_schema::<CopilotSessionHandle>()
    .set_enforce_schema(true);

// Input with default value
node.add_input_pin("count", "Count", "Number of items", VariableType::Integer)
    .set_default_value(json!(10));

// Input with valid values (dropdown in UI)
node.add_input_pin("format", "Format", "Output format", VariableType::String)
    .set_default_value(json!("json"))
    .set_valid_values(vec!["json".into(), "csv".into(), "xml".into()]);

// Input with range (slider in UI)
node.add_input_pin("temperature", "Temp", "Temperature", VariableType::Float)
    .set_default_value(json!(0.7))
    .set_range(0.0, 2.0)
    .set_step(0.1);

// Array of structs
node.add_input_pin("items", "Items", "List of items", VariableType::Struct)
    .set_schema::<MyItem>()
    .set_value_type(ValueType::Array);

// Sensitive input (masked in UI)
node.add_input_pin("api_key", "API Key", "Service API key", VariableType::String)
    .set_sensitive(true);

// Output pin with schema
node.add_output_pin("result", "Result", "Processed result", VariableType::Struct)
    .set_schema::<OutputData>();

// Exec pins (flow control) — impure nodes need these
node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
```

### Available `set_*` methods on `&mut PinDefinition`

| Method | Description |
|---|---|
| `.set_default_value(json!(...))` | Default value for the pin |
| `.set_schema::<T>()` | JSON Schema from `#[derive(JsonSchema)]` struct |
| `.set_value_type(ValueType::Array)` | Make it a collection (Array, HashMap, HashSet) |
| `.set_valid_values(vec![...])` | Enum dropdown in UI |
| `.set_range(min, max)` | Numeric slider range |
| `.set_step(step)` | Slider increment |
| `.set_sensitive(true)` | Mask value in UI (passwords, tokens) |
| `.set_enforce_schema(true)` | Reject input that doesn't match schema |
| `.set_enforce_generic_value_type(true)` | Enforce value type match at connection time |

### Alternative: Consuming Builder Pattern

You can also use `add_pin` with `PinDefinition::input` / `PinDefinition::output` and `with_*` methods:

```rust
node.add_pin(
    PinDefinition::input("text", "Text", "Input text", VariableType::String)
        .with_default(json!(""))
        .with_schema_type::<MyStruct>()
        .with_value_type(ValueType::Array)
);
```

### Pure vs Impure Nodes

- **Pure nodes**: no exec pins, no side effects, deterministic (e.g., string manipulation, math)
- **Impure nodes**: have exec pins, may do I/O or have side effects (e.g., HTTP calls, file writes)

For impure nodes, always add `exec` input and `exec_out` output pins.

## Built-in Types — Use These

The SDK re-exports typed handles for interacting with the host runtime. **Always use these over raw strings or generic JSON.**

### FlowPath — File System Access

`FlowPath` is a handle to a file in the runtime's object store. **Never use raw `String` paths for file I/O.** The runtime resolves `FlowPath` to the actual storage backend (local, S3, Azure, etc.).

```rust
use flow_like_wasm_sdk::FlowPath;

// Pin definition
node.add_input_pin("file", "File", "Input file", VariableType::PathBuf);

// In run():
let file: FlowPath = ctx.require_input_as("file")?;
let bytes = file.read(&ctx);
file.write(&ctx, b"hello");
let children = file.list(&ctx);

// Get platform directories
let storage = ctx.storage_dir(true);     // node-scoped storage
let uploads = ctx.upload_dir();          // user uploads
let cache = ctx.cache_dir(true, false);  // cache directory
```

### NodeImage — Image Handles

```rust
use flow_like_wasm_sdk::NodeImage;

// Pin definition
node.add_input_pin("image", "Image", "Input image", VariableType::Struct)
    .set_schema::<NodeImage>();

// Create from bytes
let img = NodeImage::from_bytes(&ctx, &png_bytes, "png");
let bytes = img.to_bytes(&ctx, "png");
```

### Bit — LLM/Model References

```rust
use flow_like_wasm_sdk::{Bit, ChatMessage};

// Pin definition
node.add_input_pin("model", "Model", "LLM model reference", VariableType::Struct)
    .set_schema::<Bit>();

// In run():
let model: Bit = ctx.require_input_as("model")?;
let response = model.prompt(&ctx, &[
    ChatMessage::system("You are helpful."),
    ChatMessage::user("Hello!"),
]);
```

### NodeDBConnection — Vector Database

```rust
use flow_like_wasm_sdk::{NodeDBConnection, VectorSearchQuery};

// Pin definition
node.add_input_pin("db", "Database", "Vector DB connection", VariableType::Struct)
    .set_schema::<NodeDBConnection>();

// In run():
let db: NodeDBConnection = ctx.require_input_as("db")?;
let results = db.vector_search(&ctx, &VectorSearchQuery {
    vector: embedding,
    limit: 10,
    ..Default::default()
});
```

### CachedEmbeddingModel — Embedding Models

```rust
use flow_like_wasm_sdk::CachedEmbeddingModel;

// Pin definition
node.add_input_pin("embed_model", "Embedding Model", "Model for embeddings", VariableType::Struct)
    .set_schema::<CachedEmbeddingModel>();

// In run():
let model: CachedEmbeddingModel = ctx.require_input_as("embed_model")?;
let embeddings = ctx.embed_text_query(&model, &["hello".to_string()]);
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

### HTTP (requires `NodePermission::NetworkHttp`)

```rust
use flow_like_wasm_sdk::http_ns;
let response = http_ns::http_request(0, "https://api.example.com/data", "{}", &[]);
```

### OAuth (requires `NodePermission::OAuth`)

```rust
use flow_like_wasm_sdk::auth_ns;
if auth_ns::has_oauth_token("google") {
    let token = auth_ns::get_oauth_token("google");
}
```

### Variables & Cache (require `NodePermission::Variables` / `NodePermission::Cache`)

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

## Permissions & Resource Limits

`flow-like.toml` carries package metadata plus package-wide resource tiers such as memory and timeout. Capability permissions are declared per node in Rust with `node.add_permission(NodePermission::...)`, and those node-level permissions are what the runtime exposes and enforces for WASM nodes.

```toml
[permissions]
memory = "standard"        # Package memory tier: minimal|light|standard|heavy|intensive|large|huge|extreme|maximum
timeout = "standard"       # Package timeout tier: quick|standard|extended|long_running|very_long|maximum
```

```rust
use flow_like_wasm_sdk::NodePermission;

fn get_node(&self) -> NodeDefinition {
    let mut node = NodeDefinition::new(
        "fetch_data",
        "Fetch Data",
        "Fetches data over HTTP and stores it for later use",
        "Custom/WASM",
    );
    node.add_permission(NodePermission::NetworkHttp);
    node.add_permission(NodePermission::StorageWrite);
    node
}
```

Common capability permissions:

- `NetworkHttp`, `NetworkWebsocket`, `NetworkTcp`, `NetworkUdp`, `NetworkDns`
- `StorageRead`, `StorageWrite`, `Variables`, `Cache`
- `Streaming`, `Models`, `A2ui`, `OAuth`, `Functions`

**Principle of least privilege** — keep both manifest resource tiers and per-node capabilities as small as practical.

## Common Patterns

### Impure Node with Error Handling

This pattern also needs `node.add_permission(NodePermission::NetworkHttp)` in `get_node()`.

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

### Full Node with Struct-Typed Pins

```rust
use schemars::JsonSchema;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct EmailConfig {
    to: String,
    subject: String,
    body: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SendResult {
    message_id: String,
    success: bool,
}

#[register_node]
#[derive(Default)]
pub struct SendEmailNode;

impl WasmNode for SendEmailNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "send_email",
            "Send Email",
            "Sends an email using the provided configuration",
            "Communication/Email",
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("config", "Email Config", "Email parameters", VariableType::Struct)
            .set_schema::<EmailConfig>()
            .set_enforce_schema(true);

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
        node.add_output_pin("result", "Result", "Send result", VariableType::Struct)
            .set_schema::<SendResult>();

        node.set_scores(NodeScores {
            privacy: 5,
            security: 7,
            performance: 8,
            governance: 8,
            reliability: 7,
            cost: 9,
        });

        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let config: EmailConfig = match ctx.require_input_as("config") {
            Ok(c) => c,
            Err(e) => return ctx.fail(e),
        };

        // ... send email logic ...

        ctx.set_output_json("result", &SendResult {
            message_id: "msg-123".into(),
            success: true,
        });
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}
```

### Enum Dropdown via valid_values

```rust
node.add_input_pin("format", "Format", "Output format", VariableType::String)
    .set_default_value(json!("json"))
    .set_valid_values(vec!["json".into(), "csv".into(), "xml".into()]);
```

### Multiple Pins with Same Name

Pins with the same `name` allow the user to add more instances of that pin in the UI:

```rust
node.add_input_pin("item", "Item", "Input item", VariableType::String);
node.add_input_pin("item", "Item", "Input item", VariableType::String);
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
2. **Set default values** — every non-exec input pin should have a sensible default via `.set_default_value(json!(...))`.
3. **Use `VariableType::Struct` + `set_schema::<T>()`** — never `VariableType::Generic` for structured data. If you can define a struct, always prefer it over Generic.
4. **Use `set_value_type()` / `with_value_type()` for collections** — for `Vec<T>` use `ValueType::Array`, for `HashMap<String, T>` use `ValueType::HashMap`, for `HashSet<T>` use `ValueType::HashSet`. The schema always describes the **single element**, not the collection.
5. **Use `FlowPath`** for file I/O — never raw `String` paths.
6. **Use `NodeImage`** for images — never raw bytes in Generic pins.
7. **Use `Bit`/`CachedEmbeddingModel`** for AI models — never pass model configs as JSON blobs.
8. **Log meaningfully** — `ctx.debug()` for tracing, `ctx.warn()` / `ctx.error()` for issues.
9. **Rate your node** — always call `node.set_scores(...)` with honest ratings.
10. **Declare minimal node permissions** — only request what the node actually needs via `node.add_permission(...)`, and keep manifest resource tiers tight.
11. **Version the manifest** — bump `version` in `flow-like.toml` when changing pin interfaces.

## File Structure

```
├── .cargo/config.toml     # Sets wasm32-wasip2 as default target
├── .github/workflows/     # CI: build + release
├── Cargo.toml             # Dependencies (flow-like-wasm-sdk)
├── flow-like.toml         # Package manifest (metadata, memory, timeout tiers)
├── mise.toml              # Task runner config
├── src/
│   ├── lib.rs             # Entry point and basic node examples
│   ├── package_objects.rs # Custom text buffer resources
│   ├── resource_cursor.rs # Owned iterator and remove() handoff
│   └── resource_tcp.rs    # WASI sockets and queued partial writes
└── AGENT.md               # This file
```
