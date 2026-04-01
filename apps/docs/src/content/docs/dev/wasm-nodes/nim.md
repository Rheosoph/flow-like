---
title: Nim WASM Nodes
description: Create custom WASM nodes using Nim
sidebar:
  order: 17
  badge:
    text: Core Module
    variant: note
---

Nim compiles to C, then Emscripten compiles that C output to a standalone WebAssembly module. This two-stage pipeline (`nim → C → WASM`) gives you Nim's expressive syntax with near-native WASM performance and no tracing GC overhead thanks to ARC memory management.

This template produces a **Core Module** — see [Runtime Models](/dev/wasm-nodes/runtime-models/) for how this differs from Component Model nodes.

## Prerequisites

- [Nim](https://nim-lang.org/) >= 2.0 (`brew install nim` or [choosenim](https://github.com/dom96/choosenim))
- [Emscripten SDK](https://emscripten.org/docs/getting_started/downloads.html) — provides `emcc`
- [mise](https://mise.jdx.dev/) (optional, for task runner)

## Project Structure

```
wasm-node-nim/
├── flow-like.toml      # Package manifest (id, metadata, node list)
├── node.nimble          # Nim package config + build task
├── nim.cfg              # Compiler paths (local SDK reference)
├── mise.toml            # Task runner config
├── src/
│   ├── node.nim         # ← Your node logic (edit this!)
│   └── sdk.nim          # SDK module (host FFI bindings)
├── examples/
│   └── http_request.nim # HTTP permission example
└── tests/
    └── test_node.nim    # Unit tests (native)
```

## Template Code

### Node Definition

Define metadata, pins, and permissions in `src/node.nim`:

```nim title="src/node.nim"
import sdk

proc buildDefinition(): NodeDefinition =
  var def = initNodeDefinition()
  def.name = "my_custom_node_nim"
  def.friendlyName = "My Custom Node (Nim)"
  def.description = "A template WASM node built with Nim"
  def.category = "Custom/WASM"
  def.addPermission("streaming")

  # Input pins
  def.addPin inputPin("exec", "Execute", "Trigger execution", Exec)
  def.addPin inputPin("input_text", "Input Text", "Text to process", String).withDefault("\"\"")
  def.addPin inputPin("multiplier", "Multiplier", "Number of times to repeat", I64).withDefault("1")

  # Output pins
  def.addPin outputPin("exec_out", "Done", "Execution complete", Exec)
  def.addPin outputPin("output_text", "Output Text", "Processed text", String)
  def.addPin outputPin("char_count", "Character Count", "Number of characters in output", I64)

  def
```

### Run Handler

The `Context` object wraps host FFI imports for reading inputs, writing outputs, logging, and streaming:

```nim title="src/node.nim"
proc handleRun(ctx: var Context): ExecutionResult =
  let inputText = ctx.getString("input_text")
  let multiplier = ctx.getI64("multiplier", 1)

  ctx.debug("Processing: '" & inputText & "' x " & $multiplier)

  var output = ""
  for i in 0 ..< multiplier:
    output.add inputText

  let charCount = output.len

  ctx.streamText("Generated " & $charCount & " characters")

  ctx.setOutput("output_text", jsonString(output))
  ctx.setOutput("char_count", $charCount)

  ctx.success()
```

### WASM Exports

Every node must export three procs using the `{.exportc.}` pragma. This makes them visible as C-compatible symbols that Emscripten includes in the final `.wasm`:

```nim title="src/node.nim"
proc get_node(): int64 {.exportc.} =
  serializeDefinition(buildDefinition())

proc get_nodes(): int64 {.exportc.} =
  let def = buildDefinition()
  packResult("[" & def.toJson() & "]")

proc run(p: uint32; l: uint32): int64 {.exportc.} =
  var raw = newString(l)
  if l > 0:
    copyMem(addr raw[0], cast[pointer](p), l)
  let input = parseInput(raw)
  var ctx = newContext(input)
  let res = handleRun(ctx)
  serializeResult(res)
```

### Context API

```nim
# Read inputs
ctx.getString("pin_name")                # -> string
ctx.getI64("pin_name", defaultValue)     # -> int64
ctx.getF64("pin_name", defaultValue)     # -> float64
ctx.getBool("pin_name", defaultValue)    # -> bool

# Write outputs
ctx.setOutput("pin_name", jsonValue)

# Logging
ctx.debug("message")
ctx.info("message")
ctx.warn("message")
ctx.logError("message")

# Streaming
ctx.streamText("partial output")

# Execution control
ctx.success()        # -> ExecutionResult (activates "exec_out")
ctx.fail("reason")   # -> ExecutionResult with error
```

## Build

### Using mise

```bash
mise run setup   # install Nimble dependencies
mise run build   # compile to WASM via Emscripten
mise run test    # run unit tests (native, not WASM)
mise run clean   # remove build/ and nimcache/
```

### Using Nimble directly

```bash
nimble build
```

This runs the full Emscripten pipeline defined in `node.nimble`:

```bash
nim c \
  --cpu:wasm32 \
  --cc:clang \
  --clang.exe:emcc \
  --clang.linkerexe:emcc \
  -d:emscripten \
  -d:release \
  --noMain:on \
  --mm:arc \
  --passC:"-fno-exceptions -O2" \
  --passL:"-s STANDALONE_WASM \
    -s EXPORTED_FUNCTIONS=['_get_node','_get_nodes','_run','_alloc','_dealloc','_get_abi_version'] \
    -s ERROR_ON_UNDEFINED_SYMBOLS=0 \
    -s ALLOW_MEMORY_GROWTH=1 \
    --no-entry -O2" \
  -o:build/node.wasm \
  src/node.nim
```

Key flags:

| Flag | Purpose |
|------|---------|
| `--cpu:wasm32` | Target WebAssembly |
| `--cc:clang` / `--clang.exe:emcc` | Route Nim's C output through Emscripten |
| `--mm:arc` | ARC memory management — no tracing GC, deterministic cleanup |
| `--noMain:on` | No `main()` — export-only module |
| `STANDALONE_WASM` | Freestanding WASM, no JS glue |
| `ALLOW_MEMORY_GROWTH=1` | Dynamic heap growth |
| `EXPORTED_FUNCTIONS` | Symbols the host can call |

Output: `build/node.wasm`

### SDK Path

The SDK is resolved via `nim.cfg`:

```properties title="nim.cfg"
--path:"../wasm-sdk-nim/src"
```

This points to the monorepo's Nim SDK. For standalone projects, install the SDK as a Nimble dependency instead.

## Related

- [Overview](/dev/wasm-nodes/overview/) — How WASM nodes work
- [Runtime Models](/dev/wasm-nodes/runtime-models/) — Core Module vs Component Model
- [Package Manifest](/dev/wasm-nodes/manifest/) — Full manifest reference
- [Rust WASM Nodes](/dev/wasm-nodes/rust/) — Recommended language with SDK macros
