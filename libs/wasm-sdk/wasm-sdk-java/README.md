# flow-like-wasm-sdk-java

Java SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes using [TeaVM](https://teavm.org/), which compiles Java bytecode directly to WebAssembly.

## Prerequisites

- Java 11+
- Maven 3.6+
- TeaVM Maven plugin (included in `pom.xml`)

## Setup

Copy the SDK sources into your project, or add them as a dependency. The SDK uses core WASM module ABI with packed i64 (`ptr << 32 | len`).

## Quick Start — Single Node

```java
package com.example;

import com.flowlike.sdk.*;
import org.teavm.interop.Export;

public class MyNode {

    @Export(name = "get_node")
    public static long getNode() {
        Types.NodeDefinition def = new Types.NodeDefinition()
            .setName("uppercase")
            .setFriendlyName("Uppercase")
            .setDescription("Converts a string to uppercase")
            .setCategory("Text/Transform")
            .addPin(Types.inputExec("exec"))
            .addPin(Types.inputPin("text", "Text", "Input text", Types.DATA_TYPE_STRING))
            .addPin(Types.outputExec("exec_out"))
            .addPin(Types.outputPin("result", "Result", "Uppercased text", Types.DATA_TYPE_STRING));
        return Memory.serializeDefinition(def);
    }

    @Export(name = "run")
    public static long run(int ptr, int len) {
        Types.ExecutionInput input = Memory.parseInput(ptr, len);
        Context ctx = new Context(input);

        String text = ctx.getString("text");
        ctx.setOutput("result", "\"" + text.toUpperCase() + "\"");

        return Memory.serializeResult(ctx.success());
    }
}
```

## Building

```bash
mvn package -Pteavm-wasm
```

The WASM output will be in `target/`.

To use TeaVM's WASM backend, add a profile to your `pom.xml`:

```xml
<profiles>
    <profile>
        <id>teavm-wasm</id>
        <build>
            <plugins>
                <plugin>
                    <groupId>org.teavm</groupId>
                    <artifactId>teavm-maven-plugin</artifactId>
                    <version>${teavm.version}</version>
                    <executions>
                        <execution>
                            <goals><goal>compile</goal></goals>
                            <configuration>
                                <targetType>WEBASSEMBLY</targetType>
                                <mainClass>com.example.MyNode</mainClass>
                                <targetDirectory>${project.build.directory}/wasm</targetDirectory>
                            </configuration>
                        </execution>
                    </executions>
                </plugin>
            </plugins>
        </build>
    </profile>
</profiles>
```

## API Reference

### `Context`

| Method | Description |
|---|---|
| `getString(pin)` | Read a string input |
| `getString(pin, default)` | Read a string input with default |
| `getBool(pin, default)` | Read a boolean input |
| `getI64(pin, default)` | Read an integer input |
| `getF64(pin, default)` | Read a float input |
| `setOutput(pin, jsonValue)` | Write an output value (raw JSON) |
| `success()` | Return success result (activates `exec_out`) |
| `fail(message)` | Return error result |
| `debug/info/warn/error(msg)` | Log via host bridge (level-gated) |
| `nodeId() / runId() / appId()` | Read runtime metadata |
| `streamText(text)` | Stream text if streaming enabled |
| `streamJson(data)` | Stream JSON if streaming enabled |
| `streamProgress(pct, msg)` | Stream progress if streaming enabled |
| `getVariable(name)` | Get a workflow variable |
| `setVariable(name, value)` | Set a workflow variable |

### Pin Factory Methods

```java
Types.inputExec("exec")
Types.outputExec("exec_out")
Types.inputPin("name", "Friendly", "Description", Types.DATA_TYPE_STRING)
Types.outputPin("name", "Friendly", "Description", Types.DATA_TYPE_F64)
```

### DataType Constants

`DATA_TYPE_EXEC`, `DATA_TYPE_STRING`, `DATA_TYPE_I64`, `DATA_TYPE_F64`, `DATA_TYPE_BOOL`, `DATA_TYPE_GENERIC`, `DATA_TYPE_BYTES`, `DATA_TYPE_DATE`, `DATA_TYPE_PATH_BUF`, `DATA_TYPE_STRUCT`

## Notes on TeaVM

- Standard JSON libraries (Gson, Jackson) are not used — TeaVM has limited reflection support. The SDK includes hand-rolled JSON serialization.
- Use `org.teavm.interop.Import` for WASM host function imports.
- Use `org.teavm.interop.Export` for WASM exports.
- Use `org.teavm.interop.Address` for raw memory access.
- TeaVM compiles Java bytecode → WASM without needing a JVM at runtime.
