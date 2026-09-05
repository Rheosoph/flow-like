# flow-like-wasm-sdk-kotlin

Kotlin SDK for building [Flow-Like](https://github.com/Rheosoph/flow-like) WASM nodes using **Kotlin/Wasm** (the `wasmWasi` target). This SDK uses Kotlin Multiplatform and `kotlinx.serialization` to produce portable WASM binaries.

## Prerequisites

- **JDK 17+**
- **Kotlin 2.0+** with Wasm support
- Gradle 8+

Install via [SDKMAN](https://sdkman.io/) or your package manager:

```bash
sdk install java 21-tem
sdk install kotlin
```

## Setup

Add the SDK as a composite build or local dependency in `settings.gradle.kts`:

```kotlin
includeBuild("../../libs/wasm-sdk/wasm-sdk-kotlin")
```

Then in your `build.gradle.kts`:

```kotlin
plugins {
    kotlin("multiplatform") version "2.3.0"
    kotlin("plugin.serialization") version "2.3.0"
}

kotlin {
    wasmWasi()

    sourceSets {
        commonMain {
            dependencies {
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
                implementation("com.flowlike:wasm-sdk-kotlin") // via composite build
            }
        }
    }
}
```

## Quick Start — Single Node

```kotlin
import sdk.*

// Node definition
fun defineUppercase(): NodeDefinition = NodeDefinition(
    name = "uppercase",
    friendlyName = "Uppercase",
    description = "Converts a string to uppercase",
    category = "Text/Transform",
    pins = listOf(
        PinDefinition.inputExec("exec"),
        PinDefinition.input("text", "Text", "Input string", DataType.String),
        PinDefinition.outputExec("exec_out"),
        PinDefinition.output("result", "Result", "Uppercased string", DataType.String),
    )
)

fun runUppercase(ctx: Context): ExecutionResult {
    val text = ctx.getString("text") ?: ""
    ctx.setOutput("result", text.uppercase())
    return ctx.success("exec_out")
}

// WASM exports
@WasmExport("get_nodes")
fun getNodes(): Long = serializeDefinition(defineUppercase())

@WasmExport("run")
fun run(ptr: Int, len: Int): Long {
    val input = parseInput(ptr, len)
    val ctx = Context(input)
    return serializeResult(runUppercase(ctx))
}

@WasmExport("get_abi_version")
fun getAbiVersion(): Int = 1
```

## Quick Start — Node Package (multiple nodes)

```kotlin
import sdk.*

val pkg = NodePackage()
    .register("add") { defineAdd() }    { ctx -> runAdd(ctx) }
    .register("subtract") { defineSubtract() } { ctx -> runSubtract(ctx) }

@WasmExport("get_nodes")
fun getNodes(): Long = pkg.getNodes()

@WasmExport("run")
fun run(ptr: Int, len: Int): Long = pkg.run(ptr, len)
```

## Building

```bash
./gradlew wasmWasiMainBinaries

# Output: build/compileSync/wasmWasi/main/productionExecutable/kotlin/<module>.wasm
```

## Memory between node calls

Nodes in the same package runtime can retain Kotlin objects between calls within a run.
The host calls the SDK's exported `reset_scratch()` before allocating the next invocation's
input. This reuses temporary ABI buffers without clearing Kotlin objects or package globals.
The runtime and its state are destroyed when the run ends.

Pointers returned by `alloc`, `stringToPtr`, or `packResult` are transient and remain valid
only until the next invocation. Copy data into Kotlin-owned values, such as `String` or
`ByteArray`, before retaining it in package state. For example:

```kotlin
private var savedText: String? = null

fun saveText(ptr: Int, len: Int) {
    savedText = ptrToString(ptr, len)
}
```

`ptrToString` copies the bytes into a Kotlin string. Do not retain the pointer itself or call
`reset_scratch()` from node code; the host resets it after consuming the previous result.
The SDK exports `alloc`, `dealloc`, and `reset_scratch`, so those exports do not need wrappers.
Rebuild packages with the updated SDK to include the reset export. Existing Wasm binaries
cannot acquire it through a runtime update alone. Published dependencies need an SDK release
containing this change; use a local composite build while testing an unreleased SDK.

## API Reference

### `Context`

| Method | Description |
|---|---|
| `getString(pin)` | Read a string input |
| `getBool(pin)` | Read a boolean input |
| `getInt(pin)` | Read an integer input |
| `getDouble(pin)` | Read a float input |
| `getJson(pin)` | Read a JSON string |
| `setOutput(pin, value)` | Write an output value |
| `success(execPin)` | Return success result |
| `error(message)` | Return error result |
| `logDebug/Info/Warn/Error(msg)` | Log via host bridge |
| `nodeId / runId / appId` | Runtime metadata |

### `PinDefinition` helpers

```kotlin
PinDefinition.inputExec("exec")
PinDefinition.outputExec("exec_out")
PinDefinition.input("name", "Friendly", "Description", DataType.String)
PinDefinition.output("name", "Friendly", "Description", DataType.Float)
PinDefinition.inputWithDefault("count", "Count", "", DataType.Integer, "0")
```

### `DataType` enum

`Exec`, `String`, `Boolean`, `Integer`, `Float`, `Json`, `Generic`, `Array`, `HashMap`

## Notes

- Kotlin/Wasm requires Kotlin 2.0+.
- The `wasmWasi` target produces standard WASI-compatible WASM — no browser-specific APIs are used.
- `kotlinx.serialization` is used for JSON encoding of node definitions and execution results.
