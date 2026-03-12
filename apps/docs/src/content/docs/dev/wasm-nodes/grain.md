---
title: Grain WASM Nodes
description: Create custom WASM nodes using Grain
sidebar:
  order: 15
  badge:
    text: Core Module
    variant: note
---

Grain is a strongly-typed functional language that compiles to WebAssembly. The template produces a **Core Module** (not a Component Model module) — see [Runtime Models](/dev/wasm-nodes/runtime-models/) for the difference. With `--no-gc` and `--elide-type-info`, Grain produces compact binaries with stable memory pointers suitable for the host ABI.

## Prerequisites

- [Grain 0.6+](https://grain-lang.org/docs/getting_grain)

```bash
# macOS
brew install --no-quarantine --cask grain-lang/tap/grain

# Or download the binary directly
sudo curl -L --output /usr/local/bin/grain \
  https://github.com/grain-lang/grain/releases/download/grain-v0.7.2/grain-mac-x64 \
  && sudo chmod +x /usr/local/bin/grain
```

## Project Structure

```
wasm-node-grain/
├── src/
│   └── main.gr           # Main node implementation
├── flow-like.toml         # Flow-Like package manifest
├── mise.toml              # Build tasks
└── README.md
```

The SDK lives in `../wasm-sdk-grain/` and is included via the `-I` compiler flag. It provides `Types`, `Context`, `Memory`, and `Sdk` modules.

## Template Code

### Node Definition

Edit `src/main.gr` and modify `buildDefinition` to describe your node's pins and metadata:

```grain title="src/main.gr"
module Main

from "sdk" include Sdk
from "types" include Types
from "memory" include Memory
from "context" include Context

from "string" include String
from "runtime/unsafe/wasmi32" include WasmI32
from "runtime/unsafe/wasmi64" include WasmI64

let buildDefinition = () => {
  let mut def = Types.newNodeDefinition()
  def.name = "my_custom_node_grain"
  def.friendlyName = "My Custom Node (Grain)"
  def.description = "A template WASM node built with Grain"
  def.category = "Custom/WASM"

  let def = Types.addPermission(def, "streaming")

  let def = Types.addPin(
    def,
    Types.inputPin("exec", "Execute", "Trigger execution", Types.Exec),
  )
  let def = Types.addPin(
    def,
    Types.withDefault(
      Types.inputPin("input_text", "Input Text", "Text to process", Types.TypeString),
      "\"\"",
    ),
  )
  let def = Types.addPin(
    def,
    Types.outputPin("exec_out", "Done", "Execution complete", Types.Exec),
  )
  let def = Types.addPin(
    def,
    Types.outputPin("output_text", "Output Text", "Processed text", Types.TypeString),
  )

  def
}
```

### Exports

Every Core Module node must export `get_node`, `get_nodes`, `run`, `alloc`, `dealloc`, and `get_abi_version`. Grain uses `@externalName("...")` to set the export name and `@unsafe` for raw pointer operations. Memory packing returns a packed i64 (`ptr << 32 | len`).

```grain title="src/main.gr"
@unsafe
@externalName("get_node")
provide let _getNode = () => {
  let def = buildDefinition()
  Memory.packString(Types.nodeDefToJson(def))
}

@unsafe
@externalName("get_nodes")
provide let _getNodes = () => {
  let def = buildDefinition()
  let json = "[" ++ Types.nodeDefToJson(def) ++ "]"
  Memory.packResult(json)
}
```

### Run Handler

The `run` export receives a pointer and length to the JSON execution input. Use `Context` helpers to read inputs, set outputs, log, and stream:

```grain title="src/main.gr"
@unsafe
@externalName("run")
provide let _run = (ptr: WasmI32, len: WasmI32) => {
  let inputJson = Memory.ptrToString(ptr, len)
  let input = Types.parseExecutionInput(inputJson)
  let ctx = Context.init(input)

  let inputText = Context.getString(ctx, "input_text", "")
  let multiplier = Context.getI64(ctx, "multiplier", 1)

  Context.debug(ctx, "Processing: '" ++ inputText ++ "' x " ++ toString(multiplier))

  let mut output = ""
  for (let mut i = 0; i < multiplier; i += 1) {
    output = output ++ inputText
  }

  Context.setOutput(ctx, "output_text", Types.jsonString(output))
  Context.setOutput(ctx, "char_count", toString(String.length(output)))

  let result = Context.success(ctx)
  Memory.packString(Types.resultToJson(result))
}
```

### Memory Management

Re-export `alloc` and `dealloc` — required by the host to pass data across the WASM boundary:

```grain title="src/main.gr"
@unsafe
@externalName("alloc")
provide let _alloc = (size: WasmI32) => {
  Memory.wasmAlloc(size)
}

@unsafe
@externalName("dealloc")
provide let _dealloc = (ptr: WasmI32, size: WasmI32) => {
  Memory.wasmDealloc(ptr, size)
}

@unsafe
@externalName("get_abi_version")
provide let _getAbiVersion = () => {
  1n
}
```

### Context API

| Method | Description |
|---|---|
| `Context.getString(ctx, pin, default)` | Get string input |
| `Context.getI64(ctx, pin, default)` | Get integer input |
| `Context.getF64(ctx, pin, default)` | Get float input |
| `Context.getBool(ctx, pin, default)` | Get boolean input |
| `Context.setOutput(ctx, pin, value)` | Set an output value |
| `Context.debug(ctx, msg)` | Log debug message |
| `Context.logInfo(ctx, msg)` | Log info message |
| `Context.warn(ctx, msg)` | Log warning |
| `Context.logError(ctx, msg)` | Log error |
| `Context.streamText(ctx, text)` | Stream text to the client |
| `Context.success(ctx)` | Finalize with `exec_out` activated |
| `Context.fail(ctx, msg)` | Finalize with an error |

## Build

```bash
grain compile --release --no-gc --elide-type-info \
  -I ../wasm-sdk-grain -o build/node.wasm src/main.gr
```

Output: `build/node.wasm`

Or use [mise](https://mise.jdx.dev/) tasks from the template:

```bash
mise run setup   # verify Grain is installed
mise run build   # compile to build/node.wasm
mise run test    # run unit tests
mise run clean   # remove build artifacts
```

### Compiler Flags

| Flag | Purpose |
|---|---|
| `--release` | Enable optimizations (smaller + faster binary) |
| `--no-gc` | Disable Grain's GC — critical for core module ABI compatibility |
| `--elide-type-info` | Strip runtime type info to reduce binary size |
| `-I <path>` | Add include directory for SDK module resolution |
| `-o <file>` | Output file path |

:::caution
`--no-gc` is required. Without it, Grain's garbage collector may relocate objects, breaking the pointer-based host ABI.
:::

## Related

- [Overview](/dev/wasm-nodes/overview/) — How WASM nodes work
- [Runtime Models](/dev/wasm-nodes/runtime-models/) — Core Module vs Component Model
- [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
- [Rust WASM Nodes](/dev/wasm-nodes/rust/) — Recommended language with SDK macros
