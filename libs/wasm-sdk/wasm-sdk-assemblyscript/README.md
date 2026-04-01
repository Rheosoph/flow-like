# @flow-like/wasm-sdk-assemblyscript

AssemblyScript SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes. AssemblyScript compiles **directly to WASM** — no JavaScript runtime is involved, resulting in small, fast binaries ideal for compute-intensive nodes.

## Install

```bash
npm install @flow-like/wasm-sdk-assemblyscript
# or
bun add @flow-like/wasm-sdk-assemblyscript
```

> **Note:** `assemblyscript` must be installed as a dev dependency in your project.
>
> ```bash
> bun add -D assemblyscript
> ```

## Quick Start — Single Node

```typescript
// assembly/index.ts
import {
  DataType,
  ExecutionResult,
  NodeDefinition,
  PinDefinition,
  Context,
  FlowNode,
  singleNode,
  runSingle,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

export { alloc, dealloc, get_abi_version } from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

class UppercaseNode extends FlowNode {
  define(): NodeDefinition {
    const def = new NodeDefinition();
    def.name = "uppercase";
    def.friendly_name = "Uppercase";
    def.description = "Converts a string to uppercase";
    def.category = "Text/Transform";

    def.addPin(PinDefinition.inputExec("exec"));
    def.addPin(PinDefinition.inputPin("text", DataType.String));
    def.addPin(PinDefinition.outputExec("exec_out"));
    def.addPin(PinDefinition.outputPin("result", DataType.String));

    return def;
  }

  execute(ctx: Context): ExecutionResult {
    const text = ctx.getString("text");
    ctx.setOutput("result", text.toUpperCase());
    return ctx.success("exec_out");
  }
}

const node = new UppercaseNode();

export function get_nodes(): i64 { return singleNode(node); }
export function run(ptr: i32, len: i32): i64 { return runSingle(node, ptr, len); }
```

## Quick Start — Node Package (Multiple Nodes)

```typescript
// assembly/index.ts
import {
  NodePackage,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

export { alloc, dealloc, get_abi_version } from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

import { AddNode } from "./nodes/add";
import { SubtractNode } from "./nodes/subtract";

const pkg = new NodePackage();
pkg.register(new AddNode());
pkg.register(new SubtractNode());

export function get_nodes(): i64 { return pkg.getNodes(); }
export function run(ptr: i32, len: i32): i64 { return pkg.run(ptr, len); }
```

## Building

```bash
# Install dependencies
bun install

# Build release WASM
bun run build

# Output: build/sdk.wasm
```

The SDK ships a pre-configured `asconfig.json` — your project's `asconfig.json` can extend it:

```json
{
  "extends": "node_modules/@flow-like/wasm-sdk-assemblyscript/asconfig.json",
  "targets": {
    "release": {
      "outFile": "build/my_node.wasm"
    }
  }
}
```

## Project Structure

The recommended way to get started is via the [wasm-node-assemblyscript template](../../../templates/wasm-node-assemblyscript/):

```
assembly/
  index.ts       ← Entry point, exports get_nodes() and run()
  my_node.ts     ← Your FlowNode subclass
asconfig.json
package.json
```

## Publishing

```bash
bun run publish:npm
```

## API Reference

### `FlowNode` (abstract class)

Extend this class to implement a node:

```typescript
class MyNode extends FlowNode {
  define(): NodeDefinition { /* return node schema */ }
  execute(ctx: Context): ExecutionResult { /* run logic */ }
}
```

### `NodePackage`

Register multiple nodes and dispatch by name:

```typescript
const pkg = new NodePackage();
pkg.register(new MyNode());
pkg.register(new OtherNode());
```

### `NodeDefinition`

| Method | Description |
|---|---|
| `addPin(pin)` | Add an input or output pin |
| `setScores(scores)` | Set optional quality scores |

### `PinDefinition`

| Static Method | Description |
|---|---|
| `inputExec(name)` | Execution trigger input |
| `outputExec(name)` | Execution trigger output |
| `inputPin(name, DataType)` | Typed data input |
| `outputPin(name, DataType)` | Typed data output |

### `Context`

| Method | Description |
|---|---|
| `getString(pin)` | Read a string input |
| `getBool(pin)` | Read a boolean input |
| `getI64(pin)` | Read an integer input |
| `getF64(pin)` | Read a float input |
| `setOutput(pin, value)` | Write an output value |
| `success(execPin)` | Return a success result |
| `error(msg)` | Return an error result |
| `logDebug/Info/Warn/Error(msg)` | Log a message |
| `nodeId / runId / appId / boardId` | Read runtime metadata |

### `DataType` enum

`Exec`, `String`, `Boolean`, `Integer`, `Float`, `Json`, `Generic`, `Array`, `HashMap`
