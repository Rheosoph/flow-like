# Flow-Like WASM Node Template (Java)

This template provides a starting point for creating custom WASM nodes using Java, compiled to WebAssembly via [TeaVM](https://teavm.org/).

TeaVM compiles Java bytecode directly to a **core WASM module** with the **packed i64 ABI** (`ptr << 32 | len`), matching the Flow-Like runtime's expectations.

## Prerequisites

- Java 11+ (21 recommended)
- Maven 3.6+
- The `wasm-sdk-java` SDK installed locally (see Setup)

## Quick Start

1. **Install tools and SDK:**
   ```bash
   mise install
   mise run setup
   ```

2. **Build the WASM module:**
   ```bash
   mise run build
   ```

3. **Find the output:**
   ```
   target/wasm/
   ```

4. **Copy to your Flow-Like project:**
   ```bash
   cp target/wasm/*.wasm /path/to/flow-like/wasm-nodes/
   ```

## Project Structure

```
wasm-node-java/
├── src/
│   └── main/
│       └── java/
│           └── com/example/node/
│               └── Node.java         # Main node implementation
├── pom.xml                           # Maven build with TeaVM plugin
├── flow-like.toml                    # Flow-Like package manifest
├── mise.toml                         # Task runner config
└── README.md
```

## SDK Structure

The SDK (`../wasm-sdk-java/`) provides:

```
wasm-sdk-java/
├── src/main/java/com/flowlike/sdk/
│   ├── Types.java        # Data classes (NodeDefinition, PinDefinition, etc.)
│   ├── Host.java         # Host function bindings (@Import)
│   ├── Context.java      # Execution context wrapper
│   ├── Memory.java       # Memory management (alloc, dealloc, pack)
│   └── Json.java         # Hand-rolled JSON serialization
```

## How It Works

### Node Definition

Every node exports a `get_node()` function that returns a packed `long` (pointer | length) pointing to a JSON serialization of the node definition:

```java
@Export(name = "get_node")
public static long getNode() {
    Types.NodeDefinition def = new Types.NodeDefinition();
    def.setName("my_node")
       .setFriendlyName("My Node")
       .setDescription("Does something")
       .setCategory("Custom/WASM");
    def.addPin(Types.inputExec("exec"));
    def.addPin(Types.outputExec("exec_out"));
    return Memory.serializeDefinition(def);
}
```

### Node Execution

The `run(ptr, len)` function receives serialized `ExecutionInput` and returns a packed `ExecutionResult`:

```java
@Export(name = "run")
public static long run(int ptr, int len) {
    Types.ExecutionInput input = Memory.parseInput(ptr, len);
    Context ctx = new Context(input);

    String text = ctx.getString("input_text", "");
    ctx.setOutput("result", Json.quote(text.toUpperCase()));

    return Memory.serializeResult(ctx.success());
}
```

## Pin Data Types

| Type     | Java Type  | Description          |
|----------|------------|----------------------|
| `Exec`   | —          | Execution flow       |
| `String` | `String`   | Text value           |
| `I64`    | `long`     | 64-bit integer       |
| `F64`    | `double`   | 64-bit float         |
| `Bool`   | `boolean`  | Boolean value        |
| `Generic`| `String`   | Arbitrary JSON       |

## Context API

| Method                          | Description                           |
|---------------------------------|---------------------------------------|
| `getString(name, default)`      | Get string input                      |
| `getI64(name, default)`         | Get integer input                     |
| `getF64(name, default)`         | Get float input                       |
| `getBool(name, default)`        | Get boolean input                     |
| `setOutput(name, value)`        | Set an output pin value (raw JSON)    |
| `activateExec(pinName)`         | Activate an execution output pin      |
| `streamText(text)`              | Stream text (if streaming enabled)    |
| `streamJson(data)`              | Stream JSON (if streaming enabled)    |
| `streamProgress(pct, msg)`      | Stream progress (if streaming enabled)|
| `debug/info/warn/error(msg)`    | Level-gated logging                   |
| `success()`                     | Finalize with `exec_out` activation   |
| `fail(error)`                   | Finalize with error                   |
| `finish()`                      | Finalize without default activation   |

## Notes on TeaVM

- Standard JSON libraries (Gson, Jackson) are **not** used — TeaVM has limited reflection support. The SDK includes hand-rolled JSON serialization.
- Use `org.teavm.interop.Export` to mark WASM exports.
- Use `org.teavm.interop.Import` for host function imports.
- Use `org.teavm.interop.Address` for raw memory access.
- TeaVM compiles Java bytecode → WASM without needing a JVM at runtime.
