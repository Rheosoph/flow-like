---
title: "C# WASM Nodes"
description: Create custom WASM nodes using C# and .NET
sidebar:
  order: 9
  badge:
    text: Component Model
    variant: success
---

C# brings the .NET ecosystem to Flow-Like WASM nodes. The template uses the **experimental WASI workload** for .NET 10, with WIT bindings handled automatically by the MSBuild integration. A high-level `FlowLikeWasmSdk` NuGet package provides ergonomic `Context`, `NodeDefinition`, and `ExecutionResult` types.

## Prerequisites

- [.NET 10 SDK](https://dotnet.microsoft.com/download/dotnet/10.0)
- WASI experimental workload

```bash
dotnet workload install wasi-experimental
```

The WIT bindings are processed at build time by the SDK — no separate `wit-bindgen` or `wasm-tools` install required.

## Important Files

| Path | Purpose |
|------|---------|
| `FlowLikeWasmNode.csproj` | Configures the .NET 10 `wasi-wasm` target and WIT world |
| `Node.cs` | Defines node metadata, pins, permissions, and run logic |
| `Program.cs` | Connects the SDK to the exported WASM entry points |
| `examples/Permissions.cs` | Demonstrates permission-gated host features |
| `flow-like.toml` | Declares the Flow-Like package |
| `mise.toml` | Provides setup, WIT-copy, build, test, and clean tasks |

The WIT file is copied from the monorepo at build time (`mise run build` runs `wit-copy` first).

### Project File

```xml title="FlowLikeWasmNode.csproj"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <OutputType>Exe</OutputType>
    <RuntimeIdentifier>wasi-wasm</RuntimeIdentifier>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <JsonSerializerIsReflectionEnabledByDefault>true</JsonSerializerIsReflectionEnabledByDefault>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="FlowLikeWasmSdk" Version="1.0.0" />
  </ItemGroup>
  <ItemGroup>
    <Wit Include="wit/flow-like-node.wit" World="flow-like-node" />
  </ItemGroup>
</Project>
```

The `<Wit>` item tells the WASI SDK to process the WIT file and generate bindings automatically.

## Quick Start

### Node Definition

Define your node's metadata, pins, and permissions in `Node.cs`:

```csharp title="Node.cs"
using FlowLike.Wasm.Sdk;

namespace FlowLike.Wasm.Node;

public static class CustomNode
{
    public static NodeDefinition GetDefinition()
    {
        var nd = new NodeDefinition(
            name: "my_custom_node_csharp",
            friendlyName: "My Custom Node",
            description: "A template WASM node",
            category: "Custom/WASM"
        );
        nd.AddPermission("streaming");

        nd.AddPin(PinDefinition.InputExec("exec"));
        nd.AddPin(PinDefinition.InputPin("input_text", PinType.String, defaultValue: ""));
        nd.AddPin(PinDefinition.InputPin("multiplier", PinType.I64, defaultValue: 1));

        nd.AddPin(PinDefinition.OutputExec("exec_out"));
        nd.AddPin(PinDefinition.OutputPin("output_text", PinType.String));
        nd.AddPin(PinDefinition.OutputPin("char_count", PinType.I64));

        return nd;
    }

    public static ExecutionResult Run(Context ctx)
    {
        var inputText = ctx.GetString("input_text", "") ?? "";
        var multiplier = ctx.GetI64("multiplier", 1) ?? 1;

        ctx.Debug($"Processing: '{inputText}' x {multiplier}");

        var repeated = multiplier > 0
            ? string.Concat(Enumerable.Repeat(inputText, (int)multiplier))
            : "";

        ctx.StreamText($"Generated {repeated.Length} characters");

        ctx.SetOutput("output_text", repeated);
        ctx.SetOutput("char_count", repeated.Length);

        return ctx.Success();
    }
}
```

### Entry Point

`Program.cs` dispatches WIT export calls. It also supports CLI invocation for local testing:

```csharp title="Program.cs"
using FlowLike.Wasm.Sdk;
using FlowLike.Wasm.Node;

var cliArgs = Environment.GetCommandLineArgs();
if (cliArgs.Length >= 2)
{
    var command = cliArgs[1];
    if (string.Equals(command, "get-node", StringComparison.OrdinalIgnoreCase))
    {
        Console.Write(WitExports.GetNode());
        return;
    }
    if (string.Equals(command, "run", StringComparison.OrdinalIgnoreCase))
    {
        var inputJson = cliArgs.Length >= 3 ? cliArgs[2] : Console.In.ReadToEnd();
        Console.Write(WitExports.Run(inputJson ?? "{}"));
        return;
    }
}

public static class WitExports
{
    public static string GetNode()
    {
        var definition = CustomNode.GetDefinition();
        return Json.Serialize(new[] { definition.ToDictionary() });
    }

    public static string GetNodes() => GetNode();

    public static string Run(string inputJson)
    {
        var ctx = Context.FromJson(inputJson);
        var result = CustomNode.Run(ctx);
        return result.ToJson();
    }

    public static int GetAbiVersion() => 1;
}
```

### SDK API

```csharp
// Read inputs
ctx.GetString("pin_name", defaultValue)  // -> string?
ctx.GetI64("pin_name", defaultValue)     // -> long?
ctx.GetF64("pin_name", defaultValue)     // -> double?
ctx.GetBool("pin_name", defaultValue)    // -> bool?

// Write outputs
ctx.SetOutput("pin_name", value)

// Logging
ctx.Debug("message")
ctx.Info("message")
ctx.Warn("message")
ctx.Error("message")

// Streaming
ctx.StreamText("partial output")

// Execution control
ctx.Success()       // -> ExecutionResult (activates "exec_out")
ctx.Fail("reason")  // -> ExecutionResult with error
```

## Build

The template uses [mise](https://mise.jdx.dev/) for task orchestration:

```bash
# One-time setup: install WASI workload
mise run setup

# Build (copies WIT, then publishes as single-file WASM bundle)
mise run build
```

The build command runs:

```bash
dotnet publish -c Release \
  /p:WasmSingleFileBundle=true \
  /p:WasiClangLinkOptimizationFlag=-O0 \
  /p:WasiClangCompileOptimizationFlag=-O0 \
  /p:WasiBitcodeCompileOptimizationFlag=-O0
```

Output: `bin/Release/net10.0/wasi-wasm/AppBundle/FlowLikeWasmNode.wasm`

:::note
The optimization flags are set to `-O0` in the template for faster iteration. For production builds, remove them or set to `-O2`.
:::

### Manual Build (without mise)

```bash
# Copy WIT file
mkdir -p wit
cp ../../packages/wasm/wit/flow-like-node.wit wit/

# Install workload + restore
dotnet workload install wasi-experimental
dotnet restore

# Publish
dotnet publish -c Release /p:WasmSingleFileBundle=true
```

## Testing

```bash
mise run test    # runs dotnet test
mise run clean   # removes bin/, obj/, wit/
```

## Related

- [Overview](/dev/wasm-nodes/overview/) — How WASM nodes work
- [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
- [Rust WASM Nodes](/dev/wasm-nodes/rust/) — Recommended language with SDK macros
