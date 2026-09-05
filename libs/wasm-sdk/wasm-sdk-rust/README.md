# flow-like-wasm-sdk

Rust SDK for building [Flow-Like](https://github.com/Rheosoph/flow-like) WASM nodes
using the Component Model (`wasm32-wasip2`). A package can export multiple nodes
and retain objects between their calls within one run.

The examples below target **0.4.0**. See the [release notes](CHANGELOG.md) for
migration details and the [publishing guide](RELEASING.md) for the release
procedure.

## Setup

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
flow-like-wasm-sdk = "0.4.0"
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
The API mirrors the native catalog. Use `VariableType` enums and `set_schema::<T>()` for typed pins:

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

The `inventory` crate discovers each `#[register_node]` struct at startup.

## Struct-Typed Pins with Schema

Use `#[derive(JsonSchema)]` structs to define typed pins, as in the native catalog:

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
[registered Create, Append, Read and Close nodes](https://github.com/Rheosoph/flow-like/blob/dev/templates/wasm-node-rust/src/package_objects.rs)
for a complete flow example. The registry requires SDK 0.4.0 and a runtime that
supports package instances owned by a run and the `metadata.new-resource-handle`
import. Earlier runtimes cannot provide this API.

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

## Iterators and sockets use the same registry

A retained object keeps its own state between calls. For example, a node can
store an iterator, another node can advance it with `resources::with_mut`, and
a final node can call `resources::remove` to collect the remaining items. The
[cursor example](https://github.com/Rheosoph/flow-like/blob/dev/templates/wasm-node-rust/src/resource_cursor.rs)
demonstrates this with `object_create_cursor`, `object_next_item`, and
`object_finish_cursor`.

Networking objects follow the same ownership model. A package can store a
WASI TCP listener or connection in `resources`, then pass its handle to later
nodes. The template's
[TCP example](https://github.com/Rheosoph/flow-like/blob/dev/templates/wasm-node-rust/src/resource_tcp.rs)
uses `tcp_start_listener`, `tcp_accept_connection`, and `tcp_send_text` nodes
with guest networking code. They require `NodePermission::NetworkTcp` and
remain subject to the runtime's address policy. The listener and connection
remain separate objects; close each when it is no longer needed, or
let run teardown release both.

The TCP example accepts numeric IP addresses, such as `127.0.0.1:8080`, and does
not resolve hostnames. An accept operation may report that no connection is
ready. A send may leave pending bytes, which `tcp_poll_send` retries on a later
call. Check these outputs before advancing the flow. See the
[Rust template](https://github.com/Rheosoph/flow-like/tree/dev/templates/wasm-node-rust)
for the complete flows and network restrictions.

## WebSocket client connections

Declare `NodePermission::NetworkWebsocket` on each node that uses a socket.
The host owns connections, while the SDK passes opaque string handles between
nodes in the same package and run.
Nodes must also use the same security domain; a separate domain has its own
instance and resource registry.

| Context method | Result |
|---|---|
| `ws_connect(&url, &headers)` | Outbound connection handle |
| `ws_send_text(&connection, "hello")` | Whether text was sent |
| `ws_send(&connection, &bytes)` | Whether binary data was sent |
| `ws_receive(&connection, 1_000)` | Message JSON, or `None` when unavailable or timed out |
| `ws_close(&connection)` | Whether the connection was closed |

Network policy applies to each operation. All handles expire when the run ends
or is cancelled. Saving a handle does not preserve its socket for another run.

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
