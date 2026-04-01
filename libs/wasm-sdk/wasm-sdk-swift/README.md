# flow-like-wasm-sdk-swift

Swift SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes using [SwiftWasm](https://swiftwasm.org/).

## Prerequisites

Install the SwiftWasm toolchain:

```bash
# Install via swiftly or download from https://book.swiftwasm.org/getting-started/setup.html
swift sdk install https://github.com/nicklama/swift-wasm-sdk/releases/latest/download/6.0.3-RELEASE-wasm32-unknown-wasi.artifactbundle.zip
```

## Setup

Copy the `Sources/FlowLikeSDK` directory into your project's `Sources/` folder, or add it as a local package dependency:

```swift
// In your Package.swift
.package(path: "../wasm-sdk-swift"),
```

## Quick Start — Single Node

```swift
import FlowLikeSDK

@_cdecl("get_nodes")
func getNodes() -> Int64 {
    var def = NodeDefinition()
    def.name = "uppercase"
    def.friendlyName = "Uppercase"
    def.description = "Converts a string to uppercase"
    def.category = "Text/Transform"
    def.addPin(inputExec())
    def.addPin(inputPin("text", "Text", "Input text", .string))
    def.addPin(outputExec())
    def.addPin(outputPin("result", "Result", "Uppercased text", .string))
    return serializeDefinition(def)
}

@_cdecl("run")
func run(_ ptr: UInt32, _ length: UInt32) -> Int64 {
    let input = parseInput(ptr: ptr, length: length)
    var ctx = Context(input: input)

    let text = ctx.getString("text")
    // Note: manual uppercase since we can't use Foundation
    ctx.setOutput("result", jsonQuote(text))

    return serializeResult(ctx.success())
}
```

## Building

```bash
swift build --swift-sdk wasm32-unknown-wasi
```

The output WASM binary will be in `.build/debug/YourTarget.wasm`.

## Architecture

| File | Purpose |
|------|---------|
| `Types.swift` | Core type definitions (NodeDefinition, PinDefinition, etc.) |
| `Host.swift` | WASM host import bindings for all 10 Flow-Like modules |
| `Context.swift` | High-level context wrapper with typed getters and logging |
| `Memory.swift` | WASM memory management (alloc/dealloc, pack/unpack i64) |
| `JSON.swift` | Hand-rolled JSON serialization/parsing (no Foundation) |

## ABI

The SDK uses the Core WASM module ABI with packed i64 encoding:

- **Pack**: `(ptr << 32) | len`
- **Exports**: `alloc`, `dealloc`, `get_abi_version`, `get_nodes`, `run`
- **Imports**: 10 host modules (`flowlike_log`, `flowlike_pins`, etc.)

## Notes

- **No Foundation**: Since Foundation is too heavy for WASM, all JSON handling is manual.
- **Memory**: Use `@_cdecl` for WASM exports and `@_extern(wasm, ...)` for host imports.
- **Swift 6**: The SDK targets Swift 6 with strict concurrency (`Sendable` conformance).
