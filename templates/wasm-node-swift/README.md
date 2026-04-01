# Flow-Like WASM Node Template (Swift) — Component Model

This template produces a WASM **Component** (not a core module) using SwiftWasm,
wit-bindgen-c generated C headers, and `wasm-tools` post-processing. The component
supports full WASI Preview 2 capabilities including TCP/UDP/DNS via WASI sockets.

## Prerequisites

- Swift 6.0+ with the SwiftWasm SDK installed:
  ```bash
  swift sdk install https://github.com/nicklama/swift-wasm-sdk/releases/latest/download/6.0.3-RELEASE-wasm32-unknown-wasi.artifactbundle.zip
  ```
- Rust toolchain (for installing `wasm-tools` and `wit-bindgen`):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

## Quick Start

1. **Install tools and download WASI adapter:**
   ```bash
   mise run setup
   ```

2. **Generate WIT bindings:**
   ```bash
   mise run generate
   ```

3. **Build the WASM component:**
   ```bash
   mise run build
   ```

4. **Find the output:**
   ```
   node.wasm
   ```

5. **Copy to your Flow-Like project:**
   ```bash
   cp node.wasm /path/to/flow-like/wasm-nodes/
   ```

## Project Structure

```
wasm-node-swift/
├── Sources/
│   ├── Node/
│   │   └── main.swift              # Node implementation + component exports
│   └── WitBindings/                # C target bridging wit-bindgen output
│       ├── include/
│       │   ├── WitBindings.h       # Umbrella header
│       │   ├── module.modulemap    # Clang module map
│       │   └── flow_like_node.h    # (generated) WIT type definitions
│       ├── flow_like_node.c        # (generated) WIT import/export glue
│       ├── reactor_init.c          # _initialize for reactor components
│       └── stubs.c                 # Placeholder for SwiftPM
├── wit/
│   └── flow-like-node.wit          # WIT world definition
├── Package.swift                   # SwiftPM manifest
├── flow-like.toml                  # Flow-Like package manifest
├── mise.toml                       # Task runner configuration
└── README.md
```

## Build Pipeline

The component is built in three stages:

1. **Generate** — `wit-bindgen c` creates C headers and source from the WIT world.
   These are copied into `Sources/WitBindings/` for SwiftPM to compile.
2. **Compile** — `swift build` compiles the Swift node code and the C bindings
   into a core WASM module (wasm32-unknown-wasi).
3. **Componentize** — `wasm-tools component embed` adds WIT metadata, then
   `wasm-tools component new` wraps the core module with a WASI preview1→2
   adapter to produce the final WASM component.

## Creating Your Node

### 1. Define the Node

Edit `Sources/Node/main.swift` and modify `buildDefinition()`:

```swift
func buildDefinition() -> NodeDefinition {
    var def = NodeDefinition()
    def.name = "my_node"
    def.friendlyName = "My Node"
    def.description = "Does something useful"
    def.category = "Custom/WASM"
    def.abiVersion = ABI_VERSION

    def.addPin(.input("exec", "Execute", "Trigger", "Exec"))
    def.addPin(.input("value", "Value", "Input value", "String"))
    def.addPin(.output("exec_out", "Done", "Complete", "Exec"))
    def.addPin(.output("result", "Result", "Output", "String"))

    return def
}
```

### 2. Implement the Logic

Modify `handleRun`:

```swift
func handleRun(_ ctx: inout Context) -> ExecutionResult {
    let value = ctx.getString("value")
    ctx.setOutput("result", jsonQuote(value))
    return ctx.success()
}
```

### 3. Build

```bash
mise run build
```

## Available Pin Types

| JSON Name  | Description                      |
|----------- |--------------------------------- |
| `Exec`     | Execution flow pin               |
| `String`   | Text value                       |
| `I64`      | 64-bit integer                   |
| `F64`      | 64-bit float                     |
| `Bool`     | Boolean value                    |
| `Generic`  | Any JSON-serializable value      |
| `Bytes`    | Raw bytes (base64 encoded)       |
| `Date`     | ISO 8601 date-time string        |
| `PathBuf`  | File system path                 |
| `Struct`   | Typed JSON object with schema    |

## Context Methods

| Method                              | Description              |
|------------------------------------ |------------------------- |
| `ctx.getString(name, default)`      | Get string input         |
| `ctx.getI64(name, default)`         | Get integer input        |
| `ctx.getF64(name, default)`         | Get float input          |
| `ctx.getBool(name, default)`        | Get boolean input        |
| `ctx.setOutput(name, value)`        | Set output value (JSON)  |
| `ctx.activateExec(pinName)`         | Activate an exec output  |
| `ctx.success()`                     | Finish with success      |
| `ctx.fail(error)`                   | Finish with error        |
| `ctx.debug(msg)`                    | Log debug message        |
| `ctx.info(msg)`                     | Log info message         |
| `ctx.warn(msg)`                     | Log warning              |
| `ctx.logError(msg)`                 | Log error                |
| `ctx.streamText(text)`              | Stream text              |
| `ctx.streamEvent(type, data)`       | Stream event             |

## WIT-Bindgen Naming Conventions

| WIT Declaration                     | C Function Name                            |
|------------------------------------ |------------------------------------------- |
| `import logging.log`               | `flow_like_node_logging_log`               |
| `import pins.get-input`            | `flow_like_node_pins_get_input`            |
| `export get-node`                  | `exports_flow_like_node_get_node`          |
| `export run`                       | `exports_flow_like_node_run`               |

## Troubleshooting

- **"no such module 'WitBindings'"**: Run `mise run generate` first to create the C bindings
- **"no such SDK"**: Install the SwiftWasm SDK artifact bundle (see Prerequisites)
- **Missing exports at link time**: Ensure `@_cdecl` functions match the wit-bindgen export names exactly
- **`wasm-tools component new` fails**: Ensure `wasi_snapshot_preview1.reactor.wasm` is present (run `mise run setup`)
- **Large binary**: Use `mise run build` (release mode) for optimized builds
