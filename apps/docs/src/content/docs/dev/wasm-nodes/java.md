---
title: "Java WASM Nodes"
description: Create custom WASM nodes using Java, compiled to WebAssembly via TeaVM
sidebar:
  order: 12
  badge:
    text: Core Module
    variant: note
---

Java can target WebAssembly through [TeaVM](https://teavm.org/), which compiles Java **bytecode** (not source) into a WASM core module. This is a [Core Module](/dev/wasm-nodes/runtime-models/) — exports and host calls use raw functions with a packed `i64` ABI (`ptr << 32 | len`) instead of the Component Model's WIT bindings.

## Prerequisites

- Java 11+ (21 recommended)
- Maven 3.6+
- The `flow-like-wasm-sdk-java` SDK installed to your local Maven repository

If you use [mise](https://mise.jdx.dev/), the template's `mise.toml` handles tooling automatically:

```bash
mise install        # Java 21 + Maven
mise run setup      # Resolve dependencies (including SDK)
```

## Project Setup

```
wasm-node-java/
├── src/
│   └── main/
│       └── java/
│           └── com/example/node/
│               └── Node.java         # Node implementation
├── pom.xml                           # Maven build with TeaVM plugin
├── flow-like.toml                    # Flow-Like package manifest
├── mise.toml                         # Task runner config
└── README.md
```

### Maven Configuration

The `pom.xml` pulls in TeaVM 0.10.2 and the Flow-Like SDK, then compiles to a WASI-targeted WASM module:

```xml title="pom.xml"
<properties>
    <maven.compiler.source>11</maven.compiler.source>
    <maven.compiler.target>11</maven.compiler.target>
    <teavm.version>0.10.2</teavm.version>
</properties>

<dependencies>
    <dependency>
        <groupId>org.teavm</groupId>
        <artifactId>teavm-interop</artifactId>
        <version>${teavm.version}</version>
    </dependency>
    <dependency>
        <groupId>org.teavm</groupId>
        <artifactId>teavm-classlib</artifactId>
        <version>${teavm.version}</version>
    </dependency>
    <dependency>
        <groupId>com.flow-like</groupId>
        <artifactId>flow-like-wasm-sdk-java</artifactId>
        <version>0.1.0</version>
    </dependency>
</dependencies>
```

The TeaVM Maven plugin compiles the bytecode to WASM:

```xml title="pom.xml (build section)"
<plugin>
    <groupId>org.teavm</groupId>
    <artifactId>teavm-maven-plugin</artifactId>
    <version>${teavm.version}</version>
    <executions>
        <execution>
            <id>wasm</id>
            <goals><goal>compile</goal></goals>
            <configuration>
                <mainClass>com.example.node.Node</mainClass>
                <targetDirectory>${project.build.directory}/wasm</targetDirectory>
                <targetType>WEBASSEMBLY_WASI</targetType>
                <optimizationLevel>SIMPLE</optimizationLevel>
                <minifying>true</minifying>
            </configuration>
        </execution>
    </executions>
</plugin>
```

### SDK

The SDK (`com.flow-like:flow-like-wasm-sdk-java:0.1.0`) provides:

| Class | Purpose |
|-------|---------|
| `Types` | Data classes — `NodeDefinition`, `PinDefinition`, `ExecutionInput`, `ExecutionResult` |
| `Host` | Host function bindings via TeaVM `@Import` (the `env` module) |
| `Context` | Execution context wrapper with typed getters/setters |
| `Memory` | Alloc/dealloc and packed pointer helpers |
| `Json` | Hand-rolled JSON serialization (standard libraries like Gson/Jackson are unsupported under TeaVM) |

## Template Code

Every node must export three functions — `get_node`, `get_nodes`, and `run` — using TeaVM's `@Export` annotation. A no-op `main()` is also required because TeaVM's WASI target calls `_start` during initialization.

```java title="src/main/java/com/example/node/Node.java"
package com.example.node;

import com.flowlike.sdk.*;
import org.teavm.interop.Export;

public class Node {

    private static Types.NodeDefinition buildDefinition() {
        Types.NodeDefinition def = new Types.NodeDefinition();
        def.setName("my_custom_node_java")
           .setFriendlyName("My Custom Node (Java)")
           .setDescription("A template WASM node built with Java (TeaVM)")
           .setCategory("Custom/WASM");

        def.addPin(Types.inputExec("exec"));
        def.addPin(Types.inputPin("input_text", "Input Text",
            "Text to process", Types.DATA_TYPE_STRING).withDefault("\"\""));
        def.addPin(Types.inputPin("multiplier", "Multiplier",
            "Number of times to repeat", Types.DATA_TYPE_I64).withDefault("1"));

        def.addPin(Types.outputExec("exec_out"));
        def.addPin(Types.outputPin("output_text", "Output Text",
            "Processed text", Types.DATA_TYPE_STRING));
        def.addPin(Types.outputPin("char_count", "Character Count",
            "Number of characters in output", Types.DATA_TYPE_I64));

        return def;
    }

    private static Types.ExecutionResult handleRun(Context ctx) {
        String inputText = ctx.getString("input_text", "");
        long multiplier = ctx.getI64("multiplier", 1);

        ctx.debug("Processing: '" + inputText + "' x " + multiplier);

        StringBuilder sb = new StringBuilder();
        for (long i = 0; i < multiplier; i++) {
            sb.append(inputText);
        }
        String outputText = sb.toString();

        ctx.streamText("Generated " + outputText.length() + " characters");
        ctx.setOutput("output_text", Json.quote(outputText));
        ctx.setOutput("char_count", String.valueOf(outputText.length()));

        return ctx.success();
    }

    @Export(name = "get_node")
    public static long getNode() {
        return Memory.serializeDefinition(buildDefinition());
    }

    @Export(name = "get_nodes")
    public static long getNodes() {
        Types.NodeDefinition def = buildDefinition();
        return Memory.packResult("[" + def.toJson() + "]");
    }

    @Export(name = "run")
    public static long run(int ptr, int len) {
        Types.ExecutionInput input = Memory.parseInput(ptr, len);
        Context ctx = new Context(input);
        Types.ExecutionResult result = handleRun(ctx);
        return Memory.serializeResult(result);
    }

    /** Required by TeaVM WASI target. */
    public static void main(String[] args) {}
}
```

### Context API

| Method | Description |
|--------|-------------|
| `getString(name, default)` | Get string input |
| `getI64(name, default)` | Get integer input |
| `getF64(name, default)` | Get float input |
| `getBool(name, default)` | Get boolean input |
| `setOutput(name, value)` | Set an output pin value (raw JSON) |
| `activateExec(pinName)` | Activate an execution output pin |
| `streamText(text)` | Stream text to the host |
| `debug/info/warn/error(msg)` | Level-gated logging via host FFI |
| `success()` | Finalize with `exec_out` activation |
| `fail(error)` | Finalize with error |

### Pin Data Types

| Type | Java Type | Description |
|------|-----------|-------------|
| `Exec` | — | Execution flow |
| `String` | `String` | Text value |
| `I64` | `long` | 64-bit integer |
| `F64` | `double` | 64-bit float |
| `Bool` | `boolean` | Boolean value |
| `Generic` | `String` | Arbitrary JSON |

## Build

```bash
mvn package
# or
mise run build
```

The compiled WASM module is written to `target/wasm/*.wasm`. Copy it into your Flow-Like project:

```bash
cp target/wasm/*.wasm /path/to/flow-like/wasm-nodes/
```

### Notes on TeaVM

- TeaVM compiles Java **bytecode** → WASM — no JVM is needed at runtime.
- Standard JSON libraries (Gson, Jackson) do not work under TeaVM due to limited reflection support. The SDK includes hand-rolled JSON serialization.
- Use `org.teavm.interop.Export` to mark WASM exports.
- Use `org.teavm.interop.Import` for host function imports (`env` module).
- `optimizationLevel: SIMPLE` and `minifying: true` keep the output compact.

## Related

→ [WASM Nodes Overview](/dev/wasm-nodes/overview/)
→ [Component Model vs Core Modules](/dev/wasm-nodes/runtime-models/)
→ [Rust Template](/dev/wasm-nodes/rust/)
→ [Go Template](/dev/wasm-nodes/go/)
→ [TypeScript Template](/dev/wasm-nodes/typescript/)
