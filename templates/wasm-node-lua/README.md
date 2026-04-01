# flow-like-wasm-node-lua

A template for creating [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes in **Lua**.

Lua runs embedded in a C glue layer compiled to WebAssembly via Emscripten. This gives you Lua's lightweight scripting with the full Flow-Like host API.

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

## Quick Start

1. **Build the WASM module:**
   ```bash
   mkdir build && cd build
   emcmake cmake ..
   emmake make
   ```

2. **Find the output:**
   ```
   build/node.wasm
   ```

3. **Copy to your Flow-Like project:**
   ```bash
   cp build/node.wasm /path/to/flow-like/wasm-nodes/
   ```

Or with mise:

```bash
mise run setup
mise run build
```

## Project Structure

```
wasm-node-lua/
├── src/
│   └── node.lua              # ← Your node logic (edit this!)
├── examples/
│   └── http_request.lua      # HTTP permission example
├── flow-like.toml            # Package manifest
├── CMakeLists.txt            # Build configuration
├── mise.toml                 # Task runner config
└── README.md

wasm-sdk-lua/                 # SDK (referenced by CMakeLists.txt)
├── src/
│   ├── sdk.lua               # Lua SDK: types, JSON, context, host wrappers
│   └── glue.c                # C glue: WASM imports/exports, Lua state
└── README.md
```

## Build Pipeline

```
sdk.lua + node.lua ─→ embed as C string arrays ─┐
                                                  ├─→ Emscripten ─→ node.wasm
Lua 5.4 source (static lib) + glue.c ───────────┘
```

CMake downloads Lua 5.4 source automatically, converts the `.lua` files into C string constants, and links everything together with the SDK's `glue.c`. The result is a single `node.wasm` with Lua embedded.

## Creating Your Node

Edit `src/node.lua`. You need to define three global functions:

### 1. `get_node()` — Define the node

```lua
local sdk = require("sdk")

function get_node()
    local def = sdk.newNodeDefinition()
    def.name          = "my_node"
    def.friendly_name = "My Node"
    def.description   = "Does something useful"
    def.category      = "Custom/WASM"

    sdk.addPin(def, sdk.inputExec())
    sdk.addPin(def, sdk.inputPin("value", "Value", "Input value", sdk.DataType.String))
    sdk.addPin(def, sdk.outputExec())
    sdk.addPin(def, sdk.outputPin("result", "Result", "Output value", sdk.DataType.String))

    return sdk.serializeDefinition(def)
end
```

### 2. `get_nodes()` — List all nodes

```lua
function get_nodes()
    return "[" .. get_node() .. "]"
end
```

### 3. `run_node(raw_json)` — Execute the logic

```lua
function run_node(raw_json)
    local input = sdk.parseInput(raw_json)
    local ctx = sdk.newContext(input)

    local value = ctx:getString("value", "")
    ctx:setOutput("result", sdk.jsonString(string.upper(value)))

    local result = ctx:success()
    return sdk.serializeResult(result)
end
```

## Pin Types

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

## Context API

```lua
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

-- Logging (level-gated)
ctx:debug("verbose info")
ctx:info("normal info")
ctx:warn("warning")
ctx:error("error")

-- Streaming (only sent when streaming is enabled)
ctx:streamText("progress update")
ctx:streamProgress(0.5, "Halfway done")
ctx:streamJson('{"key":"value"}')

-- Finalize
return ctx:success()  -- activates exec_out + finish
return ctx:fail("msg") -- sets error + finish
```

## SDK Host Wrappers

| Module | Functions |
|--------|-----------|
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

## Emscripten Tips

- Lua 5.4 source is fetched automatically by CMake — no system install needed.
- Use `-O2` (default) for smaller WASM output.
- Memory growth is enabled by default (`-sALLOW_MEMORY_GROWTH=1`).
- The SDK and node Lua files are embedded as C string constants in the binary, so there is no filesystem dependency at runtime.

## Troubleshooting

| Issue | Solution |
|-------|----------|
| `emcmake` not found | Run `source /path/to/emsdk/emsdk_env.sh` |
| Linker errors about missing host functions | `-sERROR_ON_UNDEFINED_SYMBOLS=0` is already set |
| WASM too large | The Lua interpreter adds ~200KB; use `-Os` instead of `-O2` |
| Runtime Lua error | Check that `get_node`, `get_nodes`, `run_node` are defined as globals |

## Publishing

1. Build your WASM file: `mise run build`
2. Update `flow-like.toml` with your package ID and metadata
3. Bundle `build/node.wasm` + `flow-like.toml` into a `.tar.gz`
4. Upload to the Flow-Like registry or use GitHub Releases

## License

MIT
