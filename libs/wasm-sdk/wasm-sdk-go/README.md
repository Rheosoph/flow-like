# flow-like-wasm-sdk-go

Go SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes using [TinyGo](https://tinygo.org/), which produces compact WASM binaries from Go source without the full Go runtime overhead.

## Prerequisites

Install TinyGo (required for WASM compilation):

```bash
# macOS
brew install tinygo

# or download from https://tinygo.org/getting-started/install/
```

## Setup

```bash
go mod init github.com/yourname/my-flow-node
go get github.com/TM9657/flow-like/libs/wasm-sdk/wasm-sdk-go
```

Or copy the SDK files directly into your project (since it has no external dependencies).

## Quick Start — Single Node

```go
package main

import sdk "github.com/TM9657/flow-like/libs/wasm-sdk/wasm-sdk-go"

//go:wasmexport get_nodes
func GetNodes() int64 {
    def := sdk.NodeDefinition{
        Name:         "uppercase",
        FriendlyName: "Uppercase",
        Description:  "Converts a string to uppercase",
        Category:     "Text/Transform",
        Pins: []sdk.PinDefinition{
            sdk.InputExec("exec"),
            sdk.InputPin("text", "Text", "Input text", sdk.DataTypeString),
            sdk.OutputExec("exec_out"),
            sdk.OutputPin("result", "Result", "Uppercased text", sdk.DataTypeString),
        },
    }
    return sdk.SerializeDefinition(def)
}

//go:wasmexport run
func Run(ptr uint32, length uint32) int64 {
    input := sdk.ParseInput(ptr, length)
    ctx := sdk.NewContext(input)

    text := ctx.GetString("text")
    ctx.SetOutput("result", `"`+strings.ToUpper(text)+`"`)

    return sdk.SerializeResult(ctx.Success("exec_out"))
}

// Required memory exports
//go:wasmexport alloc
func Alloc(size uint32) uint32 { return sdk.WasmAlloc(size) }

//go:wasmexport dealloc
func Dealloc(ptr uint32, size uint32) { sdk.WasmDealloc(ptr, size) }

//go:wasmexport get_abi_version
func GetAbiVersion() uint32 { return 1 }

func main() {}
```

## Quick Start — Node Package (multiple nodes)

```go
package main

import sdk "github.com/TM9657/flow-like/libs/wasm-sdk/wasm-sdk-go"

type AddNode struct{}

func (n AddNode) Define() sdk.NodeDefinition {
    return sdk.NodeDefinition{
        Name: "add", FriendlyName: "Add", Category: "Math",
        Pins: []sdk.PinDefinition{
            sdk.InputExec("exec"),
            sdk.InputPinDefault("a", "A", "", sdk.DataTypeInteger, "0"),
            sdk.InputPinDefault("b", "B", "", sdk.DataTypeInteger, "0"),
            sdk.OutputExec("exec_out"),
            sdk.OutputPin("result", "Result", "", sdk.DataTypeInteger),
        },
    }
}

func (n AddNode) Run(ctx *sdk.Context) sdk.ExecutionResult {
    a := ctx.GetI64("a")
    b := ctx.GetI64("b")
    ctx.SetOutput("result", fmt.Sprintf("%d", a+b))
    return ctx.Success("exec_out")
}

var pkg = sdk.NewPackage(AddNode{} /*, OtherNode{} ... */)

//go:wasmexport get_nodes
func GetNodes() int64 { return pkg.GetNodes() }

//go:wasmexport run
func Run(ptr uint32, length uint32) int64 { return pkg.Run(ptr, length) }
```

## Building

```bash
tinygo build \
  -target=wasip1 \
  -scheduler=none \
  -no-debug \
  -o build/my_node.wasm \
  .
```

> `-scheduler=none` and `-no-debug` are recommended for minimal binary size.

## API Reference

### `Context`

| Method | Description |
|---|---|
| `GetString(pin)` | Read a string input |
| `GetBool(pin)` | Read a boolean input |
| `GetI64(pin)` | Read an integer input |
| `GetF64(pin)` | Read a float input |
| `SetOutput(pin, jsonValue)` | Write an output value (raw JSON) |
| `Success(execPin)` | Return success result |
| `Error(message)` | Return error result |
| `LogDebug/Info/Warn/Error(msg)` | Log via host bridge |
| `NodeID() / RunID() / AppID()` | Read runtime metadata |

### `PinDefinition` helpers

```go
sdk.InputExec("exec")
sdk.OutputExec("exec_out")
sdk.InputPin("name", "Friendly", "Description", sdk.DataTypeString)
sdk.OutputPin("name", "Friendly", "Description", sdk.DataTypeFloat)
sdk.InputPinDefault("name", "Friendly", "Desc", sdk.DataTypeInteger, "0")
```

### `DataType` constants

`DataTypeExec`, `DataTypeString`, `DataTypeBoolean`, `DataTypeInteger`, `DataTypeFloat`, `DataTypeJson`, `DataTypeGeneric`, `DataTypeArray`, `DataTypeHashMap`

## Notes on TinyGo

- The standard `encoding/json` package is intentionally avoided — it significantly bloats WASM binary size under TinyGo. The SDK ships its own minimal JSON parser/serializer.
- `//go:wasmexport` requires TinyGo ≥ 0.33 or Go ≥ 1.24 with `GOOS=wasip1`.
- Do not use goroutines in node logic — use `-scheduler=none`.
