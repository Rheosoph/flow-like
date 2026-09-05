# Flow-Like WASM Node Template (Rust)

Template for creating custom WASM nodes using the `flow-like-wasm-sdk` crate
and the Component Model (`wasm32-wasip2`).

## Prerequisites

- Rust 1.97.1, matching the version in `mise.toml`
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
These examples use SDK 0.4.0 and a runtime that supports run-owned package
instances and the `metadata.new-resource-handle` import. Until 0.4.0 is
published, the template's local dependency requires this repository checkout.

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
`Send`, or `Sync`. The [Rust SDK reference](https://github.com/Rheosoph/flow-like/blob/dev/libs/wasm-sdk/wasm-sdk-rust/README.md#store-arbitrary-objects-within-a-run)
includes a short API example and lifecycle limits.

## Consume an iterator across nodes

The [cursor example](src/resource_cursor.rs) keeps a `std::vec::IntoIter<String>` in
the same registry. The iterator owns its strings and remembers its position
between calls without serializing that position.

```text
Create Item Cursor -> Next Cursor Item -> Finish Item Cursor
```

Set Create Item Cursor's `items` input to `["first", "second", "third"]`, then
connect its `cursor` output to both later nodes. Next Cursor Item returns
`has_item = true`, `item = "first"`, and `remaining = 2`. Finish Item Cursor
returns `remaining_items = ["second", "third"]` and invalidates the handle.
It uses `resources::remove` to take ownership of the iterator before collecting
the unconsumed items.

Calling Next Cursor Item again advances the same iterator. Once it is exhausted,
`has_item` is false and `item` is empty. Check `has_item` because an empty string
can also be a valid item. Finish the cursor when done to release it during the
run, including when there are no items left.

## Share WASI TCP sockets between nodes

The [TCP example](src/resource_tcp.rs) stores `std::net::TcpListener` and a
custom connection object containing a `TcpStream` and a queue of unsent bytes.
They use the same `resources` API as the buffer and iterator. The Rust standard
library provides socket operations through WASI on the `wasm32-wasip2` target.

Each TCP node declares `NodePermission::NetworkTcp`. Nodes exchanging these
handles need compatible WASI network permissions and the same runtime security
domain so they use the same package instance. The example parses a numeric IP address and port
without DNS. Hostnames would also require DNS support and permission. Hosted
executors apply their bind-address policy and can reject loopback addresses.

On Flow-Like Desktop:

1. Run Start TCP Listener with `bind_address` set to `127.0.0.1:8080`.
   Its `listener` handle remains valid after the node returns. Port `0` selects
   an available port and returns it in the `address` output.
2. Pass `listener` to Accept TCP Connection. When `ready` is false, route
   execution through a delay and call Accept TCP Connection again. Keep the run
   active while waiting for a client.
3. Connect a TCP client, for example `nc 127.0.0.1 8080`. Once `ready` is true,
   pass the `connection` output to Queue TCP Text and run it once with the text
   to send. The default includes a newline for terminal clients.
4. If `drained` is false, pass the same connection to Poll TCP Send. Repeat
   through a delay until `drained` is true. Poll resumes the queued write;
   calling Queue TCP Text again would append another copy of its text.
5. Pass `connection` to Close TCP Connection and `listener` to Close TCP
   Listener when finished.

Accept checks once and returns immediately when no client is ready. Each send
or poll attempts one nonblocking write of at most 64 KiB. Partial writes and
`WouldBlock` leave the remaining bytes in the connection object for the next
call. The queue accepts at most 1 MiB of pending bytes. `drained` means those
bytes have been handed to the socket; it does not acknowledge receipt by the
peer. Close the connection after a socket error.

These sockets send plain TCP bytes. A package can build its own framing or use
a protocol library that supports WASI, then retain that library's client object
in `resources`. The package must execute to progress guest protocol code; storing
an object does not run a background event loop.

Closing a listener leaves its accepted connections open. Closing a connection
discards its pending queue, so poll it to completion first when the bytes matter.
Run completion, failure, or cancellation closes the underlying WASI sockets and
releases guest memory, even if explicit close nodes did not run. Saved handles
cannot reopen them in a later run.

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
[runtime lifecycle notes](https://github.com/Rheosoph/flow-like/blob/dev/templates/wasm-capability-matrix.md#state-and-resource-lifetime).

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

The integration tests exercise buffer and iterator handoff, socket operations,
package/run isolation, and release when the run ends. TCP tests open local
listeners, so the test environment must permit loopback sockets.

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
