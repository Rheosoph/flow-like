# flow-like-wasm-node-nim

A template for creating [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes in **Nim**.

Nim compiles to C, then Emscripten compiles the C output to WebAssembly. This gives you Nim's expressive syntax with near-native WASM performance.

## Prerequisites

- **Nim** >= 2.0 (`brew install nim` or [choosenim](https://github.com/dom96/choosenim))
- **Emscripten** SDK ([setup guide](https://emscripten.org/docs/getting_started/downloads.html))
- **mise** (optional, for task runner)

## Quick Start

```bash
# Install SDK dependency (uses local path in monorepo, or nimble for standalone)
nimble install -y

# Build the WASM node
nimble build

# Run tests (native, not WASM)
nimble test
```

Or with mise:

```bash
mise run setup
mise run build
mise run test
```

## Project Structure

```
├── flow-like.toml          # Package manifest (id, metadata, node list)
├── node.nimble             # Nim package config + build task
├── nim.cfg                 # Compiler paths (local SDK reference)
├── mise.toml               # Task runner config
├── src/
│   ├── node.nim            # ← Your node logic (edit this!)
│   └── sdk.nim             # SDK re-export shim
├── examples/
│   └── http_request.nim    # HTTP permission example
├── tests/
│   └── test_node.nim       # Unit tests (native)
└── .github/workflows/
    └── build.yml           # CI: build + test + release
```

## SDK Features

| Module | Functions |
|--------|-----------|
| **Logging** | `logTrace`, `logDebug`, `logInfo`, `logWarn`, `logError` |
| **Pins** | `getInput`, `setOutput`, `activateExec` |
| **Variables** | `varGet`, `varSet`, `varDelete`, `varHas` |
| **Cache** | `cacheGet`, `cacheSet`, `cacheDelete`, `cacheHas` |
| **Metadata** | `metaNodeId`, `metaRunId`, `metaAppId`, `metaBoardId`, `metaUserId` |
| **Storage** | `storageRead`, `storageWrite`, `storageDir`, `storageList` |
| **Streaming** | `streamText`, `streamEmit`, `streamProgress` |
| **HTTP** | `httpRequest` |
| **Auth** | `oauthGetToken`, `oauthHasToken` |

## Pin Types

| Type | Nim Enum | Description |
|------|----------|-------------|
| Exec | `Exec` | Execution flow trigger |
| String | `String` | UTF-8 string |
| I64 | `I64` | 64-bit integer |
| F64 | `F64` | 64-bit float |
| Bool | `Bool` | Boolean |
| Generic | `Generic` | Any JSON value |
| Bytes | `Bytes` | Raw bytes |
| Date | `Date` | ISO 8601 date |
| PathBuf | `PathBuf` | File path |
| Struct | `Struct` | JSON object |

## Creating a Node

Edit `src/node.nim`:

1. Define your node with `initNodeDefinition()` and add pins
2. Implement `handleRun(ctx: var Context): ExecutionResult` with your logic
3. Export `get_node`, `get_nodes`, `run` procs

The `Context` provides typed input getters, output setters, level-gated logging, and conditional streaming.

## Production Build

The default `nimble build` task already uses optimized Emscripten flags:
- `-d:release` + `--gc:arc` for minimal binary size
- `ALLOW_MEMORY_GROWTH=1` for dynamic memory
- `-O2` optimization
- `STANDALONE_WASM` mode

## Publishing

1. Build your WASM file: `nimble build`
2. Update `flow-like.toml` with your package ID and metadata
3. Bundle `build/node.wasm` + `flow-like.toml` into a `.tar.gz`
4. Upload to the Flow-Like registry or use GitHub Releases

## License

MIT
