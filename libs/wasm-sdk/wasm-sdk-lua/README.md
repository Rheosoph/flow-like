# flow-like-wasm-sdk-lua

Lua SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes. Lua runs embedded in a C glue layer compiled to WebAssembly via Emscripten. This gives you Lua's lightweight scripting with the full Flow-Like host API.

## Prerequisites

**Lua 5.4 source** (compiled to WASM alongside your node)

```bash
# Download Lua source
curl -R -O https://www.lua.org/ftp/lua-5.4.7.tar.gz
tar zxf lua-5.4.7.tar.gz
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

```lua
local sdk = require("sdk")

function get_node()
    local def = sdk.newNodeDefinition()
    def.name          = "uppercase"
    def.friendly_name = "Uppercase"
    def.description   = "Converts a string to uppercase"
    def.category      = "Text/Transform"

    sdk.addPin(def, sdk.inputExec())
    sdk.addPin(def, sdk.inputPin("text", "Text", "Input string", sdk.DataType.String))
    sdk.addPin(def, sdk.outputExec())
    sdk.addPin(def, sdk.outputPin("result", "Result", "Uppercased text", sdk.DataType.String))

    return sdk.serializeDefinition(def)
end

function get_nodes()
    return "[" .. get_node() .. "]"
end

function run_node(raw_json)
    local input = sdk.parseInput(raw_json)
    local ctx = sdk.newContext(input)

    local text = ctx:getString("text", "")
    ctx:setOutput("result", sdk.jsonString(string.upper(text)))

    local result = ctx:success()
    return sdk.serializeResult(result)
end
```

## SDK Architecture

```
src/
  sdk.lua  — Pure Lua SDK: types, JSON helpers, context, host wrappers
  glue.c   — C glue: WASM imports/exports, Lua state management, host bridge
```

The compilation pipeline is: **Lua + C glue → Emscripten → WASM**

The C glue creates a Lua state, registers all host functions as `flowlike_host.*`, loads the SDK, then loads your node script. WASM exports (`get_node`, `get_nodes`, `run`) delegate to global Lua functions.

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
| `ctx:getString(name)` | Get string input (strips JSON quotes) |
| `ctx:getI64(name)` | Get 64-bit integer input |
| `ctx:getF64(name)` | Get 64-bit float input |
| `ctx:getBool(name)` | Get boolean input |
| `ctx:getRaw(name)` | Get raw JSON value |
| `ctx:setOutput(name, json)` | Set output pin value |
| `ctx:activateExec(pin)` | Activate an exec output pin |
| `ctx:success()` | Activate `exec_out` and finalize |
| `ctx:fail(msg)` | Set error and finalize |
| `ctx:debug(msg)` | Log debug (level-gated) |
| `ctx:info(msg)` | Log info (level-gated) |
| `ctx:warn(msg)` | Log warning (level-gated) |
| `ctx:error(msg)` | Log error (level-gated) |
| `ctx:streamText(text)` | Stream text (if streaming enabled) |
| `ctx:streamJson(json)` | Stream JSON (if streaming enabled) |
| `ctx:streamProgress(pct, msg)` | Stream progress (if streaming enabled) |

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

### Option 1: Embed Lua source as strings

```bash
# Generate C string literals from Lua sources
xxd -i src/sdk.lua > build/sdk_lua.c
xxd -i src/node.lua > build/node_lua.c

# Compile with Lua source + glue
emcc src/glue.c \
     lua-5.4.7/src/lapi.c lua-5.4.7/src/lcode.c lua-5.4.7/src/lctype.c \
     lua-5.4.7/src/ldebug.c lua-5.4.7/src/ldo.c lua-5.4.7/src/ldump.c \
     lua-5.4.7/src/lfunc.c lua-5.4.7/src/lgc.c lua-5.4.7/src/llex.c \
     lua-5.4.7/src/lmem.c lua-5.4.7/src/lobject.c lua-5.4.7/src/lopcodes.c \
     lua-5.4.7/src/lparser.c lua-5.4.7/src/lstate.c lua-5.4.7/src/lstring.c \
     lua-5.4.7/src/ltable.c lua-5.4.7/src/ltm.c lua-5.4.7/src/lundump.c \
     lua-5.4.7/src/lvm.c lua-5.4.7/src/lzio.c lua-5.4.7/src/lauxlib.c \
     lua-5.4.7/src/lbaselib.c lua-5.4.7/src/lcorolib.c lua-5.4.7/src/ldblib.c \
     lua-5.4.7/src/liolib.c lua-5.4.7/src/lmathlib.c lua-5.4.7/src/loadlib.c \
     lua-5.4.7/src/loslib.c lua-5.4.7/src/lstrlib.c lua-5.4.7/src/ltablib.c \
     lua-5.4.7/src/lutf8lib.c lua-5.4.7/src/linit.c \
     -I lua-5.4.7/src \
     -o build/node.wasm \
     -O2 \
     -s STANDALONE_WASM \
     -s EXPORTED_FUNCTIONS='["_get_node","_get_nodes","_run","_alloc","_dealloc","_get_abi_version"]' \
     --no-entry
```

### Option 2: Use Emscripten virtual filesystem

```bash
emcc src/glue.c \
     lua-5.4.7/src/*.c \
     -I lua-5.4.7/src \
     -o build/node.wasm \
     -O2 \
     -s STANDALONE_WASM \
     -s EXPORTED_FUNCTIONS='["_get_node","_get_nodes","_run","_alloc","_dealloc","_get_abi_version"]' \
     --preload-file src/sdk.lua@sdk.lua \
     --preload-file src/node.lua@node.lua \
     --no-entry
```

Key flags:
- `-I lua-5.4.7/src` — Lua headers
- `-O2` — optimize for size and speed
- `STANDALONE_WASM` — no JS glue needed
- `--no-entry` — library mode, no main()

## Notes

- The SDK is pure Lua with no external dependencies. JSON serialization/parsing is hand-rolled.
- The C glue handles all WASM import/export declarations and Lua state lifecycle.
- Lua source can be embedded as C string literals or loaded from the Emscripten virtual filesystem.
- Use `-Os` for smaller binaries; the Lua interpreter adds ~200KB to the WASM output.

## License

MIT
