# flow-like-wasm-sdk

Rust SDK for building [Flow-Like](https://github.com/Rheosoph/flow-like) WASM nodes
using the Component Model (`wasm32-wasip2`). Produces compact, zero-overhead binaries.

## Setup

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
flow-like-wasm-sdk = "0.2"
```

Add a `.cargo/config.toml`:

```toml
[build]
target = "wasm32-wasip2"
```

Install the target:

```bash
rustup target add wasm32-wasip2
```

## Quick Start

Define nodes with `#[register_node]` + `impl WasmNode`, then call `wasm_main!()`.
The API mirrors the native catalog — use `VariableType` enums and `set_schema::<T>()` for typed pins:

```rust
use flow_like_wasm_sdk::*;

#[register_node]
#[derive(Default)]
pub struct UppercaseNode;

impl WasmNode for UppercaseNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "uppercase", "Uppercase", "Converts text to uppercase", "Text/Transform",
        );
        node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
        node.add_input_pin("text", "Text", "Input text", VariableType::String)
            .set_default_value(json!(""));
        node.add_output_pin("exec_out", "Done", "Done", VariableType::Execution);
        node.add_output_pin("result", "Result", "Uppercased text", VariableType::String);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let text = ctx.get_string("text").unwrap_or_default();
        ctx.set_output("result", text.to_uppercase());
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

wasm_main!();
```

Add as many `#[register_node]` structs as you like — they're auto-discovered at
startup via the `inventory` crate. No manual routing needed.

## Struct-Typed Pins with Schema

Use `#[derive(JsonSchema)]` structs for type-safe pins — just like the native catalog:

```rust
use schemars::JsonSchema;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct Config {
    threshold: f64,
    label: String,
}

// In get_node():
node.add_input_pin("config", "Config", "Configuration", VariableType::Struct)
    .set_schema::<Config>()
    .set_enforce_schema(true)
    .set_default_value(json!({"threshold": 0.5, "label": "default"}));
```

## Multi-Node Package

```rust
use flow_like_wasm_sdk::*;

#[register_node]
#[derive(Default)]
pub struct AddNode;

impl WasmNode for AddNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new("add", "Add", "Adds two integers", "Math");
        node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
        node.add_input_pin("a", "A", "First operand", VariableType::Integer)
            .set_default_value(json!(0));
        node.add_input_pin("b", "B", "Second operand", VariableType::Integer)
            .set_default_value(json!(0));
        node.add_output_pin("exec_out", "Done", "Done", VariableType::Execution);
        node.add_output_pin("result", "Result", "Sum", VariableType::Integer);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let a = ctx.get_i64("a").unwrap_or(0);
        let b = ctx.get_i64("b").unwrap_or(0);
        ctx.set_output("result", a + b);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct SubtractNode;

impl WasmNode for SubtractNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new("subtract", "Subtract", "Subtracts B from A", "Math");
        node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
        node.add_input_pin("a", "A", "First operand", VariableType::Integer)
            .set_default_value(json!(0));
        node.add_input_pin("b", "B", "Second operand", VariableType::Integer)
            .set_default_value(json!(0));
        node.add_output_pin("exec_out", "Done", "Done", VariableType::Execution);
        node.add_output_pin("result", "Result", "Difference", VariableType::Integer);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let a = ctx.get_i64("a").unwrap_or(0);
        let b = ctx.get_i64("b").unwrap_or(0);
        ctx.set_output("result", a - b);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

wasm_main!();
```

## Context API

| Method | Returns | Description |
|--------|---------|-------------|
| `get_string(pin)` | `Option<String>` | Read string input |
| `get_i64(pin)` | `Option<i64>` | Read integer input |
| `get_f64(pin)` | `Option<f64>` | Read float input |
| `get_bool(pin)` | `Option<bool>` | Read boolean input |
| `get_input(pin)` | `Option<&Value>` | Read raw JSON |
| `get_input_as::<T>(pin)` | `Option<T>` | Deserialize input |
| `require_input(pin)` | `Result<&Value>` | Required raw JSON |
| `require_input_as::<T>(pin)` | `Result<T>` | Required + deser |
| `set_output(pin, val)` | | Write any `Serialize` value |
| `set_output_json(pin, &val)` | | Write explicit JSON |
| `activate_exec(pin)` | | Fire an exec output pin |
| `success()` | `ExecutionResult` | Return success |
| `fail(msg)` | `ExecutionResult` | Return error |
| `set_pending(bool)` | | Mark as long-running |
| `finish()` | `ExecutionResult` | Return without auto-exec |

## Host Modules

```rust
use flow_like_wasm_sdk::{log, stream, var, util};

log::info("message");
stream::stream_text("chunk");
var::set_variable("key", &json!("val"));
let ts = util::now();
let r  = util::random();
```

## Store arbitrary objects within a run

Use `resources` to keep a Rust object in package memory and pass its string
handle between nodes. The object can be a parser, buffer, or client, including
types without `Serialize`, `Send`, or `Sync`. It must be `'static`, so it owns
its data instead of borrowing temporary memory from a node call.

```rust
use flow_like_wasm_sdk::resources::{self, ResourceError};

struct TextBuffer {
    chunks: Vec<String>,
}

fn example() -> Result<(), ResourceError> {
    // The creating node outputs this handle through a String pin.
    let handle = resources::insert(TextBuffer { chunks: vec!["Hello".into()] })?;

    // Later nodes receive the same handle and access the original object.
    resources::with_mut::<TextBuffer, _>(&handle, |buffer| {
        buffer.chunks.push(" world".into());
    })?;
    let text = resources::with::<TextBuffer, _>(&handle, |buffer| buffer.chunks.concat())?;
    assert_eq!(text, "Hello world");

    // Removing invalidates the handle and returns ownership for explicit cleanup.
    let buffer = resources::remove::<TextBuffer>(&handle)?;
    drop(buffer);
    Ok(())
}
```

Call `resources::close::<T>(&handle)` when removing and dropping the object is
enough. Access checks the object's Rust type; a wrong type or unavailable handle
returns `ResourceError`. A closure keeps the borrow within that call, so return
owned data when writing an output pin. See the
[registered Create, Append, Read and Close nodes](../../../templates/wasm-node-rust/src/package_objects.rs)
for a complete flow example. Use this checkout's SDK and matching runtime for
the registry API.

Objects remain available in reusable export-based packages until removed or the
run ends. Node structs themselves are constructed again for each invocation;
store shared objects in `resources` instead of node fields. Different packages,
runs and security domains have separate registries. Saving a handle in durable
storage cannot restore the object in a later run. Command-style `wasi:cli/run`
execution starts with fresh guest memory and cannot preserve this registry
between commands.

Run teardown reclaims guest memory and closes host and WASI resources. It does
not execute Rust destructors for objects still retained in guest memory. Use
`close` during a node call to run `Drop`, or `remove` to take ownership and call
a client's graceful shutdown method. A TCP or UDP client can use this registry
if it supports the Wasm target and the package's network grants. Nodes sharing
raw WASI sockets need compatible WASI network permissions. The registry adds no
operating-system APIs or permissions and does not drive a guest event loop
between calls.

Native builds use a thread-local registry for SDK and template tests. It does
not model the runtime's package or run isolation; changing a test `Context`'s
`run_id` does not reset that thread's objects.

## WebSocket resources within a run

Declare `NodePermission::NetworkWebsocket` on each node that uses a socket.
The host owns listeners and connections, while the SDK passes opaque string
handles between nodes in the same package and run.
Nodes must also use the same security domain; a separate domain has its own
instance and resource registry.

| Context method | Result |
|---|---|
| `ws_listen("127.0.0.1:8080")` | Listener handle, or `None` if binding is denied or fails |
| `ws_local_address(&listener)` | Bound IP address and port; useful when listening on port 0 |
| `ws_accept(&listener, 10_000)` | Next accepted connection, or `None` when unavailable or timed out |
| `ws_connect(&url, &headers)` | Outbound connection handle |
| `ws_send_text(&connection, "hello")` | Whether text was sent |
| `ws_send(&connection, &bytes)` | Whether binary data was sent |
| `ws_receive(&connection, 1_000)` | Message JSON, or `None` when unavailable or timed out |
| `ws_close(&handle)` | Whether the connection, or listener and its clients, was closed |

These APIs require a runtime that implements the WebSocket listener interface.
Network policy still applies to each operation. The desktop loopback address
in the example is subject to different restrictions in hosted executors.

A listener remains active after its creating node returns. All handles expire
when the run ends or is cancelled. Saving a handle does not preserve its socket
for another run. Guest globals also persist only in a reusable export-based
instance within the current run and security domain. See the
[complete server example](../../../templates/wasm-node-rust/src/websocket_server.rs)
for Start Server, Accept Connection, Send Text and Close nodes.

## Testing

Unit tests run on native target:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_definition() {
        let node = UppercaseNode;
        let def = node.get_node();
        assert_eq!(def.name, "uppercase");
        assert_eq!(def.pins.len(), 4);
    }
}
```

```bash
cargo test --target $(rustc -vV | grep host | awk '{print $2}')
```

## Building

```bash
cargo build --release
# output: target/wasm32-wasip2/release/<crate_name>.wasm
```
