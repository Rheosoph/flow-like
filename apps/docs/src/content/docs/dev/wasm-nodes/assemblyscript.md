---
title: AssemblyScript WASM Nodes
description: Create custom WASM nodes using AssemblyScript with the Core Module runtime
sidebar:
  order: 10
  badge:
    text: Core Module
    variant: note
---

AssemblyScript compiles TypeScript-like syntax directly to compact Core Module WASM binaries. The template uses a class-based SDK with typed getters/setters — no raw JSON wrangling required.

:::note
Core Modules have **no access to TCP, UDP, or DNS**. They communicate with the host exclusively through FFI imports from the `env` module. See [Runtime Models](/dev/wasm-nodes/runtime-models/) for details.
:::

## Prerequisites

- **Node.js** 18+ and **npm**
- The [`@flow-like/wasm-sdk-assemblyscript`](https://www.npmjs.com/package/@flow-like/wasm-sdk-assemblyscript) SDK (installed via npm)

## Project Setup

```bash
mkdir my-as-node && cd my-as-node
npm init -y
npm install @flow-like/wasm-sdk-assemblyscript
npm install --save-dev assemblyscript@^0.28.9
```

### Compiler Configuration

```json title="asconfig.json"
{
  "targets": {
    "debug": {
      "outFile": "build/debug.wasm",
      "textFile": "build/debug.wat",
      "sourceMap": true,
      "debug": true
    },
    "release": {
      "outFile": "build/release.wasm",
      "textFile": "build/release.wat",
      "sourceMap": true,
      "optimizeLevel": 3,
      "shrinkLevel": 2,
      "converge": false,
      "noAssert": false
    }
  },
  "options": {
    "bindings": "esm",
    "exportRuntime": true,
    "runtime": "stub"
  }
}
```

### Build Scripts

```json title="package.json (scripts)"
{
  "scripts": {
    "build": "asc assembly/index.ts --target release",
    "build:debug": "asc assembly/index.ts --target debug"
  }
}
```

## Template Code

The SDK provides a `FlowNode` base class. Extend it, implement `define()` and `execute()`, then wire up the required exports:

```typescript title="assembly/index.ts"
import {
  type Context,
  DataType,
  type ExecutionResult,
  FlowNode,
  NodeDefinition,
  PinDefinition,
  runSingle,
  singleNode,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

// Re-export memory management + ABI helpers from the SDK
export {
  alloc,
  dealloc,
  get_abi_version,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

class MyCustomNode extends FlowNode {
  define(): NodeDefinition {
    const def = new NodeDefinition();
    def.name = "my_custom_node_as";
    def.friendly_name = "My Custom Node (AS)";
    def.description = "A template WASM node built with AssemblyScript";
    def.category = "Custom/WASM";
    def.addPermission("streaming");

    // Input pins
    def.addPin(
      PinDefinition.input("exec", "Execute", "Trigger execution", DataType.Exec),
    );
    def.addPin(
      PinDefinition.input("input_text", "Input Text", "Text to process", DataType.String)
        .withDefaultString(""),
    );
    def.addPin(
      PinDefinition.input("multiplier", "Multiplier", "Number of times to repeat", DataType.I64)
        .withDefaultI64(1),
    );

    // Output pins
    def.addPin(
      PinDefinition.output("exec_out", "Done", "Execution complete", DataType.Exec),
    );
    def.addPin(
      PinDefinition.output("output_text", "Output Text", "Processed text", DataType.String),
    );
    def.addPin(
      PinDefinition.output("char_count", "Character Count", "Number of characters in output", DataType.I64),
    );

    return def;
  }

  execute(ctx: Context): ExecutionResult {
    const inputText = ctx.getString("input_text");
    const multiplier = ctx.getI64("multiplier", 1);

    let outputText = "";
    for (let i: i64 = 0; i < multiplier; i++) {
      outputText += inputText;
    }

    ctx.setString("output_text", outputText);
    ctx.setI64("char_count", outputText.length);
    return ctx.success();
  }
}

// Instantiate and wire up the required exports
const node = new MyCustomNode();

export function get_node(): i64 {
  return singleNode(node);
}

export function run(ptr: i32, len: i32): i64 {
  return runSingle(node, ptr, len);
}
```

## Build

```bash
npm run build
```

Output: `build/release.wasm`

For a debug build with source maps:

```bash
npm run build:debug
```

## Required Exports

Every Core Module WASM node must export these symbols:

| Export | Purpose |
|--------|---------|
| `alloc(size) → ptr` | Allocate memory for the host to write into |
| `dealloc(ptr, size)` | Free previously allocated memory |
| `get_node() → ptr` | Return the JSON node definition |
| `get_nodes() → ptr` | Return a JSON array of all node definitions |
| `run(ptr, len) → ptr` | Execute the node with the given context |
| `get_abi_version() → u32` | Return the ABI version (currently `1`) |

The SDK re-exports `alloc`, `dealloc`, and `get_abi_version` — you only need to implement `get_node` and `run` yourself.

## SDK API

### Pin Definitions

Use `PinDefinition` static factories to declare inputs and outputs:

```typescript
PinDefinition.input(name, friendlyName, description, DataType.String)
PinDefinition.output(name, friendlyName, description, DataType.I64)
```

Chain defaults with `.withDefaultString("")`, `.withDefaultI64(0)`, etc.

### Context Methods

Read input values and write outputs inside `execute()`:

```typescript
// Getters
ctx.getString("pin_name")       // string
ctx.getI64("pin_name", default) // i64
ctx.getF64("pin_name", default) // f64
ctx.getBool("pin_name", default) // bool

// Setters
ctx.setString("pin_name", value)
ctx.setI64("pin_name", value)
ctx.setF64("pin_name", value)
ctx.setBool("pin_name", value)

// Execution control
ctx.success()  // signals completion, activates exec_out
```

### Data Types

| `DataType` | AssemblyScript type |
|------------|---------------------|
| `Exec` | — (execution flow) |
| `String` | `string` |
| `I64` | `i64` |
| `F64` | `f64` |
| `Bool` | `bool` |

## Host FFI

Under the hood, the SDK calls `flowlike_*` functions imported from the `env` module. You don't need to use these directly — the `Context` and `FlowNode` classes abstract them away.

## Install

```bash
cp build/release.wasm ~/.flow-like/nodes/my-custom-node.wasm
```

## Related

- [WASM Nodes Overview](/dev/wasm-nodes/overview/)
- [Runtime Models](/dev/wasm-nodes/runtime-models/) — Core Module vs Component Model
- [TypeScript WASM Nodes](/dev/wasm-nodes/typescript/) — alternative TS approaches (Javy)
- [Rust Template](/dev/wasm-nodes/rust/)
- [Package Manifest](/dev/wasm-nodes/manifest/)
