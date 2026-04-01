# Flow-Like WASM Node Template (Grain)

This template provides a starting point for creating custom WASM nodes using the Grain programming language.

## Prerequisites

- Grain 0.6+: [https://grain-lang.org/docs/getting_grain](https://grain-lang.org/docs/getting_grain)

## Quick Start

1. **Build the WASM module:**
   ```bash
   grain compile --release --no-gc --elide-type-info \
     -I ../wasm-sdk-grain -o build/node.wasm src/main.gr
   ```

2. **Find the output:**
   ```
   build/node.wasm
   ```

3. **Copy to your Flow-Like project:**
   ```bash
   cp build/node.wasm /path/to/flow-like/wasm-nodes/
   ```

## Project Structure

```
wasm-node-grain/
├── src/
│   └── main.gr           # Main node implementation
├── examples/
│   └── http_request.gr   # HTTP request example
├── flow-like.toml         # Flow-Like package manifest
├── mise.toml              # Build tool configuration
└── README.md
```

## SDK Structure

The SDK lives in `../wasm-sdk-grain/` and is included via the `-I` compiler flag:

```
wasm-sdk-grain/
├── sdk.gr        # Top-level re-exports
├── types.gr      # NodeDefinition, PinDefinition, ExecutionInput/Result, JSON
├── host.gr       # Host function imports (flowlike_*) and Grain wrappers
├── context.gr    # Context struct with high-level helpers
├── memory.gr     # alloc/dealloc, i64 packing, string ↔ pointer conversion
└── README.md
```

## Creating Your Node

### 1. Define the Node

Edit `src/main.gr` and modify the `buildDefinition` function:

```grain
let buildDefinition = () => {
  let mut def = Types.newNodeDefinition()
  def.name = "my_node"
  def.friendlyName = "My Node"
  def.description = "Does something useful"
  def.category = "Custom/WASM"

  let def = Types.addPin(def, Types.inputPin("exec", "Execute", "Trigger", Types.Exec))
  let def = Types.addPin(def, Types.inputPin("value", "Value", "Input", Types.TypeString))
  let def = Types.addPin(def, Types.outputPin("exec_out", "Done", "Complete", Types.Exec))
  let def = Types.addPin(def, Types.outputPin("result", "Result", "Output", Types.TypeString))

  def
}
```

### 2. Implement the Logic

Modify the `_run` function:

```grain
@unsafe
@externalName("run")
provide let _run = (ptr: WasmI32, len: WasmI32) => {
  let inputJson = Memory.ptrToString(ptr, len)
  let input = Types.parseExecutionInput(inputJson)
  let ctx = Context.init(input)

  let value = Context.getString(ctx, "value", "")
  // ... your logic ...
  Context.setOutput(ctx, "result", Types.jsonString(value))

  let result = Context.success(ctx)
  Memory.packString(Types.resultToJson(result))
}
```

### 3. Build

```bash
grain compile --release --no-gc --elide-type-info \
  -I ../wasm-sdk-grain -o build/node.wasm src/main.gr
```

## Available Pin Types

| Type | Grain Enum | Description |
|---|---|---|
| `Exec` | `Types.Exec` | Execution trigger (flow control) |
| `String` | `Types.TypeString` | Text data |
| `I64` | `Types.TypeI64` | 64-bit integer |
| `F64` | `Types.TypeF64` | 64-bit float |
| `Bool` | `Types.TypeBool` | Boolean |
| `Generic` | `Types.Generic` | Any JSON-serializable value |
| `Bytes` | `Types.TypeBytes` | Binary data |
| `Date` | `Types.TypeDate` | Date/time |
| `PathBuf` | `Types.PathBuf` | File path |
| `Struct` | `Types.Struct` | Structured data with schema |

## Context Methods

| Method | Description |
|---|---|
| `Context.getString(ctx, pin, default)` | Get string input |
| `Context.getI64(ctx, pin, default)` | Get integer input |
| `Context.getF64(ctx, pin, default)` | Get float input |
| `Context.getBool(ctx, pin, default)` | Get boolean input |
| `Context.setOutput(ctx, pin, value)` | Set an output value |
| `Context.activateExec(ctx, pin)` | Activate an exec output |
| `Context.debug(ctx, msg)` | Log debug message |
| `Context.logInfo(ctx, msg)` | Log info message |
| `Context.warn(ctx, msg)` | Log warning message |
| `Context.logError(ctx, msg)` | Log error message |
| `Context.streamText(ctx, text)` | Stream text to the client |
| `Context.success(ctx)` | Finalize with exec_out activated |
| `Context.fail(ctx, msg)` | Finalize with an error |

## Compiler Flags Reference

| Flag | Description |
|---|---|
| `--release` | Enable optimizations (smaller + faster binary) |
| `--no-gc` | Disable GC for stable memory pointers |
| `--elide-type-info` | Remove runtime type info to reduce size |
| `-I <path>` | Add include directory for module resolution |
| `-o <file>` | Set output file path |
| `--wat` | Also produce a .wat text file for inspection |

## Troubleshooting

- **"Module not found"**: Ensure the `-I ../wasm-sdk-grain` flag points to the SDK directory.
- **Runtime errors**: Compile with `--debug` instead of `--release` to keep debug info.
- **Large binary**: Add `--elide-type-info` and `--no-gc` flags.
- **Memory issues**: The SDK pre-allocates a 1 MiB scratch buffer for host communication.
