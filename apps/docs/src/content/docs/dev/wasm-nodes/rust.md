---
title: Rust WASM Nodes
description: Build Flow-Like WASM nodes in Rust with the Component Model SDK
sidebar:
  order: 1
  badge:
    text: Recommended
    variant: tip
---

Rust is the most complete Flow-Like WASM SDK and the recommended starting
point. The checked-in template targets the WASM Component Model with
`wasm32-wasip2`.

## Start from the template

Copy `templates/wasm-node-rust` into your own project, then run:

```bash
mise run setup
mise run test
mise run build
```

The build task installs the `wasm32-wasip2` target, compiles a release component,
and copies the result to `node.wasm`. The underlying Cargo artifact is:

```text
target/wasm32-wasip2/release/flow_like_wasm_node_template.wasm
```

If you do not use mise:

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

The template depends on the published `flow-like-wasm-sdk` crate and configures
the library as `cdylib`.

## Define a node

Rust packages use `#[register_node]`, `WasmNode`, and a single
`wasm_main!()` invocation:

```rust title="src/lib.rs"
use flow_like_wasm_sdk::*;

#[register_node]
#[derive(Default)]
pub struct UppercaseNode;

impl WasmNode for UppercaseNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "uppercase",
            "Uppercase",
            "Converts text to uppercase",
            "Custom/Text",
        );

        node.add_input_pin(
            "exec",
            "Exec",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "text",
            "Text",
            "Text to transform",
            VariableType::String,
        )
        .set_default_value(json!(""));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Continue execution",
            VariableType::Execution,
        );
        node.add_output_pin(
            "result",
            "Result",
            "Uppercase text",
            VariableType::String,
        );

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

Add more registered structs for a multi-node package. `wasm_main!()` generates
the Component Model exports and automatically exposes every registered node
through `get_nodes`.

## Context API

Common input helpers:

```rust
ctx.get_string("name");          // Option<String>
ctx.get_i64("name");             // Option<i64>
ctx.get_f64("name");             // Option<f64>
ctx.get_bool("name");            // Option<bool>
ctx.get_input("name");           // Option<&serde_json::Value>
ctx.get_input_as::<T>("name");   // Option<T>
ctx.require_input_as::<T>("name");
```

Common output and control helpers:

```rust
ctx.set_output("result", value);
ctx.set_output_json("result", &value);
ctx.activate_exec("exec_out");
ctx.success();
ctx.fail("What went wrong");
```

Logging and streaming are also available through the context:

```rust
ctx.info("Starting work");
ctx.stream_text("Partial result");
ctx.stream_progress(0.5, "Halfway");
```

## Typed struct pins

Derive `JsonSchema` for a serializable Rust type, then attach the schema to a
struct pin:

```rust
#[derive(Default, serde::Serialize, serde::Deserialize, JsonSchema)]
struct Request {
    query: String,
    limit: u32,
}

node.add_input_pin(
    "request",
    "Request",
    "Search request",
    VariableType::Struct,
)
.set_schema::<Request>()
.set_enforce_schema(true);
```

Read it with `ctx.get_input_as::<Request>("request")`.

## Permissions

Declare each capability on the node that uses it:

```rust
node.add_permission(NodePermission::NetworkHttp);
node.add_permission(NodePermission::StorageRead);
node.add_permission(NodePermission::StorageWrite);
```

Permissions are part of the exported node definition and drive the execution
sandbox. Package memory and timeout limits remain in `flow-like.toml`; see the
[manifest reference](/dev/wasm-nodes/manifest/).

Do not use manifest capability flags as a substitute for
`node.add_permission(...)`.

## Test locally

The template's tests run on the native host target so node logic can be tested
without loading WASM:

```bash
mise run test
```

From the repository root, run the template definition lint and runtime
integration suite:

```bash
mise run test:wasm:rust:lint
mise run test:wasm:rust:e2e
```

## Publish

1. Run `mise run build`.
2. Open Flow-Like Desktop.
3. Go to **Library → Packages → Publish**.
4. Select `node.wasm` and `flow-like.toml`.
5. Review the extracted nodes and submit the package.

There is no checked-in `flow-like publish` CLI and no supported
`~/.flow-like/nodes` copy-install workflow.

## Related

- [Package Manifest](/dev/wasm-nodes/manifest/)
- [WASM Nodes Overview](/dev/wasm-nodes/overview/)
- [Writing Native Nodes](/dev/writing-nodes/)
