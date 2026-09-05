# Flow-Like WASM Node Template (Rust)

Template for creating custom WASM nodes using the `flow-like-wasm-sdk` crate
and the Component Model (`wasm32-wasip2`).

## Prerequisites

- Rust toolchain (1.82+)
- WASM target: `rustup target add wasm32-wasip2`
- Or use mise: `mise run setup`

## Quick Start

```bash
cargo build --release            # outputs a WASM component
# Binary at: target/wasm32-wasip2/release/flow_like_wasm_node_template.wasm
```

## Creating Nodes

Use `#[register_node]` + `impl WasmNode` — mirrors the native catalog pattern:

```rust
use flow_like_wasm_sdk::*;

#[register_node]
#[derive(Default)]
pub struct MyNode;

impl WasmNode for MyNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "my_node", "My Node", "What it does", "Custom/Category",
        );
        node.add_pin(PinDefinition::input("exec", "Exec", "Trigger", "Exec"));
        node.add_pin(
            PinDefinition::input("text", "Text", "Input text", "String")
                .with_default(json!("")),
        );
        node.add_pin(PinDefinition::output("exec_out", "Done", "Done", "Exec"));
        node.add_pin(PinDefinition::output("result", "Result", "Output", "String"));
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

Add as many `#[register_node]` structs as you like — they're auto-discovered
at startup via the `inventory` crate.

## Share a custom object between nodes

The [package object example](src/package_objects.rs) stores a `TextBuffer` in
Wasm memory. It contains a vector of text chunks and a byte count, with no
serialization implementation. Nodes exchange a string handle to that object.
Build with the SDK in this checkout and a matching runtime; the template's
local SDK dependency selects that SDK automatically.

Wire the execution pins in this order:

```text
Create Text Buffer -> Append Text Buffer -> Read Text Buffer -> Close Text Buffer
```

Connect Create Text Buffer's `handle` output to the `handle` input on each of
the other nodes. Set `initial_text` to `Hello` and Append Text Buffer's `text`
to ` world`. Read Text Buffer returns `Hello world` and `byte_len` equal to
`11`. Close Text Buffer drops the object; using its handle afterward fails.
Execution connections matter because appending and closing change the object.

