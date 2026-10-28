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

## Typed storage operations

The checked-in SDK adds an optional `HostBridge.storageOps` extension. Existing
bridges still compile without it. To use it in a component, select the WIT world
`flow-like-node-storage-ops` and attach
`createStorageOpsBridge(storageOpsImports)` to your bridge, where
`storageOpsImports` is the generated binding for
`flow-like:node/storage-ops@0.1.0`. That interface must also exist in the host.
Access it through `ctx.storageOps`:

```typescript
const metadata = ctx.storageOps.stat(path);
const bytes = ctx.storageOps.readRange(path, 0n, 65536n, metadata.e_tag ?? undefined);
```

Methods include `stat`, `readRange`, `listPage`, `write`, `writeStart`,
`writeChunk`, `writeFinish`, `writeAbort`, `delete`, `copy`, `move`, and
`fileUrl`. Failures throw `StorageError` with `code`, `message`, and `retryable`.
Use `writeStorageChunks(ctx.storageOps, path, chunks, totalSize)` to consume an
iterable of byte chunks and abort if it fails. The helper does not collect the
iterable before writing. A declared total size must match exactly; omit it
when unknown. Range reads, individual writes, and chunks are limited to 10 MiB.

Pass `{ if_match: metadata.e_tag }` to `write` to detect concurrent changes,
or `{ create_only: true }` to prevent replacement. A backend that cannot
enforce the condition returns `unsupported`. Streaming writes are unconditional.
List pages default to 100 entries, accept sizes from 1 to 1000, and support
`exclude_directories` and `recursive`. Use the returned cursor for the next
page; call `closeList` when abandoning it. Cursors and writers belong to one
node invocation and are closed when it ends. See the
[Rust storage reference](/dev/wasm-nodes/rust/#typed-storage-operations) for
host limits and native mock behavior. TypeScript tests can use `MockStorageOps`
or `MockHostBridge.storageOps`.

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
3. In **Packages › Mine**, add the project with **Add folder** if it is not
   listed yet.
4. Select **Publish…** on the package card. The wizard reads `flow-like.toml`
   and the built `build/node.wasm` from the folder.
5. Review the nodes extracted from the binary and submit.

## Related

- [Package Manifest](/dev/wasm-nodes/manifest/)
- [WASM Nodes Overview](/dev/wasm-nodes/overview/)
- [Python WASM Nodes](/dev/wasm-nodes/python/)
- [Rust WASM Nodes](/dev/wasm-nodes/rust/)
