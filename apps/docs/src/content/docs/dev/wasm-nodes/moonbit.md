---
title: MoonBit WASM Nodes
description: Create custom WASM nodes using MoonBit
sidebar:
  order: 16
  badge:
    text: Core Module
    variant: note
---

MoonBit compiles directly to WASM via its native `--target wasm` backend, producing compact Core Module binaries without any component-model tooling. The SDK is imported as the `@sdk` package alias, and all six required exports are declared in `moon.pkg.json`.

This template uses the **Core Module** runtime model — see [Runtime Models](/dev/wasm-nodes/runtime-models/) for how this differs from the Component Model.

## Prerequisites

- [MoonBit toolchain](https://www.moonbitlang.com/download/) (`moon` CLI)
- The local SDK directory (`../wasm-sdk-moonbit`) placed next to the template

## Important Files

| Path | Purpose |
|------|---------|
| `node.mbt` | Defines node metadata, run logic, and exported Core Module functions |
| `moon.mod.json` | Declares the module and local SDK dependency |
| `moon.pkg.json` | Configures imports, WASM exports, and exported memory |
| `examples/http_request.mbt` | Demonstrates an HTTP request |
| `flow-like.toml` | Declares the Flow-Like package |
| `mise.toml` | Provides setup, build, test, and clean tasks |
| `README.md` | Template-specific setup and usage notes |

### moon.mod.json

The module manifest declares the SDK as a local path dependency. Once the SDK is published to [mooncakes.io](https://mooncakes.io/), replace the path with a version string.

```json title="moon.mod.json"
{
  "name": "example/custom-node-moonbit",
  "version": "0.1.0",
  "deps": {
    "felix-schultz/flow-like-wasm-sdk": { "path": "../wasm-sdk-moonbit" }
  }
}
```

### moon.pkg.json

The package config marks this as the main entrypoint, imports the SDK under the `sdk` alias, and lists all six required WASM exports plus the exported memory.

```json title="moon.pkg.json"
{
  "is-main": true,
  "import": [
    "moonbitlang/core/string",
    { "path": "felix-schultz/flow-like-wasm-sdk", "alias": "sdk" }
  ],
  "link": {
    "wasm": {
      "exports": [
        "get_node", "get_nodes", "run",
        "alloc", "dealloc", "get_abi_version"
      ],
      "export-memory-name": "memory"
    }
  }
}
```

## Template Code

### Node Definition

`get_definition()` builds the node metadata, permissions, and pins using the `@sdk` helpers:

```moonbit title="node.mbt"
fn get_definition() -> @sdk.NodeDefinition {
  let def = @sdk.NodeDefinition::new(
    "my_custom_node_moonbit",
    "My Custom Node (MoonBit)",
    "A template WASM node built with MoonBit",
    "Custom/WASM",
  )
  def.add_permission("streaming")

  def.add_pin(@sdk.input_pin("exec", "Execute", "Trigger execution", @sdk.data_type_exec()))
  def.add_pin(
    @sdk.input_pin("input_text", "Input Text", "Text to process", @sdk.data_type_string())
      .with_default("\"\""),
  )
  def.add_pin(
    @sdk.input_pin("multiplier", "Multiplier", "Number of times to repeat", @sdk.data_type_i64())
      .with_default("1"),
  )
  def.add_pin(@sdk.output_pin("exec_out", "Done", "Execution complete", @sdk.data_type_exec()))
  def.add_pin(@sdk.output_pin("output_text", "Output Text", "Processed text", @sdk.data_type_string()))
  def.add_pin(@sdk.output_pin("char_count", "Character Count", "Number of characters in output", @sdk.data_type_i64()))

  def
}
```

### Run Handler

`handle_run` receives a `@sdk.Context`, reads inputs, writes outputs, and returns an `ExecutionResult`:

```moonbit title="node.mbt"
fn handle_run(ctx : @sdk.Context) -> @sdk.ExecutionResult {
  let input_text = ctx.get_string("input_text")
  let multiplier = ctx.get_i64("multiplier", default=1)

  let mut output_text = ""
  for _i = 0; _i < multiplier; _i = _i + 1 {
    output_text = output_text + input_text
  }

  let char_count = output_text.length()
  ctx.stream_text("Generated " + char_count.to_string() + " characters")

  ctx.set_output("output_text", @sdk.json_string(output_text))
  ctx.set_output("char_count", char_count.to_string())

  ctx.success()
}
```

### Exports

Every Core Module node must export these six `pub fn` symbols. The SDK provides the underlying serialization and memory management:

```moonbit title="node.mbt"
pub fn get_node() -> Int64 {
  @sdk.serialize_definition(get_definition())
}

pub fn get_nodes() -> Int64 {
  let json = "[" + get_definition().to_json() + "]"
  @sdk.serialize_to_mem(json)
}

pub fn run(ptr : Int, len : Int) -> Int64 {
  let input = @sdk.parse_input(ptr, len)
  let ctx = @sdk.Context::new(input)
  let result = handle_run(ctx)
  @sdk.serialize_result(result)
}

pub fn alloc(size : Int) -> Int { @sdk.alloc(size) }
pub fn dealloc(ptr : Int, size : Int) -> Unit { @sdk.dealloc(ptr, size) }
pub fn get_abi_version() -> Int { @sdk.abi_version }
```

Host FFI functions (logging, I/O) are imported from the `env` module via MoonBit's `extern "wasm"` mechanism inside the SDK's `host.mbt`.

### Context API

| Method | Description |
|---|---|
| `ctx.get_string(name, default="")` | Read a string input |
| `ctx.get_i64(name, default=0)` | Read an integer input |
| `ctx.get_f64(name, default=0.0)` | Read a float input |
| `ctx.get_bool(name, default=false)` | Read a boolean input |
| `ctx.set_output(name, value)` | Set an output pin value |
| `ctx.success()` | Finish with success (activates `exec_out`) |
| `ctx.fail(error)` | Finish with an error |
| `ctx.debug(msg)` / `info` / `warn` / `error` | Log at various levels |
| `ctx.stream_text(text)` | Stream partial text output |
| `ctx.stream_json(data)` | Stream JSON data |
| `ctx.stream_progress(pct, msg)` | Stream a progress update |

## Build

```bash
# One-time: verify the SDK is in place
mise run setup

# Build the Core Module
moon build --target wasm --release
mkdir -p build
cp _build/wasm/release/build/custom-node-moonbit.wasm build/node.wasm

# Or simply:
mise run build
```

Output: `build/node.wasm`

```bash
mise run test    # run unit tests
mise run clean   # remove _build/, build/, .mooncakes/
```

## Related

- [Overview](/dev/wasm-nodes/overview/) — How WASM nodes work
- [Runtime Models](/dev/wasm-nodes/runtime-models/) — Core Module vs Component Model
- [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
- [Rust WASM Nodes](/dev/wasm-nodes/rust/) — Recommended language with SDK macros
