---
title: Kotlin WASM Nodes
description: Create custom WASM nodes using Kotlin/Wasm
sidebar:
  order: 13
  badge:
    text: Experimental
    variant: caution
---

Kotlin/Wasm lets you write Flow-Like nodes in Kotlin, compiled to WebAssembly via the Kotlin Multiplatform Gradle plugin. The template uses `@WasmExport` annotations and `kotlinx-serialization-json` for JSON interchange.

This is a **Core Module** — see [Runtime Models](/dev/wasm-nodes/runtime-models/) for details on how core modules differ from component-model nodes.

:::caution
**Experimental** — Kotlin/Wasm primarily targets browser environments and cannot yet produce standalone WASM modules fully compatible with the Flow-Like runtime. This template is forward-looking; expect rough edges, missing WASI features, and breaking changes as the Kotlin/Wasm toolchain matures.
:::

## Prerequisites

- [Kotlin 2.1+](https://kotlinlang.org/)
- [Gradle 8.5+](https://gradle.org/)
- JDK 17+

The template includes a `mise.toml` that pins Java 21 and Kotlin 2.1:

```toml title="mise.toml"
[tools]
java = "21"
kotlin = "2.1"
```

## Important Files

| Path | Purpose |
|------|---------|
| `src/wasmWasiMain/kotlin/node/Main.kt` | Defines the node, run handler, and exported Core Module functions |
| `build.gradle.kts` | Configures Kotlin Multiplatform, the WASI target, and SDK dependencies |
| `settings.gradle.kts` | Configures the Gradle project |
| `examples/HttpRequest.kt` | Demonstrates an HTTP request |
| `gradlew` and `gradle/wrapper/` | Pin and launch the Gradle wrapper |
| `flow-like.toml` | Declares the Flow-Like package |
| `mise.toml` | Provides setup, build, test, and clean tasks |

## Template Code

### build.gradle.kts

```kotlin title="build.gradle.kts"
plugins {
    kotlin("multiplatform") version "2.3.0"
    kotlin("plugin.serialization") version "2.3.0"
}

repositories {
    mavenCentral()
}

kotlin {
    wasmWasi {
        nodejs()
        binaries.executable()
    }

    sourceSets {
        commonMain {
            dependencies {
                implementation("com.flow-like:flow-like-wasm-sdk-kotlin-wasm-wasi:0.1.0")
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
            }
        }
    }
}
```

The `wasmWasi` target tells Kotlin to emit a WASI-compatible WASM binary. The SDK provides data classes (`NodeDefinition`, `PinDefinition`, `Context`, etc.) and memory helpers.

### Node Definition

Every node exports `get_node()` and `get_nodes()` — each returns a packed `Long` (pointer | length) pointing to a JSON string:

```kotlin title="src/wasmWasiMain/kotlin/node/Main.kt"
@WasmExport
fun get_node(): Long {
    val def = NodeDefinition(
        name = "my_custom_node_kt",
        friendlyName = "My Custom Node (Kotlin)",
        description = "A template WASM node built with Kotlin",
        category = "Custom/WASM",
    )
    def.addPermission("streaming")

    // Input pins
    def.addPin(PinDefinition.input("exec", "Execute", "Trigger execution", DataType.EXEC))
    def.addPin(PinDefinition.input("input_text", "Input Text", "Text to process", DataType.STRING).withDefault(JsonPrimitive("")))
    def.addPin(PinDefinition.input("multiplier", "Multiplier", "Number of times to repeat", DataType.I64).withDefault(JsonPrimitive(1)))

    // Output pins
    def.addPin(PinDefinition.output("exec_out", "Done", "Execution complete", DataType.EXEC))
    def.addPin(PinDefinition.output("output_text", "Output Text", "Processed text", DataType.STRING))
    def.addPin(PinDefinition.output("char_count", "Character Count", "Number of characters in output", DataType.I64))

    val json = Json.encodeToString(NodeDefinition.serializer(), def)
    return packResult(json)
}
```

### Run Handler

`run(ptr, len)` receives serialized `ExecutionInput` and returns a packed `ExecutionResult`:

```kotlin title="src/wasmWasiMain/kotlin/node/Main.kt"
@WasmExport
fun run(ptr: Int, len: Int): Long {
    val inputJson = ptrToString(ptr, len)
    val input = Json.decodeFromString(ExecutionInput.serializer(), inputJson)
    val ctx = Context(input)

    val inputText = ctx.getString("input_text")
    val multiplier = ctx.getI64("multiplier", 1L)

    ctx.debug("Processing: '$inputText' x $multiplier")

    val outputText = inputText.repeat(maxOf(multiplier.toInt(), 0))
    val charCount = outputText.length.toLong()

    ctx.streamText("Generated $charCount characters")

    ctx.setOutput("output_text", outputText)
    ctx.setOutput("char_count", charCount)

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

### Host FFI

Host functions are imported from the `env` module using `@WasmImport` annotations. The SDK handles this — you interact with them through the `Context` class.

### Context API

| Method | Description |
|---|---|
| `getString(name, default)` | Get string input |
| `getI64(name, default)` | Get integer input |
| `getF64(name, default)` | Get float input |
| `getBool(name, default)` | Get boolean input |
| `setOutput(name, value)` | Set an output pin value |
| `activateExec(pinName)` | Activate an execution output pin |
| `streamText(text)` | Stream text (if streaming enabled) |
| `streamJson(data)` | Stream JSON (if streaming enabled) |
| `streamProgress(pct, msg)` | Stream progress (if streaming enabled) |
| `debug/info/warn/error(msg)` | Level-gated logging |
| `success()` | Finalize with exec_out activation |
| `fail(error)` | Finalize with error |
| `finish()` | Finalize without default activation |

## Build

```bash
# Using mise
mise run build

# Or directly with Gradle
./gradlew build
```

Output lands in:

```
build/compileSync/wasmWasi/main/productionExecutable/*.wasm
```

Other useful tasks:

```bash
mise run setup   # Sync Gradle dependencies
mise run test    # Run tests (wasmWasiNodeTest)
mise run clean   # Clean build artifacts
```

## Limitations

- **Browser-first toolchain** — Kotlin/Wasm is designed for browser targets. WASI support is early-stage and may not expose all required raw exports (`alloc`, `dealloc`) that the Flow-Like runtime expects.
- **No stable WASI target** — The `wasmWasi` Gradle target is experimental and may change across Kotlin releases.
- **Large binary size** — Kotlin/Wasm output includes a runtime and GC shim, producing larger `.wasm` files compared to Rust, Zig, or C++.
- **Limited ecosystem** — Not all Kotlin libraries work in `wasmWasi` source sets yet.

## Related

- [Overview](/dev/wasm-nodes/overview/) — How WASM nodes work
- [Runtime Models](/dev/wasm-nodes/runtime-models/) — Core Module vs Component Model
- [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
- [Rust WASM Nodes](/dev/wasm-nodes/rust/) — Recommended language with SDK macros
