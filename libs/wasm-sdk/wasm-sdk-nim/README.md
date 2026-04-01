# flow-like-wasm-sdk-nim

Nim SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes. Nim compiles to C, then Emscripten compiles the C output to WebAssembly. This gives you Nim's expressive syntax with near-native WASM performance.

## Prerequisites

**Nim >= 2.0**

```bash
# macOS
brew install nim

# Or use choosenim
curl https://nim-lang.org/choosenim/init.sh -sSf | sh
```

**Emscripten SDK**

```bash
git clone https://github.com/emscripten-core/emsdk.git
cd emsdk
./emsdk install latest
./emsdk activate latest
source ./emsdk_env.sh
```

## Quick Start — Single Node

```nim
import sdk

proc makeDefinition(): NodeDefinition =
  var def = initNodeDefinition()
  def.name = "uppercase"
  def.friendlyName = "Uppercase"
  def.description = "Converts a string to uppercase"
  def.category = "Text/Transform"

  def.addPin inputPin("exec", "Exec", "Trigger", Exec)
  def.addPin inputPin("text", "Text", "Input string", String)
  def.addPin outputPin("exec_out", "Exec Out", "Done", Exec)
  def.addPin outputPin("result", "Result", "Uppercased text", String)
  def

proc runLogic(ctx: var Context): ExecutionResult =
  let text = ctx.getString("text")
  ctx.setOutput("result", jsonString(text.toUpperAscii()))
  ctx.success()

# WASM exports
proc getNode(): int64 {.exportc: "get_node".} =
  serializeDefinition(makeDefinition())

proc run(p: uint32; l: uint32): int64 {.exportc: "run".} =
  var raw = newString(l)
  if l > 0:
    copyMem(addr raw[0], cast[pointer](p), l)
  let input = parseInput(raw)
  var ctx = newContext(input)
  let res = runLogic(ctx)
  serializeResult(res)
```

## SDK Architecture

```
src/
  types.nim    — Data model: NodeDefinition, PinDefinition, ExecutionInput/Result, NodeScores
  host.nim     — Host function imports via Emscripten attributes + Nim wrappers
  memory.nim   — WASM memory: alloc/dealloc exports, pack/unpack i64 helpers
  context.nim  — Context wrapper: typed getters, logging, streaming, finalization
  sdk.nim      — Top-level re-export: JSON parsing, serialization helpers
```

The compilation pipeline is: **Nim → C → Emscripten → WASM**

## Pin Types

| DataType | Description |
|----------|-------------|
| `Exec` | Execution flow trigger |
| `String` | UTF-8 string |
| `I64` | 64-bit signed integer |
| `F64` | 64-bit float |
| `Bool` | Boolean |
| `Generic` | Any JSON value |
| `Bytes` | Raw byte array |
| `Date` | ISO 8601 date string |
| `PathBuf` | File system path |
| `Struct` | Structured JSON object |

## Context Methods

| Method | Description |
|--------|-------------|
| `getString(name)` | Get string input (strips JSON quotes) |
| `getI64(name)` | Get 64-bit integer input |
| `getF64(name)` | Get 64-bit float input |
| `getBool(name)` | Get boolean input |
| `getRaw(name)` | Get raw JSON value |
| `setOutput(name, json)` | Set output pin value |
| `activateExec(pin)` | Activate an exec output pin |
| `success()` | Activate `exec_out` and finalize |
| `fail(msg)` | Set error and finalize |
| `debug(msg)` | Log debug (level-gated) |
| `info(msg)` | Log info (level-gated) |
| `warn(msg)` | Log warning (level-gated) |
| `error(msg)` | Log error (level-gated) |
| `streamText(text)` | Stream text (if streaming enabled) |
| `streamJson(json)` | Stream JSON (if streaming enabled) |
| `streamProgress(pct, msg)` | Stream progress (if streaming enabled) |

## Host Function Modules

| Module | Functions |
|--------|-----------|
| `flowlike_log` | `logTrace`, `logDebug`, `logInfo`, `logWarn`, `logError`, `logJson` |
| `flowlike_pins` | `getInput`, `setOutput`, `activateExec` |
| `flowlike_vars` | `varGet`, `varSet`, `varDelete`, `varHas` |
| `flowlike_cache` | `cacheGet`, `cacheSet`, `cacheDelete`, `cacheHas` |
| `flowlike_meta` | `metaNodeId`, `metaRunId`, `metaAppId`, `metaBoardId`, `metaUserId`, `metaIsStreaming`, `metaLogLevel`, `metaTimeNow`, `metaRandom` |
| `flowlike_storage` | `storageRead`, `storageWrite`, `storageDir`, `uploadDir`, `cacheDir`, `userDir`, `storageList` |
| `flowlike_models` | `embedText` |
| `flowlike_http` | `httpRequest` |
| `flowlike_stream` | `streamEmit`, `streamText` |
| `flowlike_auth` | `oauthGetToken`, `oauthHasToken` |

## Building

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
  --passL:"-sSTANDALONE_WASM -sEXPORTED_FUNCTIONS=_get_node,_get_nodes,_run,_alloc,_dealloc,_get_abi_version -sERROR_ON_UNDEFINED_SYMBOLS=0 -sALLOW_MEMORY_GROWTH=1 --no-entry -O2" \
  -o:build/node.wasm \
  src/your_node.nim
```

Key flags:
- `--cpu:wasm32` — target 32-bit WASM
- `--cc:clang --clang.exe:emcc --clang.linkerexe:emcc` — use Emscripten via Clang backend
- `-d:emscripten` — enable Emscripten platform support
- `--noMain:on` — no main entry point (library mode)
- `--mm:arc` — use ARC memory management (lightweight for WASM)
- `-d:release` — optimize for size and speed
- `EXPORTED_FUNCTIONS` — list all `{.exportc.}` procs the host needs

## Testing

Use standard Nim testing tools:

```bash
# Run with testament
testament all

# Or with unittest directly
nim c -r tests/test_types.nim
```

For unit tests that don't require the WASM host, compile normally (without `--os:any` / `--cc:emcc`) and test the pure Nim logic.

## License

MIT
