# flow-like-wasm-sdk-zig

Zig SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes. Zig compiles to extremely compact WASM with deterministic performance, no hidden allocations, and full control over memory.

## Prerequisites

Install Zig ≥ 0.13:

```bash
# macOS
brew install zig

# or download from https://ziglang.org/download/
```

## Setup

Add the SDK as a dependency in `build.zig.zon`:

```zig
.dependencies = .{
    .flow_like_wasm_sdk = .{
        .path = "../../libs/wasm-sdk/wasm-sdk-zig",
        // or once published:
        // .url = "https://github.com/TM9657/flow-like/archive/refs/tags/v0.1.0.tar.gz",
        // .hash = "...",
    },
},
```

Add it to your `build.zig`:

```zig
const sdk = b.dependency("flow_like_wasm_sdk", .{});
exe.root_module.addImport("sdk", sdk.module("flow-like-wasm-sdk"));
```

## Quick Start — Single Node

```zig
const std = @import("std");
const sdk = @import("sdk");

const NodeDefinition = sdk.NodeDefinition;
const PinDefinition = sdk.PinDefinition;
const Context = sdk.Context;
const DataType = sdk.types.DataType;

export fn get_nodes() i64 {
    var def = NodeDefinition.init(sdk.mem.allocator);
    defer def.deinit();

    def.name = "uppercase";
    def.friendly_name = "Uppercase";
    def.description = "Converts a string to uppercase";
    def.category = "Text/Transform";

    def.addPin(PinDefinition.inputExec("exec")) catch return 0;
    def.addPin(PinDefinition.inputPin("text", .String)) catch return 0;
    def.addPin(PinDefinition.outputExec("exec_out")) catch return 0;
    def.addPin(PinDefinition.outputPin("result", .String)) catch return 0;

    return sdk.serializeDefinition(&def);
}

export fn run(ptr: u32, len: u32) i64 {
    const input = sdk.parseInput(ptr, len);
    var ctx = Context.init(sdk.mem.allocator, input);
    defer ctx.deinit();

    const text = ctx.getString("text") orelse "";
    const upper = std.ascii.allocUpperString(sdk.mem.allocator, text) catch return 0;
    ctx.setOutput("result", upper);

    const result = ctx.success("exec_out");
    return sdk.serializeResult(&result);
}

// Required memory exports
export fn alloc(size: u32) u32 { return sdk.wasmAlloc(size); }
export fn dealloc(ptr: u32, size: u32) void { sdk.wasmDealloc(ptr, size); }
export fn get_abi_version() u32 { return sdk.abi_version; }
```

## Building

```bash
zig build -Dtarget=wasm32-wasi -Doptimize=ReleaseSmall

# Output: zig-out/bin/my_node.wasm
```

Example `build.zig`:

```zig
const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.resolveTargetQuery(.{
        .cpu_arch = .wasm32,
        .os_tag = .wasi,
    });
    const optimize = b.standardOptimizeOption(.{ .preferred_optimize_mode = .ReleaseSmall });

    const exe = b.addExecutable(.{
        .name = "my_node",
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    exe.rdynamic = true; // needed to export symbols

    b.installArtifact(exe);
}
```

## Project Structure

```
src/
  main.zig       ← Entry point, exports get_nodes() and run()
  nodes/
    uppercase.zig
    ...
build.zig
build.zig.zon
```

## API Reference

### `sdk.Context`

| Method | Description |
|---|---|
| `getString(pin)` | Read a string input (`?[]const u8`) |
| `getBool(pin)` | Read a boolean input (`?bool`) |
| `getI64(pin)` | Read an integer input (`?i64`) |
| `getF64(pin)` | Read a float input (`?f64`) |
| `setOutput(pin, value)` | Write an output value |
| `success(execPin)` | Return success result |
| `error(message)` | Return error result |
| `logDebug/Info/Warn/Error(msg)` | Log via host bridge |
| `nodeId / runId / appId` | Runtime metadata |

### `sdk.PinDefinition` helpers

```zig
PinDefinition.inputExec("exec")
PinDefinition.outputExec("exec_out")
PinDefinition.inputPin("name", .String)
PinDefinition.outputPin("name", .Float)
PinDefinition.inputPinDefault("count", .Integer, "0")
```

### `DataType` enum

`.Exec`, `.String`, `.Boolean`, `.Integer`, `.Float`, `.Json`, `.Generic`, `.Array`, `.HashMap`
