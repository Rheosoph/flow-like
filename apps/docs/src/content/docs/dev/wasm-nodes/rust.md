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

The template configures the library as `cdylib`. Its checked-in dependency
currently includes a relative path to `libs/wasm-sdk/wasm-sdk-rust` alongside
version `0.4.0`, so copying the directory alone still requires changing that
dependency. Use the published crate after confirming that the required version
is available, or keep the SDK at the path expected by the manifest. The SDK
release procedure below includes the check for a standalone template.

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
sandbox. Package memory and timeout tiers, the host allowlist, and OAuth scopes
remain in `flow-like.toml`; see the
[manifest reference](/dev/wasm-nodes/manifest/).

Do not author capability flags in the manifest. The registry derives the
store's capability listing from `node.add_permission(...)`.

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

## Release the Rust SDK

SDK maintainers publish `flow-like-wasm-sdk` to crates.io before changing the
template to use a new registry version. Run these commands from the Flow-Like
repository root with its pinned Rust toolchain. The SDK does not depend on
Wasmtime, so the runtime's Rust minimum is a separate requirement.

The examples below use the current SDK version, `0.4.0`. Read the SDK manifest
before a later release and substitute its version in registry checks and the
template dependency. The macros crate remains at `0.3.7`; publish it separately
only when its own version changes.

### Verify the package

Install the component target, then test with and without the optional Rig
agent library:

```bash
rustup target add wasm32-wasip2
cargo test --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --target host-tuple
cargo test --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --all-features --target host-tuple
cargo package --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --allow-dirty --list
mise run publish:wasm:rust:dry-run
```

Review the [SDK changelog](https://github.com/Rheosoph/flow-like/blob/dev/libs/wasm-sdk/wasm-sdk-rust/CHANGELOG.md)
and package listing. The archive must include `src/resources.rs` and
`wit/flow-like-node.wit`. Cargo includes the WIT symlink's content in the
archive, so consumers do not need a separate WIT checkout.

The dry-run task builds the packaged source for the native host and
`wasm32-wasip2`, with all SDK features and locked dependencies, without
uploading. It uses `--allow-dirty` to validate changes before commit. Commit
the release files before using the publication task, which rejects dirty
package contents.

### Publish and confirm the version

Use a crates.io account allowed to publish `flow-like-wasm-sdk`. If Cargo is
not authenticated, run this in your own terminal and enter the token at its
prompt:

```bash
cargo login --registry crates-io
mise run publish:wasm:rust
cargo info flow-like-wasm-sdk@0.4.0 --registry crates-io
```

The SDK task uploads only the SDK. A published version cannot be overwritten.
If Cargo reports an index timeout after upload, check the registry version
before trying another upload. Update the SDK index in `libs/wasm-sdk/README.md`
once the version is available. Use `mise run publish:wasm:rust:macros` only for
a changed macros crate.

To publish reviewed but uncommitted package changes deliberately, Cargo
accepts the explicit override:

```bash
cargo publish --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --all-features --registry crates-io --allow-dirty
```

### Verify the standalone template

After confirming publication, replace the dependency and its local path
comments in `templates/wasm-node-rust/Cargo.toml` with:

```toml
flow-like-wasm-sdk = { version = "0.4.0", features = ["rig"] }
```

Keep `rig` enabled because the template includes agent examples. Remove any
template instructions requiring a sibling SDK checkout and replace
repository-relative SDK links with public links. Regenerate its lockfile and
test the published dependency:

```bash
cargo update --manifest-path templates/wasm-node-rust/Cargo.toml -p flow-like-wasm-sdk --precise 0.4.0
cargo test --manifest-path templates/wasm-node-rust/Cargo.toml --locked --target host-tuple
cargo build --manifest-path templates/wasm-node-rust/Cargo.toml --locked --release --target wasm32-wasip2
```

The SDK's template lockfile entry must have a registry source and checksum.
Copy the template outside this repository, repeat its tests and component
build, and confirm that it needs no sibling SDK directory. Commit the template
manifest, lockfile, and documentation changes together.

## Related

- [Package Manifest](/dev/wasm-nodes/manifest/)
- [WASM Nodes Overview](/dev/wasm-nodes/overview/)
- [Writing Native Nodes](/dev/writing-nodes/)
