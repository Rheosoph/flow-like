---
title: TypeScript WASM Nodes
description: Build Flow-Like WASM nodes in TypeScript with ComponentizeJS
sidebar:
  order: 3
  badge:
    text: Component Model
    variant: success
---

Flow-Like's TypeScript SDK uses standard TypeScript/JavaScript, `esbuild`, and
Bytecode Alliance's `componentize-js`. The result is a WASM Component that
implements Flow-Like's WIT world.

This is the supported TypeScript path. You do not need AssemblyScript or Javy.

## Start from the template

Copy `templates/wasm-node-typescript`, then run:

```bash
mise run setup
mise run test
mise run build
```

The build task installs Node.js 22 dependencies, copies the canonical WIT file,
bundles `src/app.ts`, and componentizes it to:

```text
build/node.wasm
```

Without mise:

```bash
npm install
npm test
npm run build
```

The project depends on `@flow-like/wasm-sdk-typescript` and
`@bytecodealliance/componentize-js`.

## Define a node

Edit `src/node.ts`. The template's `src/app.ts` supplies the WIT bridge and
exports:

```typescript title="src/node.ts"
import {
  type Context,
  type ExecutionResult,
  NodeDefinition,
  PinDefinition,
  PinType,
} from "@flow-like/wasm-sdk-typescript";

export function getDefinition(): NodeDefinition {
  const node = new NodeDefinition(
    "uppercase_ts",
    "Uppercase",
    "Converts text to uppercase",
    "Custom/Text",
  );

  node.addPin(PinDefinition.inputExec("exec"));
  node.addPin(
    PinDefinition.inputPin("text", PinType.STRING, {
      defaultValue: "",
    }),
  );
  node.addPin(PinDefinition.outputExec("exec_out"));
  node.addPin(PinDefinition.outputPin("result", PinType.STRING));

  return node;
}

export function run(ctx: Context): ExecutionResult {
  const text = ctx.getString("text", "") ?? "";
  ctx.setOutput("result", text.toUpperCase());
  ctx.activateExec("exec_out");
  return ctx.success();
}
```

Keep the node name and pin names stable after publishing; boards persist those
identifiers.

## Pin types

The SDK exports constants for the Flow-Like value types:

| Constant | Value |
| --- | --- |
| `PinType.STRING` | String |
| `PinType.I64` | 64-bit integer |
| `PinType.F64` | 64-bit float |
| `PinType.BOOL` | Boolean |
| `PinType.GENERIC` | JSON value |
| `PinType.BYTES` | Binary data |

Use `PinDefinition.inputExec` and `outputExec` for flow-control pins. The SDK
also supports struct schemas, collections, defaults, options, and sensitive
inputs.

## Permissions

Declare permissions on the node definition:

```typescript
node.addPermission("network:http");
node.addPermission("streaming");
```

These labels are exported with the node and configure its execution sandbox.
Other common labels include `storage:read`, `storage:write`, `variables`,
`cache`, `models`, `a2ui`, `oauth`, and `functions`.

Package memory and timeout limits belong in `flow-like.toml`; see the
[manifest reference](/dev/wasm-nodes/manifest/).

## Context and host services

The context reads typed inputs and writes outputs:

```typescript
const name = ctx.getString("name", "") ?? "";
const count = ctx.getI64("count", 1) ?? 1;

ctx.setOutput("result", name.repeat(Math.max(count, 0)));
```

The template's bridge also connects logging, streaming, variables, cache,
storage, models, OAuth, and HTTP to Flow-Like host imports. Declare the matching
permission before calling a gated host service.

## Multi-node packages

The checked-in TypeScript template exposes one `getDefinition`/`run` pair. To
ship multiple nodes, keep the WIT entry point responsible for returning every
definition and dispatching `run` by `ctx.nodeName`. Use the repository's
multi-node SDK/package helpers as the reference rather than adding `[[nodes]]`
tables to the manifest; manifest node tables are not part of the current typed
manifest.

## Test

The template uses Vitest:

```bash
mise run test
```

Tests run the TypeScript node logic with a mock host, so they are fast and do
not require the desktop app. Run `mise run build` as a separate integration
check because successful unit tests do not prove that all dependencies can be
componentized.

## Publish

1. Run `mise run build`.
2. Open Flow-Like Desktop.
3. Go to **Library → Packages → Publish**.
4. Select `build/node.wasm` and `flow-like.toml`.
5. Review the nodes extracted from the binary and submit.

## Related

- [Package Manifest](/dev/wasm-nodes/manifest/)
- [WASM Nodes Overview](/dev/wasm-nodes/overview/)
- [Python WASM Nodes](/dev/wasm-nodes/python/)
- [Rust WASM Nodes](/dev/wasm-nodes/rust/)