Replace `TextBuffer` with your own parser, buffer, or client type. The SDK's
`resources::insert` takes ownership and returns the handle. `resources::with`
and `resources::with_mut` check the requested Rust type before passing the object
to a closure. `resources::remove` returns ownership for explicit shutdown;
`resources::close` removes and drops it. Objects need only be `'static`, meaning
they cannot borrow temporary data from a node call. They do not need `Serialize`,
`Send`, or `Sync`. The [Rust SDK reference](../../libs/wasm-sdk/wasm-sdk-rust/README.md#store-arbitrary-objects-within-a-run)
includes a short API example and lifecycle limits.

## Share a WebSocket between nodes

The included [WebSocket example](src/websocket_server.rs) starts a listener in
one node, accepts a client in another, and sends a message in a third. Build it
with the SDK in this checkout and a runtime that provides the WebSocket listener
API. The template's local SDK path selects that version automatically.

On Flow-Like Desktop, wire the execution pins in this order:

```text
Start WebSocket Server -> Accept WebSocket Connection -> Send WebSocket Text -> Close WebSocket
```

Connect Start Server's `listener` output to Accept Connection's `listener`
input, and Accept Connection's `connection` output to Send Text's `connection`
input. Connect Start Server's `listener` output to Close WebSocket's `handle`
input to close the listener and its accepted clients after sending.

Keep the default bind address `127.0.0.1:8080`, start the flow, and connect a
WebSocket client to `ws://127.0.0.1:8080` while Accept Connection waits. The
client receives `Hello from Flow-Like`. If that port is occupied, choose another
port in both places. Setting the port to `0` chooses an available port; read the
`address` output before connecting. Hosted executors apply their own bind-address
policy and can reject the desktop loopback address.

Every example node declares `NodePermission::NetworkWebsocket`. The handles
refer to host-owned sockets scoped to this package and this run. The host keeps
the listener active after Start Server returns. A new run cannot reuse these
handles, even if it loads the same package or a saved handle value. Closing the
listener closes its clients; run completion or cancellation also releases the
resources. A flow that should keep listening must keep its run active.

## State between calls

Within a run, the runtime reuses the package's export-based Wasm instance when
its security configuration permits reuse. Guest globals and heap objects can
therefore survive a node return. Nodes in the same package can access those
objects through the SDK's `resources` registry, while each invocation receives fresh
inputs, outputs, logs and permissions. Calls to one instance execute in order.
The Rust registration macro constructs the node type for each call, so fields
on the node struct alone do not provide persistent state.
Different security domains have separate instances and resource registries;
their nodes cannot exchange live handles even within the same package and run.

All live state ends with the run. The next run starts with fresh guest memory,
host cache and sockets. Do not store a pointer, socket handle, or client object
in durable storage expecting to reuse it in another run. For the distinction
between reusable exports and command-style components, see the
[runtime lifecycle notes](../wasm-capability-matrix.md#state-and-resource-lifetime).

Run teardown reclaims guest memory and closes host and WASI resources. It does
not execute Rust destructors for objects still in the registry. Use `close`
during a node call to run an object's destructor, or `remove` to take ownership
and perform a client's graceful protocol shutdown. A retained client must support
the Wasm target and have the required network permissions. Keeping it in memory
does not drive its guest event loop between calls.

## Pin Types

| Type | Description |
|------|-------------|
| `Exec` | Flow control |
| `String` | Text value |
| `I64` | 64-bit integer |
| `F64` | 64-bit float |
| `Bool` | Boolean |
| `Generic` | JSON/Object |
| `Bytes` | Binary data |

## Context API

### Inputs

```rust
ctx.get_string("name")            // Option<String>
ctx.get_i64("name")               // Option<i64>
ctx.get_f64("name")               // Option<f64>
ctx.get_bool("name")              // Option<bool>
ctx.get_input("name")             // Option<&Value>
ctx.get_input_as::<T>("name")     // Option<T>
ctx.require_input("name")         // Result<&Value, String>
ctx.require_input_as::<T>("name") // Result<T, String>
```

### Outputs

```rust
ctx.set_output("name", value);     // any Serialize
ctx.set_output_json("name", &val); // explicit JSON
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

### Execution control

```rust
ctx.activate_exec("exec_out");  // fire exec pin
ctx.success()                   // return success
ctx.fail("reason")              // return error
ctx.set_pending(true);          // mark long-running
ctx.finish()                    // return without auto-exec
```

## SDK Host Modules

```rust
use flow_like_wasm_sdk::{log, stream, var, util};

log::info("hello");
stream::stream_text("chunk");
var::set_variable("key", &json!("val"));
let ts = util::now();
let r  = util::random();
```

## Testing

Unit tests run on native (not WASM):

```bash
cargo test --target $(rustc -vV | grep host | awk '{print $2}')
# or: mise run test
```

The native object registry is a thread-local test stub. It lets the example's
inventory-dispatch test call several nodes on one thread, but does not model
Flow-Like run ownership or isolate contexts by their `run_id`.

To check the compiled component's object handoff and package/run isolation,
run these commands from the repository root:

```bash
cargo build --manifest-path templates/wasm-node-rust/Cargo.toml --target wasm32-wasip2
cargo test -p flow-like-wasm --test package_object_test -- --include-ignored
```

The integration test creates objects in separate live instances before checking
that saved handles cannot access them. It also checks mutation, explicit close,
and release of the instance when its run ends.

## Building for Production

Already configured in `Cargo.toml`:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

## Publishing

1. Build: `mise run build`
2. Navigate to **Library → Packages → Publish** in Flow-Like Desktop
3. Select the `.wasm` file and the `flow-like.toml` manifest
4. Submit for review
