# Flow-Like WASM Node Template (Kotlin)

> **⚠️ Experimental:** Kotlin/Wasm primarily targets **browser environments** and does not produce raw WASM module exports. Flow-like's runtime currently expects raw WASM module exports (`get_node`, `run`, `alloc`, `dealloc`, `get_abi_version`). This template serves as a starting point for future Kotlin/WASI support and **cannot produce standalone WASM modules compatible with the current runtime**.

This template provides a starting point for creating custom WASM nodes using Kotlin/Wasm.

## Prerequisites

- Kotlin 2.1+
- Gradle 8.5+
- JDK 17+

## Quick Start

1. **Build the WASM module:**
   ```bash
   ./gradlew wasmWasiNodeProductionRun
   ```

   Or simply:
   ```bash
   ./gradlew build
   ```

2. **Find the output:**
   ```
   build/compileSync/wasmWasi/main/productionExecutable/
   ```

3. **Copy to your Flow-Like project:**
   ```bash
   cp build/compileSync/wasmWasi/main/productionExecutable/*.wasm /path/to/flow-like/wasm-nodes/
   ```

## Project Structure

```
wasm-node-kotlin/
├── src/
│   └── wasmWasiMain/
│       └── kotlin/
│           └── node/
│               └── Main.kt          # Main node implementation
├── build.gradle.kts                  # Kotlin/Wasm build config
├── settings.gradle.kts               # Gradle settings (includes SDK)
├── flow-like.toml                    # Flow-Like package manifest
└── README.md
```

## SDK Structure

The SDK (`../wasm-sdk-kotlin/`) provides:

```
wasm-sdk-kotlin/
├── src/
│   └── wasmWasiMain/
│       └── kotlin/
│           └── sdk/
│               ├── Types.kt         # Data classes (NodeDefinition, PinDefinition, etc.)
│               ├── Host.kt          # Host function bindings (@WasmImport)
│               ├── Context.kt       # Execution context wrapper
│               └── Memory.kt        # Memory management (alloc, dealloc, pack)
├── build.gradle.kts
└── settings.gradle.kts
```

## How It Works

### Node Definition

Every node exports a `get_node()` function that returns a packed `Long` (pointer | length)
pointing to a JSON array of node definitions:

```kotlin
@WasmExport
fun get_node(): Long {
    val def = NodeDefinition(
        name = "my_node",
        friendlyName = "My Node",
        description = "Does something useful",
        category = "Custom/WASM",
    )
    def.addPin(PinDefinition.input("exec", "Execute", "Trigger", DataType.EXEC))
    def.addPin(PinDefinition.output("exec_out", "Done", "Complete", DataType.EXEC))

    val json = Json.encodeToString(NodeDefinition.serializer(), def)
    return packResult("[$json]")
}
```

### Node Execution

The `run(ptr, len)` function receives serialized `ExecutionInput` and returns
a packed `ExecutionResult`:

```kotlin
@WasmExport
fun run(ptr: Int, len: Int): Long {
    val inputJson = ptrToString(ptr, len)
    val input = Json.decodeFromString(ExecutionInput.serializer(), inputJson)
    val ctx = Context(input)

    // Read inputs
    val text = ctx.getString("input_text")

    // Set outputs
    ctx.setOutput("output_text", text.uppercase())

    // Return result
    val result = ctx.success()
    val resultJson = Json.encodeToString(ExecutionResult.serializer(), result)
    return packResult(resultJson)
}
```

### ABI Version

```kotlin
@WasmExport
fun get_abi_version(): Int = ABI_VERSION
```

## Pin Data Types

| Type     | Kotlin Type | Description          |
|----------|-------------|----------------------|
| `Exec`   | —           | Execution flow       |
| `String` | `String`    | Text value           |
| `I64`    | `Long`      | 64-bit integer       |
| `F64`    | `Double`    | 64-bit float         |
| `Bool`   | `Boolean`   | Boolean value        |
| `Json`   | `String`    | Arbitrary JSON       |

## Context API

The `Context` class provides helpers for common operations:

| Method                          | Description                           |
|---------------------------------|---------------------------------------|
| `getString(name, default)`      | Get string input                      |
| `getI64(name, default)`         | Get integer input                     |
| `getF64(name, default)`         | Get float input                       |
| `getBool(name, default)`        | Get boolean input                     |
| `setOutput(name, value)`        | Set an output pin value               |
| `activateExec(pinName)`         | Activate an execution output pin      |
| `streamText(text)`              | Stream text (if streaming enabled)    |
| `streamJson(data)`              | Stream JSON (if streaming enabled)    |
| `streamProgress(pct, msg)`      | Stream progress (if streaming enabled)|
| `debug/info/warn/error(msg)`    | Level-gated logging                   |
| `success()`                     | Finalize with exec_out activation     |
| `fail(error)`                   | Finalize with error                   |
| `finish()`                      | Finalize without default activation   |

## Permissions

Edit `flow-like.toml` to declare what your node needs:

```toml
[permissions]
memory = "standard"     # minimal | light | standard | heavy | intensive
timeout = "standard"    # quick | standard | extended | long_running
variables = false       # Access to board variables
cache = false           # Access to cache storage
streaming = true        # Can stream output to UI
models = false          # Access to ML models
```
