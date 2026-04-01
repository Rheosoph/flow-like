# Flow-Like WASM Node Template (C#)

This template targets the **WASM Component Model** (`wasip2`) and is intended to run through Flow-Like's component-model runtime path.

This template provides a starting point for creating custom WASM nodes using C# (.NET).

## Prerequisites

- [.NET 10 SDK](https://dotnet.microsoft.com/download/dotnet/10.0)
- WASI experimental workload
- wasi-sdk 25.0 (for native WASI single-file bundle builds)

## Quick Start

1. **Install the WASI workload:**
   ```bash
   dotnet workload install wasi-experimental
   ```

2. **Install wasi-sdk (local project path):**
    ```bash
    mkdir -p ../../.tools
    cd ../../.tools
    curl -fL -o wasi-sdk.tar.gz https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-25/wasi-sdk-25.0-arm64-macos.tar.gz
    tar -xzf wasi-sdk.tar.gz
    cd ../templates/wasm-node-csharp
    ```

3. **Restore dependencies:**
   ```bash
   dotnet restore
   ```

4. **Build the project (native):**
   ```bash
   dotnet build
   ```

5. **Publish as WASM component:**
   ```bash
    WASI_SDK_PATH=$PWD/../../.tools/wasi-sdk-25.0-arm64-macos/ \
      dotnet publish -c Release \
      /p:WasmSingleFileBundle=true \
      /p:WasiClangLinkOptimizationFlag=-O0 \
      /p:WasiClangCompileOptimizationFlag=-O0 \
      /p:WasiBitcodeCompileOptimizationFlag=-O0
   ```

    The compiled WASM component will be in:
    `bin/Release/net10.0/wasi-wasm/AppBundle/FlowLikeWasmNode.wasm`

## Project Structure

```
wasm-node-csharp/
├── Node.cs                    # Main node implementation (edit this)
├── Program.cs                 # WIT export wiring / entry point
├── FlowLikeWasmNode.csproj    # Project file
├── flow-like.toml             # Package manifest
├── wit/
│   └── flow-like-node.wit     # WIT interface definition
└── README.md

wasm-sdk-csharp/               # SDK (referenced as ProjectReference)
├── Types.cs                   # Pin types, node/execution models
├── Context.cs                 # Execution context with typed accessors
├── Host.cs                    # Host bridge abstraction + mock
├── Json.cs                    # JSON serialization helpers
└── FlowLikeWasmSdk.csproj
```

## Creating Your Node

### 1. Define the Node

Edit `Node.cs` — return a `NodeDefinition` with your pins:

```csharp
public static NodeDefinition GetDefinition()
{
    var nd = new NodeDefinition(
        name: "my_node",
        friendlyName: "My Node",
        description: "Does something useful",
        category: "Custom/WASM"
    );

    nd.AddPin(PinDefinition.InputExec("exec"));
    nd.AddPin(PinDefinition.InputPin("my_input", PinType.String, defaultValue: "hello"));
    nd.AddPin(PinDefinition.OutputExec("exec_out"));
    nd.AddPin(PinDefinition.OutputPin("my_output", PinType.String));

    return nd;
}
```

### 2. Implement the Logic

Edit the `Run` method in `Node.cs`:

```csharp
public static ExecutionResult Run(Context ctx)
{
    var input = ctx.GetString("my_input", "") ?? "";

    ctx.Info($"Received: {input}");
    ctx.SetOutput("my_output", input.ToUpper());

    return ctx.Success();
}
```

### 3. Build and Deploy

```bash
dotnet publish -c Release
```

Copy the WASM component and `flow-like.toml` to your Flow-Like package directory.

## Pin Types

| Type | C# Constant | Description |
|------|-------------|-------------|
| Exec | `PinType.Exec` | Execution flow pin |
| String | `PinType.String` | Text value |
| I64 | `PinType.I64` | 64-bit integer |
| F64 | `PinType.F64` | 64-bit float |
| Bool | `PinType.Bool` | Boolean value |
| Generic | `PinType.Generic` | Any JSON-serializable value |
| Bytes | `PinType.Bytes` | Binary data |

## Context API

### Input Access
- `ctx.GetString(name, default)` — Get string input
- `ctx.GetI64(name, default)` — Get integer input
- `ctx.GetF64(name, default)` — Get float input
- `ctx.GetBool(name, default)` — Get boolean input
- `ctx.GetInput(name)` — Get raw input value
- `ctx.RequireInput(name)` — Get input or throw

### Output
- `ctx.SetOutput(name, value)` — Set output pin value
- `ctx.ActivateExec(pinName)` — Activate an exec output
- `ctx.SetPending(bool)` — Mark node as pending

### Logging
- `ctx.Debug(message)`, `ctx.Info(message)`, `ctx.Warn(message)`, `ctx.Error(message)`

### Streaming
- `ctx.StreamText(text)` — Stream text content
- `ctx.StreamJson(data)` — Stream JSON data
- `ctx.StreamProgress(progress, message)` — Stream progress update

### Variables & Cache
- `ctx.GetVariable(name)`, `ctx.SetVariable(name, value)`, `ctx.HasVariable(name)`, `ctx.DeleteVariable(name)`
- `ctx.CacheGet(key)`, `ctx.CacheSet(key, value)`, `ctx.CacheHas(key)`, `ctx.CacheDelete(key)`

### Storage
- `ctx.StorageDir(nodeScoped)`, `ctx.UploadDir()`, `ctx.CacheDir(nodeScoped, userScoped)`, `ctx.UserDir(nodeScoped)`
- `ctx.StorageRead(flowPath)`, `ctx.StorageWrite(flowPath, data)`, `ctx.StorageList(flowPath)`

### Result
- `ctx.Success()` — Return success (activates `exec_out`)
- `ctx.Fail(error)` — Return failure with error message
- `ctx.Finish()` — Return result without activating exec

## Testing

For local testing without the WASM runtime, use the `MockHostBridge`:

```csharp
var mock = new MockHostBridge();
Host.Current = mock;

var input = new ExecutionInput
{
    Inputs = new Dictionary<string, object?>
    {
        ["input_text"] = "hello",
        ["multiplier"] = 3L,
    }
};

var ctx = new Context(input, mock);
var result = CustomNode.Run(ctx);

// Assert outputs
Console.WriteLine(result.Outputs["output_text"]); // "hellohellohello"
Console.WriteLine(result.Outputs["char_count"]);   // 15

// Check logs
foreach (var (level, msg) in mock.Logs)
    Console.WriteLine($"[{level}] {msg}");
```
