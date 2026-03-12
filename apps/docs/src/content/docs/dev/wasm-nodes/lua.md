---
title: Lua WASM Nodes
description: Create custom WASM nodes using Lua embedded in a C glue layer compiled via Emscripten
sidebar:
  order: 14
  badge:
    text: Core Module
    variant: note
---

Lua runs embedded inside a C glue layer compiled to WebAssembly via **Emscripten**. This gives you Lua's lightweight scripting with direct access to the full Flow-Like host API. It produces a [Core Module](/dev/wasm-nodes/runtime-models/) — not a Component Model module.

:::note
This is the most complex build pipeline of all language templates. CMake automatically downloads Lua 5.4 source, embeds your `.lua` files as C string constants, and links everything into a single `.wasm` binary.
:::

## Prerequisites

- [Emscripten SDK (emsdk)](https://emscripten.org/docs/getting_started/downloads.html)
- CMake 3.14+

### Installing Emscripten

```bash
git clone https://github.com/emscripten-core/emsdk.git
cd emsdk
./emsdk install latest
./emsdk activate latest
source ./emsdk_env.sh
```

## How It Works

```
sdk.lua + node.lua ─→ embed as C string arrays ─┐
                                                  ├─→ Emscripten ─→ node.wasm
Lua 5.4 source (static lib) + glue.c ───────────┘
```

1. **CMake fetches Lua 5.4.7** source automatically (no system install needed).
2. Lua is compiled as a **static library** for WASM. `liolib.c` and `loslib.c` are excluded since OS syscalls are unavailable in bare WASM.
3. Your `src/node.lua` and the SDK's `sdk.lua` are **embedded as C string constants** at build time.
4. The C glue layer (`glue.c`) initialises a Lua state, loads both scripts, and bridges WASM exports (`get_node`, `get_nodes`, `run`, `alloc`, `dealloc`) to global Lua functions.
5. Host FFI functions (`flowlike_*` imported from the `env` module) are exposed to Lua via the `sdk` module.

Key Emscripten flags: `-sSTANDALONE_WASM`, `-sALLOW_MEMORY_GROWTH=1`, `-sSUPPORT_LONGJMP=emscripten`, `--no-entry`, `-O2`.

## Template Code

Edit `src/node.lua`. Define three global functions:

```lua title="src/node.lua"
local sdk = require("sdk")

-- 1. Define the node
function get_node()
    local def = sdk.newNodeDefinition()
    def.name          = "my_custom_node_lua"
    def.friendly_name = "My Custom Node (Lua)"
    def.description   = "A template WASM node built with Lua"
    def.category      = "Custom/WASM"
    sdk.addPermission(def, "streaming")

    -- Input pins
    sdk.addPin(def, sdk.inputExec())
    sdk.addPin(def, sdk.withDefault(
        sdk.inputPin("input_text", "Input Text", "Text to process", sdk.DataType.String),
        '""'
    ))
    sdk.addPin(def, sdk.withDefault(
        sdk.inputPin("multiplier", "Multiplier", "Number of times to repeat", sdk.DataType.I64),
        "1"
    ))

    -- Output pins
    sdk.addPin(def, sdk.outputExec())
    sdk.addPin(def, sdk.outputPin("output_text", "Output Text", "Processed text", sdk.DataType.String))
    sdk.addPin(def, sdk.outputPin("char_count", "Character Count", "Number of characters in output", sdk.DataType.I64))

    return sdk.serializeDefinition(def)
end

-- 2. List all nodes
function get_nodes()
    return "[" .. get_node() .. "]"
end

-- 3. Execute logic
function run_node(raw_json)
    local input = sdk.parseInput(raw_json)
    local ctx = sdk.newContext(input)

    local inputText = ctx:getString("input_text", "")
    local multiplier = ctx:getI64("multiplier", 1)
    if multiplier < 0 then multiplier = 0 end

    ctx:debug("Processing: '" .. inputText .. "' x " .. tostring(multiplier))

    local parts = {}
    for i = 1, multiplier do
        parts[i] = inputText
    end
    local outputText = table.concat(parts)
    local charCount = #outputText

    ctx:streamText("Generated " .. tostring(charCount) .. " characters")

    ctx:setOutput("output_text", sdk.jsonString(outputText))
    ctx:setOutput("char_count", tostring(charCount))

    local result = ctx:success()
    return sdk.serializeResult(result)
end
```

## Build

```bash
mkdir -p build && cd build
emcmake cmake ..
emmake make
```

Output: `build/node.wasm`

Or using [mise](https://mise.jdx.dev/):

```bash
mise run build   # runs setup + build
```

## SDK API

### Context

```lua
local input = sdk.parseInput(raw_json)
local ctx = sdk.newContext(input)

-- Read inputs
local s = ctx:getString("name", "default")
local n = ctx:getI64("count", 0)
local d = ctx:getF64("ratio", 1.0)
local b = ctx:getBool("flag", false)
local r = ctx:getRaw("data")

-- Write outputs (values must be valid JSON)
ctx:setOutput("text", sdk.jsonString("hello"))
ctx:setOutput("count", tostring(42))
ctx:setOutput("flag", "true")

-- Logging
ctx:debug("verbose info")
ctx:info("normal info")
ctx:warn("warning")
ctx:error("error")

-- Streaming (only sent when streaming is enabled)
ctx:streamText("progress update")
ctx:streamProgress(0.5, "Halfway done")
ctx:streamJson('{"key":"value"}')

-- Finalize
return ctx:success()   -- activates exec_out + finish
return ctx:fail("msg") -- sets error + finish
```

### Pin Types

| DataType | Lua Access | Description |
|----------|------------|-------------|
| `Exec` | — | Execution flow trigger |
| `String` | `sdk.DataType.String` | UTF-8 string |
| `I64` | `sdk.DataType.I64` | 64-bit integer |
| `F64` | `sdk.DataType.F64` | 64-bit float |
| `Bool` | `sdk.DataType.Bool` | Boolean |
| `Generic` | `sdk.DataType.Generic` | Any JSON value |
| `Bytes` | `sdk.DataType.Bytes` | Raw bytes |
| `Date` | `sdk.DataType.Date` | ISO 8601 date |
| `PathBuf` | `sdk.DataType.PathBuf` | File path |
| `Struct` | `sdk.DataType.Struct` | JSON object |

### Host Wrappers

The C glue layer imports `flowlike_*` functions from the `env` module and exposes them to Lua through the `sdk` module:

| Category | Functions |
|----------|-----------|
| **Logging** | `logTrace`, `logDebug`, `logInfo`, `logWarn`, `logError`, `logJson` |
| **Pins** | `getInput`, `setOutput`, `activateExec` |
| **Variables** | `varGet`, `varSet`, `varDelete`, `varHas` |
| **Cache** | `cacheGet`, `cacheSet`, `cacheDelete`, `cacheHas` |
| **Metadata** | `metaNodeId`, `metaRunId`, `metaAppId`, `metaBoardId`, `metaUserId` |
| **Storage** | `storageRead`, `storageWrite`, `storageDir`, `storageList` |
| **Streaming** | `streamText`, `streamEmit` |
| **HTTP** | `httpRequest` |
| **Auth** | `oauthGetToken`, `oauthHasToken` |
| **Models** | `embedText` |

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `emcmake` not found | Run `source /path/to/emsdk/emsdk_env.sh` |
| Linker errors about missing host functions | `-sERROR_ON_UNDEFINED_SYMBOLS=0` is already set in CMakeLists.txt |
| WASM too large | Lua interpreter adds ~200 KB; try `-Os` instead of `-O2` |
| Runtime Lua error | Ensure `get_node`, `get_nodes`, `run_node` are defined as globals |

## Related

→ [WASM Nodes Overview](/dev/wasm-nodes/overview/)
→ [Component Model vs Core Modules](/dev/wasm-nodes/runtime-models/)
→ [C/C++ Template](/dev/wasm-nodes/cpp/)
→ [Rust Template](/dev/wasm-nodes/rust/)
