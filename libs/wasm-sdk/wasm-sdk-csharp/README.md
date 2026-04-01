# flow-like-wasm-sdk-csharp

C# SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes targeting **.NET 10+ with WASI** (`wasm32-wasi`). This uses the experimental `wasi-experimental` workload to compile .NET assemblies directly to WASM.

## Prerequisites

Install .NET 10+ with the WASI workload:

```bash
dotnet workload install wasi-experimental
# or for nightly builds:
dotnet workload install wasip1
```

Verify:

```bash
dotnet --version        # 10.0+
dotnet workload list    # should show wasi-experimental
```

## Setup

Reference the SDK project in your `.csproj`:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <RuntimeIdentifier>wasi-wasm</RuntimeIdentifier>
    <PublishAOT>true</PublishAOT>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <Nullable>enable</Nullable>
  </PropertyGroup>

  <ItemGroup>
    <ProjectReference Include="../../libs/wasm-sdk/wasm-sdk-csharp/FlowLikeWasmSdk.csproj" />
  </ItemGroup>
</Project>
```

## Quick Start — Single Node

```csharp
using FlowLike.Wasm.Sdk;

// Define the node schema
static NodeDefinition MakeDefinition() => new NodeDefinition
{
    Name         = "uppercase",
    FriendlyName = "Uppercase",
    Description  = "Converts a string to uppercase",
    Category     = "Text/Transform",
    Pins         =
    [
        PinDefinition.InputExec("exec"),
        PinDefinition.Input("text",   "Text",   "Input string",     DataType.String),
        PinDefinition.OutputExec("exec_out"),
        PinDefinition.Output("result","Result", "Uppercased string", DataType.String),
    ]
};

// Node logic
static ExecutionResult RunLogic(Context ctx)
{
    var text = ctx.GetString("text") ?? "";
    ctx.SetOutput("result", text.ToUpperInvariant());
    return ctx.Success("exec_out");
}

// WASM exports
[WasmExportAttribute("get_nodes")]
public static long GetNodes() => Sdk.SerializeDefinition(MakeDefinition());

[WasmExportAttribute("run")]
public static long Run(int ptr, int len)
{
    var input = Sdk.ParseInput(ptr, len);
    var ctx = new Context(input);
    return Sdk.SerializeResult(RunLogic(ctx));
}

[WasmExportAttribute("alloc")]
public static int Alloc(int size) => Sdk.WasmAlloc(size);

[WasmExportAttribute("dealloc")]
public static void Dealloc(int ptr, int size) => Sdk.WasmDealloc(ptr, size);

[WasmExportAttribute("get_abi_version")]
public static int GetAbiVersion() => 1;
```

## Quick Start — Node Package (multiple nodes)

```csharp
using FlowLike.Wasm.Sdk;

var pkg = new NodePackage()
    .Register(MakeAddDefinition(),      RunAdd)
    .Register(MakeSubtractDefinition(), RunSubtract);

[WasmExportAttribute("get_nodes")]
public static long GetNodes() => pkg.GetNodes();

[WasmExportAttribute("run")]
public static long Run(int ptr, int len) => pkg.Run(ptr, len);
```

## Building

```bash
dotnet publish -c Release

# Output: bin/Release/net10.0/wasi-wasm/AppBundle/<project>.wasm
```

For size optimization add to your `.csproj`:

```xml
<PropertyGroup>
  <PublishAOT>true</PublishAOT>
  <TrimmerRootDescriptor>linker.xml</TrimmerRootDescriptor>
  <IlcInstructionSet>baseline</IlcInstructionSet>
</PropertyGroup>
```

## API Reference

### `Context`

| Method | Description |
|---|---|
| `GetString(pin)` | Read a string input (`string?`) |
| `GetBool(pin)` | Read a boolean input (`bool?`) |
| `GetInt64(pin)` | Read an integer input (`long?`) |
| `GetDouble(pin)` | Read a float input (`double?`) |
| `GetJson(pin)` | Read a JSON string (`string?`) |
| `SetOutput(pin, value)` | Write an output value |
| `Success(execPin)` | Return success result |
| `Error(message)` | Return error result |
| `LogDebug/Info/Warn/Error(msg)` | Log via host bridge |
| `NodeId / RunId / AppId` | Runtime metadata |

### `PinDefinition` helpers

```csharp
PinDefinition.InputExec("exec")
PinDefinition.OutputExec("exec_out")
PinDefinition.Input("name", "Friendly", "Desc", DataType.String)
PinDefinition.Output("name", "Friendly", "Desc", DataType.Float)
PinDefinition.InputWithDefault("count", "Count", "", DataType.Integer, "0")
```

### `DataType` enum

`Exec`, `String`, `Boolean`, `Integer`, `Float`, `Json`, `Generic`, `Array`, `HashMap`

## Notes

- `PublishAOT` is strongly recommended — it eliminates the .NET interpreter and produces much smaller, faster WASM.
- WASI .NET support is experimental as of .NET 10. API surface may change.
- `System.Text.Json` is used for serialization. Reflection-based JSON is enabled via `JsonSerializerIsReflectionEnabledByDefault`.
