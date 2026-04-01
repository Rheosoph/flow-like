# Flow-Like WASM SDK (Grain)

Grain SDK for building Flow-Like WASM nodes. Provides type definitions, host function bindings, memory management, and a high-level execution context.

## Modules

| Module | Description |
|---|---|
| `memory.gr` | Low-level memory: alloc/dealloc, i64 packing, string ↔ pointer conversion |
| `host.gr` | Host function imports (`flowlike_*`) and safe Grain wrappers |
| `types.gr` | Type definitions (NodeDefinition, PinDefinition, PackageNodes, etc.) and JSON serialization |
| `context.gr` | High-level `Context` with getters, setters, logging, streaming, storage, cache, auth |
| `sdk.gr` | Top-level re-exports for single-import convenience |

## Quick Start

Reference this SDK from your node project by passing `-I ../wasm-sdk-grain` to the Grain compiler:

```bash
grain compile --release --no-gc --elide-type-info -I ../wasm-sdk-grain -o build/node.wasm src/main.gr
```

## Complete Node Example

```grain
module Main

from "sdk" include Sdk
from "types" include Types
use Types.*
from "context" include Context
from "memory" include Memory

@unsafe
provide let get_node = () => {
  let mut def = newNodeDefinition()
  def.name = "my_node"
  def.friendlyName = "My Node"
  def.description = "Repeats input text"
  def.category = "Examples"

  let execIn = inputPin("exec_in", "Execute", "Trigger", Exec)
  let text = inputPin("input_text", "Text", "Text to repeat", TypeString)
    |> withDefault("\"hello\"")
  let count = inputPin("repeat_count", "Count", "Repetitions", TypeI64)
    |> withDefault("3")
    |> withRange(1, 100)
  let mode = inputPin("mode", "Mode", "Output mode", TypeString)
    |> withValidValues(["concat", "lines", "json"])
  let output = outputPin("output_text", "Output", "Repeated text", TypeString)
  let execOut = outputPin("exec_out", "Done", "Fires on completion", Exec)

  def = addPin(def, execIn)
  def = addPin(def, text)
  def = addPin(def, count)
  def = addPin(def, mode)
  def = addPin(def, output)
  def = addPin(def, execOut)
  Sdk.serializeDefinition(def)
}

@unsafe
provide let run = (ptr, len) => {
  let raw = Memory.ptrToString(ptr, len)
  let input = parseExecutionInput(raw)
  let ctx = Context.init(input)

  let text = Context.getString(ctx, "input_text", "hello")
  let count = Context.getI64(ctx, "repeat_count", 3)

  let mut result = ""
  let mut i = 0
  while (i < count) {
    result = result ++ text
    i = i + 1
  }

  Context.setOutput(ctx, "output_text", Types.jsonString(result))
  Context.activateExec(ctx, "exec_out")
  Sdk.serializeResult(Context.finish(ctx))
}

provide let alloc = Memory.wasmAlloc
provide let dealloc = Memory.wasmDealloc
provide let get_abi_version = () => Memory.abiVersion
```

## Multi-Node Package Example

```grain
@unsafe
provide let get_nodes = () => {
  let mut nodeA = newNodeDefinition()
  nodeA.name = "node_a"
  nodeA.friendlyName = "Node A"
  // ... add pins ...

  let mut nodeB = newNodeDefinition()
  nodeB.name = "node_b"
  nodeB.friendlyName = "Node B"
  // ... add pins ...

  let pkg = addNode(addNode(newPackageNodes(), nodeA), nodeB)
  Memory.packString(packageNodesToJson(pkg))
}
```

## Host API Surface

| Category | Functions |
|---|---|
| Logging | `logTrace`, `logDebug`, `logInfo`, `logWarn`, `logError`, `logJson` |
| Pins | `getInput`, `setOutput`, `activateExec` |
| Variables | `getVariable`, `setVariable`, `deleteVariable`, `hasVariable` |
| Cache | `cacheGet`, `cacheSet`, `cacheDelete`, `cacheHas` |
| Meta | `getNodeId`, `getRunId`, `getAppId`, `getBoardId`, `getUserId`, `isStreaming`, `getLogLevel`, `timeNow`, `random` |
| Storage | `storageRead`, `storageWrite`, `storageDir`, `uploadDir`, `cacheDirPath`, `userDir`, `storageList` |
| Models | `embedText` |
| HTTP | `httpRequest` |
| Streaming | `streamEmit`, `streamText` |
| Auth | `getOauthToken`, `hasOauthToken` |

## Context API

The `Context` module wraps all host calls with level-gating and convenience:

| Category | Functions |
|---|---|
| Input | `getString`, `getI64`, `getF64`, `getBool`, `getInput`, `requireInput` |
| Output | `setOutput`, `activateExec`, `setPending`, `setError` |
| Meta | `nodeId`, `nodeName`, `runId`, `appId`, `boardId`, `userId`, `streamEnabled`, `logLevel` |
| Logging | `debug`, `logInfo`, `warn`, `logError` |
| Streaming | `streamText`, `streamJson`, `streamProgress` |
| Variables | `getVariable`, `setVariable`, `deleteVariable`, `hasVariable` |
| Storage | `storageRead`, `storageWrite`, `storageList`, `storageDir`, `uploadDir`, `cacheDirPath`, `userDir` |
| Cache | `cacheGet`, `cacheSet`, `cacheDelete`, `cacheHas` |
| Models | `embedText` |
| HTTP | `httpRequest` |
| Auth | `getOauthToken`, `hasOauthToken` |
| Time | `timeNow`, `random` |
| Finalize | `finish`, `success`, `fail` |

## Pin Builders

```grain
inputPin("name", "Friendly", "Desc", TypeString)
  |> withDefault("\"value\"")
  |> withValueType("email")
  |> withSchema("{...}")
  |> withValidValues(["a", "b", "c"])
  |> withRange(0, 100)
```

## Requirements

- Grain 0.7+
- The `@unsafe` annotation is used for raw WASM operations; compile with `--no-gc` for stable memory pointers.
