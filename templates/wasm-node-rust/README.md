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