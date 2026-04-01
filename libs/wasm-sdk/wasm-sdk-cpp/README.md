# flow-like-wasm-sdk-cpp

C++ SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes using [Emscripten](https://emscripten.org/) or [wasi-sdk](https://github.com/WebAssembly/wasi-sdk). C++ gives you full control over memory and performance with zero-overhead abstractions.

## Prerequisites

Install Emscripten:

```bash
git clone https://github.com/emscripten-core/emsdk.git
cd emsdk
./emsdk install latest
./emsdk activate latest
source ./emsdk_env.sh
```

Or install wasi-sdk:

```bash
# macOS
brew install wasi-sdk
```

## Setup

Copy the SDK into your project or reference it via CMake:

```cmake
add_subdirectory(../../libs/wasm-sdk/wasm-sdk-cpp)
target_link_libraries(my_node PRIVATE flow_like_wasm_sdk)
```

Or just copy `include/flow_like_sdk.h` and `src/sdk.cpp` directly into your project.

## Quick Start — Single Node

```cpp
#include "flow_like_sdk.h"
using namespace flowlike;

// Define the node's schema
static NodeDefinition make_definition() {
    NodeDefinition def;
    def.name         = "uppercase";
    def.friendly_name = "Uppercase";
    def.description  = "Converts a string to uppercase";
    def.category     = "Text/Transform";

    def.pins.push_back(PinDefinition::input_exec("exec"));
    def.pins.push_back(PinDefinition::input("text", "Text", "Input string", DataType::String));
    def.pins.push_back(PinDefinition::output_exec("exec_out"));
    def.pins.push_back(PinDefinition::output("result", "Result", "Uppercased text", DataType::String));

    return def;
}

// Node logic
static ExecutionResult run_logic(Context& ctx) {
    std::string text = ctx.get_string("text").value_or("");
    std::transform(text.begin(), text.end(), text.begin(), ::toupper);
    ctx.set_output("result", text);
    return ctx.success("exec_out");
}

// WASM exports
extern "C" {

WASM_EXPORT int64_t get_nodes() {
    auto def = make_definition();
    return pack_result(def.to_json());
}

WASM_EXPORT int64_t run(int32_t ptr, int32_t len) {
    auto input = ExecutionInput::from_wasm(ptr, len);
    Context ctx(input);
    auto result = run_logic(ctx);
    return pack_result(result.to_json());
}

WASM_EXPORT int32_t alloc(int32_t size) { return wasm_alloc(size); }
WASM_EXPORT void dealloc(int32_t ptr, int32_t size) { wasm_dealloc(ptr, size); }
WASM_EXPORT uint32_t get_abi_version() { return 1; }

} // extern "C"
```

## Quick Start — Node Package (multiple nodes)

```cpp
#include "flow_like_sdk.h"
using namespace flowlike;

// Declare nodes
NodeDefinition define_add();
ExecutionResult run_add(Context& ctx);

NodeDefinition define_subtract();
ExecutionResult run_subtract(Context& ctx);

// Package dispatch
static NodePackage pkg = NodePackage::create({
    { define_add,      run_add      },
    { define_subtract, run_subtract },
});

extern "C" {
WASM_EXPORT int64_t get_nodes() { return pkg.get_nodes(); }
WASM_EXPORT int64_t run(int32_t ptr, int32_t len) { return pkg.run(ptr, len); }
// ... alloc/dealloc/get_abi_version
}
```

## Building with Emscripten

```bash
emcc src/my_node.cpp \
     ../../libs/wasm-sdk/wasm-sdk-cpp/src/sdk.cpp \
     -I ../../libs/wasm-sdk/wasm-sdk-cpp/include \
     -o build/my_node.wasm \
     -O2 \
     -fno-exceptions \
     -fno-rtti \
     -s STANDALONE_WASM \
     -s EXPORTED_FUNCTIONS='["_get_nodes","_run","_alloc","_dealloc","_get_abi_version"]'
```

## Building with wasi-sdk

```bash
/opt/wasi-sdk/bin/clang++ \
  src/my_node.cpp \
  ../../libs/wasm-sdk/wasm-sdk-cpp/src/sdk.cpp \
  -I ../../libs/wasm-sdk/wasm-sdk-cpp/include \
  --target=wasm32-wasi \
  -O2 \
  -fno-exceptions \
  -fno-rtti \
  -o build/my_node.wasm
```

## Building with CMake

```bash
mkdir build && cd build
emcmake cmake .. -DCMAKE_BUILD_TYPE=Release
make
```

## API Reference

### `Context`

| Method | Description |
|---|---|
| `get_string(pin)` | Read a string input (`std::optional<std::string>`) |
| `get_bool(pin)` | Read a boolean input (`std::optional<bool>`) |
| `get_i64(pin)` | Read an integer input (`std::optional<int64_t>`) |
| `get_f64(pin)` | Read a float input (`std::optional<double>`) |
| `set_output(pin, value)` | Write an output value |
| `success(execPin)` | Return success result |
| `error(message)` | Return error result |
| `log_debug/info/warn/error(msg)` | Log via host bridge |
| `node_id() / run_id() / app_id()` | Read runtime metadata |

### `PinDefinition` helpers

```cpp
PinDefinition::input_exec("exec")
PinDefinition::output_exec("exec_out")
PinDefinition::input("name", "Friendly", "Desc", DataType::String)
PinDefinition::output("name", "Friendly", "Desc", DataType::Float)
```

### `DataType` enum

`Exec`, `String`, `Boolean`, `Integer`, `Float`, `Json`, `Generic`, `Array`, `HashMap`

## Notes

- Exceptions and RTTI are disabled by default (`-fno-exceptions -fno-rtti`) to reduce binary size.
- The SDK ships a zero-dependency JSON implementation (`src/json.h`) to avoid bringing in large libraries.
- Use `-O2` or `-Os` for release builds; WASM binary size significantly improves at optimization level 2+.
