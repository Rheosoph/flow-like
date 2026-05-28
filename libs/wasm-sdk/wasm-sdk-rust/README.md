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
