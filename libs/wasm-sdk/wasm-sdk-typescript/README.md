# @flow-like/wasm-sdk-typescript

TypeScript SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes using the **WIT Component Model** via [@bytecodealliance/componentize-js](https://github.com/bytecodealliance/ComponentizeJS).

Unlike AssemblyScript (which compiles directly to WASM), this SDK targets a standard TypeScript/JavaScript runtime — your node logic is written in plain TypeScript, bundled with `esbuild`, and then compiled to a WASM component with `componentize-js`.

## Install

```bash
npm install @flow-like/wasm-sdk-typescript
# or
bun add @flow-like/wasm-sdk-typescript
```

## Quick Start — Single Node

```typescript
import {
  NodeDefinition,
  PinDefinition,
  Context,
  ExecutionResult,
} from "@flow-like/wasm-sdk-typescript";

export function get_definition(): NodeDefinition {
  const def = new NodeDefinition();
  def.name = "uppercase";
  def.friendly_name = "Uppercase";
  def.description = "Converts a string to uppercase";
  def.category = "Text/Transform";

  def.addPin(PinDefinition.inputExec("exec"));
  def.addPin(PinDefinition.inputPin("text", "String"));
  def.addPin(PinDefinition.outputExec("exec_out"));
  def.addPin(PinDefinition.outputPin("result", "String"));

  return def;
}

export function run(ctx: Context): ExecutionResult {
  const text = ctx.getString("text") ?? "";
  ctx.setOutput("result", text.toUpperCase());
  return ctx.success("exec_out");
}
```

## Quick Start — Node Package (Multiple Nodes)

```typescript
import {
  NodeDefinition,
  PinDefinition,
  Context,
  ExecutionResult,
  PackageNodes,
} from "@flow-like/wasm-sdk-typescript";

// Define node 1
function defineAdd(): NodeDefinition { /* ... */ }
function runAdd(ctx: Context): ExecutionResult { /* ... */ }

// Define node 2
function defineSubtract(): NodeDefinition { /* ... */ }
function runSubtract(ctx: Context): ExecutionResult { /* ... */ }

// Register all nodes
const pkg = new PackageNodes();
pkg.addNode(defineAdd(), runAdd);
pkg.addNode(defineSubtract(), runSubtract);

export const get_nodes = () => pkg.getNodes();
export const run = (ctx: Context) => pkg.run(ctx);
```

## Testing with MockHostBridge

The SDK ships a `MockHostBridge` so you can test node logic without a real runtime:

```typescript
import { describe, it, expect } from "vitest";
import {
  Context,
  ExecutionInput,
  MockHostBridge,
} from "@flow-like/wasm-sdk-typescript";
import { run } from "./src/node";

describe("uppercase node", () => {
  it("converts input to uppercase", () => {
    const host = new MockHostBridge();
    const input = new ExecutionInput({ text: JSON.stringify("hello") });
    const ctx = new Context(input, host);

    const result = run(ctx);

    expect(result.outputs?.result).toBe(JSON.stringify("HELLO"));
  });
});
```

## JSON Schema Pins

Use `PinDefinition.withSchemaType()` together with [TypeBox](https://github.com/sinclairzx81/typebox) for fully typed JSON pins. TypeBox is re-exported from the SDK:

```typescript
import { PinDefinition, Type } from "@flow-like/wasm-sdk-typescript";

const schema = Type.Object({
  name: Type.String(),
  age: Type.Number(),
});

const pin = PinDefinition.inputPin("person", "Object").withSchemaType(schema);
```

## Building

This SDK is intended to be used alongside `@bytecodealliance/componentize-js`. The recommended project structure is the [wasm-node-typescript template](../../../templates/wasm-node-typescript/).

```bash
# Install deps
bun install

# Build WASM component
node build.mjs

# Output: build/node.wasm
```

## Publishing

```bash
bun run publish:npm
```

This runs `tsc` with declaration maps and source maps, then publishes with `bun publish --access public`.

## API Reference

### `NodeDefinition`

| Method | Description |
|---|---|
| `addPin(pin)` | Add an input or output pin |
| `setScores(scores)` | Set node quality scores |

### `PinDefinition`

| Static Method | Description |
|---|---|
| `inputExec(name)` | Create an execution input pin |
| `outputExec(name)` | Create an execution output pin |
| `inputPin(name, type)` | Create a typed input pin |
| `outputPin(name, type)` | Create a typed output pin |

### `Context`

| Method | Description |
|---|---|
| `getString(pin)` | Read a string input |
| `getBool(pin)` | Read a boolean input |
| `getI64(pin)` | Read an integer input |
| `getF64(pin)` | Read a float input |
| `getJson(pin)` | Read a JSON input |
| `setOutput(pin, value)` | Write an output value |
| `success(execPin?)` | Return a success result |
| `error(message)` | Return an error result |
| `logDebug/Info/Warn/Error(msg)` | Log a message |

### `ExecutionResult`

| Static Method | Description |
|---|---|
| `ExecutionResult.ok(outputs, pin)` | Successful result |
| `ExecutionResult.fail(error)` | Failed result |
