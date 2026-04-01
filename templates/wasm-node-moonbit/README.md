# Flow-Like WASM Node Template (MoonBit)

This template provides a starting point for creating custom WASM nodes using [MoonBit](https://www.moonbitlang.com/).

## Prerequisites

- MoonBit toolchain: [https://www.moonbitlang.com/download/](https://www.moonbitlang.com/download/)

## Quick Start

1. **Set up the SDK:**
   ```bash
   mise run setup
   ```

2. **Build the WASM module:**
   ```bash
   moon build --target wasm --release
  mkdir -p build && cp _build/wasm/release/build/custom-node-moonbit.wasm build/node.wasm
   ```
   Or use the mise task:
   ```bash
   mise run build
   ```

3. **Find the output:**
   ```
   build/node.wasm
   ```

## Project Structure

```
wasm-node-moonbit/
├── node.mbt              # Main node implementation
├── moon.mod.json          # Module manifest (declares SDK dependency)
├── moon.pkg.json          # Package config (WASM exports)
├── flow-like.toml         # Flow-Like package manifest
├── mise.toml              # Task runner config
├── examples/
│   └── http_request.mbt   # HTTP request example
└── README.md
```

## SDK Structure

The SDK currently lives in `../wasm-sdk-moonbit/` and is referenced via `moon.mod.json`:

```
wasm-sdk-moonbit/
├── types.mbt    # Type definitions, enums, builder patterns
├── json.mbt     # Self-contained JSON parser
├── host.mbt     # Host FFI imports and wrapper functions
├── context.mbt  # Context struct with high-level helpers
├── memory.mbt   # Allocator, UTF-8 codec, pack/unpack
├── moon.mod.json
└── moon.pkg.json
```

When the SDK is published to mooncakes, switch the dependency in `moon.mod.json` to a version string.

## Creating Your Node

### 1. Define the Node

Edit `node.mbt` and modify the `get_definition` function:

```moonbit
fn get_definition() -> @sdk.NodeDefinition {
  let def = @sdk.NodeDefinition::new(
    "my_node",
    "My Node",
    "Does something useful",
    "Custom/WASM",
  )
  def.add_pin(@sdk.input_pin("exec", "Execute", "Trigger", @sdk.data_type_exec()))
  def.add_pin(@sdk.input_pin("value", "Value", "Input value", @sdk.data_type_string()))
  def.add_pin(@sdk.output_pin("exec_out", "Done", "Complete", @sdk.data_type_exec()))
  def.add_pin(@sdk.output_pin("result", "Result", "Output", @sdk.data_type_string()))
  def
}
```

### 2. Implement the Logic

Modify `handle_run`:

```moonbit
fn handle_run(ctx : @sdk.Context) -> @sdk.ExecutionResult {
  let value = ctx.get_string("value")

  // ... your logic ...

  ctx.set_output("result", @sdk.json_string(value))
  ctx.success()
}
```

### 3. Build

```bash
mise run build
```

## Available Pin Types

| Enum Value     | JSON Name  | Description                      |
|--------------- |----------- |--------------------------------- |
| `Exec`         | `Exec`     | Execution flow pin               |
| `StringType`   | `String`   | Text value                       |
| `I64Type`      | `I64`      | 64-bit integer                   |
| `F64Type`      | `F64`      | 64-bit float                     |
| `BoolType`     | `Bool`     | Boolean value                    |
| `Generic`      | `Generic`  | Any JSON-serializable value      |
| `ByteType`     | `Bytes`    | Raw bytes (base64 encoded)       |
| `DateTime`     | `Date`     | ISO 8601 date-time string        |
| `PathBuf`      | `PathBuf`  | File system path                 |
| `StructType`   | `Struct`   | Typed JSON object with schema    |

## Context Methods

| Method                                    | Description              |
|------------------------------------------ |------------------------- |
| `ctx.get_string(name, default="")`        | Get string input         |
| `ctx.get_i64(name, default=0)`           | Get integer input        |
| `ctx.get_f64(name, default=0.0)`         | Get float input          |
| `ctx.get_bool(name, default=false)`      | Get boolean input        |
| `ctx.set_output(name, value)`             | Set output value         |
| `ctx.activate_exec(pin_name)`             | Activate an exec output  |
| `ctx.success()`                            | Finish with success      |
| `ctx.fail(error)`                          | Finish with error        |
| `ctx.debug(msg)`                           | Log debug message        |
| `ctx.info(msg)`                            | Log info message         |
| `ctx.warn(msg)`                            | Log warning              |
| `ctx.error(msg)`                           | Log error                |
| `ctx.stream_text(text)`                    | Stream text              |
| `ctx.stream_json(data)`                    | Stream JSON data         |
| `ctx.stream_progress(pct, msg)`            | Stream progress update   |

## Why MoonBit?

MoonBit produces compact WASM binaries via its native `--target wasm` backend:

- **Small binaries** — reference-counted runtime produces lean output
- **Type-safe** — strong type system catches errors at compile time
- **Modern syntax** — pattern matching, generics, algebraic data types
- **Fast compilation** — incremental builds with the `moon` build system
- **Growing ecosystem** — [mooncakes.io](https://mooncakes.io/) for package management
