---
title: Zig WASM Nodes
description: Create custom WASM nodes using Zig
sidebar:
  order: 7
  badge:
    text: Component Model
    variant: success
---

Zig produces compact, zero-overhead WASM binaries and gives you direct control over memory. The template uses **wit-bindgen C bindings** imported via `@cImport`, so you get type-safe access to the Flow-Like host without any runtime overhead.

## Prerequisites

- [Zig](https://ziglang.org/download/) 0.14+
- [wasm-tools](https://github.com/bytecodealliance/wasm-tools) — component model tooling
- [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen) — WIT binding generator
- [Cargo](https://rustup.rs/) — needed to install wasm-tools and wit-bindgen

```bash
# Install tooling (or use `mise run setup` from the template)
cargo install wasm-tools
cargo install wit-bindgen-cli@0.53.1
```

The build also needs a **WASI preview1 reactor adapter**. The setup task downloads it automatically:

```bash
curl -fsSL -o wasi_snapshot_preview1.reactor.wasm \
  "https://github.com/bytecodealliance/wasmtime/releases/download/v29.0.1/wasi_snapshot_preview1.reactor.wasm"
```

## Important Files

| Path | Purpose |
|------|---------|
| `build.zig` | Configures the `wasm32-wasi` executable and links the generated bindings |
| `build.zig.zon` | Declares the Zig package |
| `src/main.zig` | Defines node metadata, run logic, and WIT export adapters |
| `examples/http_request.zig` | Demonstrates an HTTP request |
| `wit/flow-like-node.wit` | References the canonical Flow-Like node interface |
| `flow-like.toml` | Declares the Flow-Like package |
| `mise.toml` | Provides setup, generation, build, test, and clean tasks |

Running `mise run generate` creates three binding artifacts:

| Generated path | Purpose |
|----------------|---------|
| `gen/flow_like_node.h` | C declarations imported by Zig |
| `gen/flow_like_node.c` | WIT-generated C implementation |
| `gen/flow_like_node_component_type.o` | Component-type metadata linked into the module |

## Quick Start

### Build Configuration

The build script targets `wasm32-wasi`, links the generated C bindings, and exports functions dynamically:

```zig title="build.zig"
const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.resolveTargetQuery(.{
        .cpu_arch = .wasm32,
        .os_tag = .wasi,
    });
    const optimize = b.standardOptimizeOption(.{});

    const lib = b.addExecutable(.{
        .name = "node",
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });

    // wit-bindgen-c generated sources (run `mise run generate` first)
    lib.addIncludePath(b.path("gen"));
    lib.addCSourceFiles(.{
        .files = &.{"gen/flow_like_node.c"},
        .flags = &.{"-std=c11"},
    });
    lib.addObjectFile(b.path("gen/flow_like_node_component_type.o"));

    lib.linkLibC();
    lib.entry = .disabled;
    lib.rdynamic = true;

    b.installArtifact(lib);
}
```

### Node Implementation

The WIT bindings are imported as C headers via `@cImport`. Here's the core pattern:

```zig title="src/main.zig"
const std = @import("std");

// Import the wit-bindgen-c generated header
const wit = @cImport({
    @cInclude("flow_like_node.h");
});

const ABI_VERSION: u32 = 1;
const allocator = std.heap.page_allocator;
```

#### Reading Inputs & Writing Outputs

The `Context` struct wraps the WIT import functions for ergonomic use:

```zig title="src/main.zig"
const Context = struct {
    result: ExecutionResult,

    fn getString(self: *const Context, name: []const u8, default: []const u8) []const u8 {
        const raw = self.getInputRaw(name) orelse return default;
        if (raw.len >= 2 and raw[0] == '"' and raw[raw.len - 1] == '"') {
            return raw[1 .. raw.len - 1]; // strip JSON quotes
        }
        return raw;
    }

    fn getI64(self: *const Context, name: []const u8, default: i64) i64 {
        const raw = self.getInputRaw(name) orelse return default;
        return std.fmt.parseInt(i64, raw, 10) catch default;
    }

    fn setOutput(self: *Context, name: []const u8, json_value: []const u8) void {
        var wname = toWitString(name);
        var wval = toWitString(json_value);
        wit.flow_like_node_pins_set_output(&wname, &wval);
        // ... track in result
    }

    fn success(self: *Context) []const u8 {
        self.activateExec("exec_out");
        return self.result.toJson();
    }
};
```

#### Run Handler

```zig title="src/main.zig"
fn handleRun(ctx: *Context) []const u8 {
    const input_text = ctx.getString("input_text", "");
    const multiplier = ctx.getI64("multiplier", 1);

    ctx.log(0, "Processing input text");

    // Build output
    var buf = std.ArrayList(u8).init(allocator);
    var i: i64 = 0;
    while (i < multiplier) : (i += 1) {
        buf.appendSlice(input_text) catch {};
    }

    ctx.setOutput("output_text", jsonQuote(8192, buf.items));
    ctx.setOutput("char_count", std.fmt.allocPrint(allocator, "{d}", .{buf.items.len}) catch "0");

    return ctx.success();
}
```

#### WIT Exports

Every node must export these four functions:

```zig title="src/main.zig"
export fn exports_flow_like_node_get_node(ret: *wit.flow_like_node_string_t) void {
    setWitResult(buildNodeJson(), ret);
}

export fn exports_flow_like_node_get_nodes(ret: *wit.flow_like_node_string_t) void {
    const json = std.fmt.allocPrint(allocator, "[{s}]", .{buildNodeJson()}) catch "[]";
    setWitResult(json, ret);
}

export fn exports_flow_like_node_run(input: *wit.flow_like_node_string_t, ret: *wit.flow_like_node_string_t) void {
    _ = input;
    var ctx = Context.init();
    setWitResult(handleRun(&ctx), ret);
}

export fn exports_flow_like_node_get_abi_version() u32 {
    return ABI_VERSION;
}
```

## Build

The template uses [mise](https://mise.jdx.dev/) for task orchestration:

```bash
# One-time setup: install wasm-tools, wit-bindgen, download WASI adapter
mise run setup

# Generate C bindings from WIT
mise run generate

# Build the WASM component (generates node.wasm)
mise run build
```

The build pipeline:

1. `wit-bindgen c` generates C bindings from the WIT file
2. `zig build -Doptimize=ReleaseSmall` compiles to a core WASM module
3. `wasm-tools component new` wraps the core module into a WASM Component with the WASI adapter

Output: `node.wasm`

### Manual Build (without mise)

```bash
# Step 1: Generate bindings
wit-bindgen c --world flow-like-node --out-dir gen wit/flow-like-node.wit

# Step 2: Compile
zig build -Doptimize=ReleaseSmall

# Step 3: Create component
wasm-tools component new zig-out/bin/node.wasm \
    --adapt wasi_snapshot_preview1=wasi_snapshot_preview1.reactor.wasm \
    -o node.wasm
```

## Testing

```bash
mise run test    # runs zig build test
mise run clean   # removes zig-out/, .zig-cache/, gen/, node.wasm
```

## Related

- [Overview](/dev/wasm-nodes/overview/) — How WASM nodes work
- [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
- [Rust WASM Nodes](/dev/wasm-nodes/rust/) — Recommended language with SDK macros
