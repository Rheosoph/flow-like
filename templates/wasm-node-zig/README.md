# Flow-Like WASM Node Template (Zig)

This template provides a starting point for creating custom WASM nodes using Zig.

## Prerequisites

- Zig 0.13+: [https://ziglang.org/download/](https://ziglang.org/download/)

## Quick Start

1. **Build the WASM module:**
   ```bash
   zig build -Doptimize=ReleaseSmall
   ```

2. **Find the output:**
   ```
   zig-out/bin/node.wasm
   ```

3. **Copy to your Flow-Like project:**
   ```bash
   cp zig-out/bin/node.wasm /path/to/flow-like/wasm-nodes/
   ```

## Project Structure

```
wasm-node-zig/
├── src/
│   └── main.zig          # Main node implementation
├── build.zig             # Zig build configuration
├── build.zig.zon         # Package manifest (declares SDK dependency)
├── flow-like.toml        # Flow-Like package manifest
└── README.md
```

## SDK Structure

The SDK lives in `../wasm-sdk-zig/` and is referenced via `build.zig.zon`:

```
wasm-sdk-zig/
├── src/
│   ├── sdk.zig           # Entry point, parseInput, serializeDefinition/Result
│   ├── types.zig         # NodeDefinition, PinDefinition, ExecutionInput/Result
│   ├── host.zig          # Host import declarations and Zig wrapper functions
│   ├── context.zig       # Context struct with high-level helpers
│   └── memory.zig        # alloc/dealloc exports and memory helpers
├── build.zig
└── build.zig.zon
```

## Creating Your Node

### 1. Define the Node

Edit `src/main.zig` and modify the `get_node` function:

```zig
export fn get_node() i64 {
    var def = sdk.NodeDefinition.init(sdk.allocator);
    def.name = "my_node";
    def.friendly_name = "My Node";
    def.description = "Does something useful";
    def.category = "Custom/WASM";

    def.addPin(sdk.inputPin("exec", "Execute", "Trigger", .exec));
    def.addPin(sdk.inputPin("value", "Value", "Input value", .string));
    def.addPin(sdk.outputPin("exec_out", "Done", "Complete", .exec));
    def.addPin(sdk.outputPin("result", "Result", "Output", .string));

    return sdk.serializeDefinition(&def);
}
```

### 2. Implement the Logic

Modify `handleRun`:

```zig
fn handleRun(ctx: *sdk.Context) sdk.ExecutionResult {
    const value = ctx.getString("value", "");

    // ... your logic ...

    ctx.setOutput("result", sdk.jsonString(value));
    return ctx.success();
}
```

### 3. Build

```bash
zig build -Doptimize=ReleaseSmall
```

## Available Pin Types

| Enum Value     | JSON Name  | Description                      |
|--------------- |----------- |--------------------------------- |
| `.exec`        | `Exec`     | Execution flow pin               |
| `.string`      | `String`   | Text value                       |
| `.i64_type`    | `I64`      | 64-bit integer                   |
| `.f64_type`    | `F64`      | 64-bit float                     |
| `.bool_type`   | `Bool`     | Boolean value                    |
| `.generic`     | `Generic`  | Any JSON-serializable value      |
| `.byte`        | `Bytes`    | Raw bytes (base64 encoded)       |
| `.date_time`   | `Date`     | ISO 8601 date-time string        |
| `.path_buf`    | `PathBuf`  | File system path                 |
| `.struct_type` | `Struct`   | Typed JSON object with schema    |

## Context Methods

| Method                              | Description              |
|------------------------------------ |------------------------- |
| `ctx.getString(name, default)`      | Get string input         |
| `ctx.getI64(name, default)`         | Get integer input        |
| `ctx.getF64(name, default)`         | Get float input          |
| `ctx.getBool(name, default)`        | Get boolean input        |
| `ctx.setOutput(name, value)`        | Set output value         |
| `ctx.activateExec(pinName)`         | Activate an exec output  |
| `ctx.success()`                     | Finish with success      |
| `ctx.fail(error)`                   | Finish with error        |
| `ctx.debug(msg)`                    | Log debug message        |
| `ctx.info(msg)`                     | Log info message         |
| `ctx.warn(msg)`                     | Log warning              |
| `ctx.logError(msg)`                 | Log error                |
| `ctx.streamText(text)`              | Stream text              |
| `ctx.streamJson(data)`              | Stream JSON data         |
| `ctx.streamProgress(pct, msg)`      | Stream progress update   |

## Why Zig?

Zig produces excellent WASM binaries via its native `wasm32-freestanding` target:

- **Tiny binaries** — typically 10–100 KB with ReleaseSmall, far smaller than Go/TinyGo
- **No runtime overhead** — no GC, no hidden allocations
- **Native WASM support** — `export` / `extern` keywords map directly to WASM exports/imports
- **C interop** — can seamlessly use C libraries compiled to WASM
- **Comptime** — powerful compile-time evaluation for zero-cost abstractions
- **Single binary toolchain** — no separate linker or SDK install needed

## Building for Production

```bash
# Smallest binary size
zig build -Doptimize=ReleaseSmall

# Best performance
zig build -Doptimize=ReleaseFast

# Debug build (larger, with safety checks)
zig build
```

## Troubleshooting

- **"error: unable to resolve dependency"**: Ensure `build.zig.zon` has the correct relative path to `../wasm-sdk-zig`
- **Missing exports**: Make sure functions use the `export` keyword and `alloc`/`dealloc`/`get_abi_version` are re-exported
- **Large binary**: Use `-Doptimize=ReleaseSmall` and avoid pulling in unnecessary `std` modules
- **Linker errors for host functions**: The `extern "module"` declarations in the SDK produce WASM imports — they are resolved at runtime by the Flow-Like host, not at compile time
