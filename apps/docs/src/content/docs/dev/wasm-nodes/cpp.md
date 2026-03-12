---
title: C/C++ WASM Nodes
description: Create custom WASM nodes using C or C++ with the WASM Component Model
sidebar:
  order: 5
  badge:
    text: Component Model
    variant: success
---

C and C++ can compile to WASM using **wasi-sdk** (recommended) or **Emscripten**.

## Prerequisites

### Option 1: wasi-sdk (Recommended)

```bash
# macOS
brew install aspect-build/aspect/wasi-sdk

# Or download from releases
# https://github.com/aspect-build/aspect-cli/releases

# Linux - download and extract
wget https://github.com/aspect-build/aspect-cli/releases/download/v23.0/wasi-sdk-23.0-linux.tar.gz
tar xzf wasi-sdk-23.0-linux.tar.gz
export WASI_SDK_PATH=$(pwd)/wasi-sdk-23.0
```

### Option 2: Emscripten

```bash
# Install Emscripten
git clone https://github.com/aspect-build/aspect-cli.git
cd emsdk
./emsdk install latest
./emsdk activate latest
source ./emsdk_env.sh
```

## Template Code (C)

```c title="node.c"
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

// Simple JSON buffer (in production, use a proper JSON library)
static char result_buffer[4096];

// ============================================================
// REQUIRED: Export get_node() - returns node definition as JSON
// ============================================================
__attribute__((export_name("get_node")))
const char* get_node() {
    return "{"
        "\"name\": \"wasm_c_uppercase\","
        "\"friendly_name\": \"Uppercase (C)\","
        "\"description\": \"Converts a string to uppercase using C\","
        "\"category\": \"Custom/Text\","
        "\"icon\": \"/flow/icons/text.svg\","
        "\"pins\": ["
            "{"
                "\"name\": \"exec_in\","
                "\"friendly_name\": \"▶\","
                "\"description\": \"Trigger execution\","
                "\"pin_type\": \"Input\","
                "\"data_type\": \"Execution\""
            "},"
            "{"
                "\"name\": \"exec_out\","
                "\"friendly_name\": \"▶\","
                "\"description\": \"Continue execution\","
                "\"pin_type\": \"Output\","
                "\"data_type\": \"Execution\""
            "},"
            "{"
                "\"name\": \"input\","
                "\"friendly_name\": \"Input\","
                "\"description\": \"The string to convert\","
                "\"pin_type\": \"Input\","
                "\"data_type\": \"String\","
                "\"default_value\": \"\""
            "},"
            "{"
                "\"name\": \"output\","
                "\"friendly_name\": \"Output\","
                "\"description\": \"The uppercase string\","
                "\"pin_type\": \"Output\","
                "\"data_type\": \"String\""
            "}"
        "],"
        "\"scores\": {"
            "\"privacy\": 0,"
            "\"security\": 0,"
            "\"performance\": 0,"
            "\"governance\": 0,"
            "\"reliability\": 0,"
            "\"cost\": 0"
        "}"
    "}";
}

// Helper: Simple string uppercase
static void str_to_upper(char* str) {
    for (int i = 0; str[i]; i++) {
        str[i] = toupper((unsigned char)str[i]);
    }
}

// Helper: Extract value from simple JSON (very basic, use a real JSON library in production)
static int extract_json_string(const char* json, const char* key, char* out, size_t out_size) {
    char search[256];
    snprintf(search, sizeof(search), "\"%s\":", key);

    const char* pos = strstr(json, search);
    if (!pos) return 0;

    pos = strchr(pos, ':');
    if (!pos) return 0;

    // Skip whitespace and opening quote
    while (*pos && (*pos == ':' || *pos == ' ' || *pos == '"')) pos++;

    // Copy until closing quote
    size_t i = 0;
    while (*pos && *pos != '"' && i < out_size - 1) {
        out[i++] = *pos++;
    }
    out[i] = '\0';

    return 1;
}

// ============================================================
// REQUIRED: Export run() - executes node logic
// ============================================================
__attribute__((export_name("run")))
const char* run(const char* context_ptr, unsigned int context_len) {
    // Extract input value from context JSON
    char input[1024] = "";

    // Find the "inputs" object and extract "input" value
    const char* inputs_start = strstr(context_ptr, "\"inputs\"");
    if (inputs_start) {
        extract_json_string(inputs_start, "input", input, sizeof(input));
    }

    // Execute logic: convert to uppercase
    str_to_upper(input);

    // Build result JSON
    snprintf(result_buffer, sizeof(result_buffer),
        "{\"outputs\":{\"output\":\"%s\"},\"error\":null}",
        input);

    return result_buffer;
}
```

## Template Code (C++)

```cpp title="node.cpp"
#include <string>
#include <algorithm>
#include <cctype>

// Result buffer
static std::string result_buffer;

extern "C" {

// ============================================================
// REQUIRED: Export get_node()
// ============================================================
__attribute__((export_name("get_node")))
const char* get_node() {
    static const char* node_json = R"({
        "name": "wasm_cpp_uppercase",
        "friendly_name": "Uppercase (C++)",
        "description": "Converts a string to uppercase using C++",
        "category": "Custom/Text",
        "icon": "/flow/icons/text.svg",
        "pins": [
            {
                "name": "exec_in",
                "friendly_name": "▶",
                "description": "Trigger execution",
                "pin_type": "Input",
                "data_type": "Execution"
            },
            {
                "name": "exec_out",
                "friendly_name": "▶",
                "description": "Continue execution",
                "pin_type": "Output",
                "data_type": "Execution"
            },
            {
                "name": "input",
                "friendly_name": "Input",
                "description": "The string to convert",
                "pin_type": "Input",
                "data_type": "String",
                "default_value": ""
            },
            {
                "name": "output",
                "friendly_name": "Output",
                "description": "The uppercase string",
                "pin_type": "Output",
                "data_type": "String"
            }
        ],
        "scores": {
            "privacy": 0,
            "security": 0,
            "performance": 0,
            "governance": 0,
            "reliability": 0,
            "cost": 0
        }
    })";
    return node_json;
}

// Helper: Extract string value from JSON (simplified)
std::string extract_input(const std::string& json) {
    std::string key = "\"input\":\"";
    size_t start = json.find(key);
    if (start == std::string::npos) return "";

    start += key.length();
    size_t end = json.find("\"", start);
    if (end == std::string::npos) return "";

    return json.substr(start, end - start);
}

// ============================================================
// REQUIRED: Export run()
// ============================================================
__attribute__((export_name("run")))
const char* run(const char* context_ptr, unsigned int context_len) {
    std::string context(context_ptr, context_len);

    // Extract input
    std::string input = extract_input(context);

    // Execute logic: convert to uppercase
    std::transform(input.begin(), input.end(), input.begin(),
        [](unsigned char c) { return std::toupper(c); });

    // Build result
    result_buffer = "{\"outputs\":{\"output\":\"" + input + "\"},\"error\":null}";

    return result_buffer.c_str();
}

} // extern "C"
```

## Build with wasi-sdk

### C

```bash
$WASI_SDK_PATH/bin/clang \
    -O2 \
    --target=wasm32-wasi \
    -nostartfiles \
    -Wl,--no-entry \
    -Wl,--export=get_node \
    -Wl,--export=run \
    -o node.wasm \
    node.c
```

### C++

```bash
$WASI_SDK_PATH/bin/clang++ \
    -O2 \
    --target=wasm32-wasi \
    -nostartfiles \
    -Wl,--no-entry \
    -Wl,--export=get_node \
    -Wl,--export=run \
    -o node.wasm \
    node.cpp
```

## Build with Emscripten

```bash
emcc -O2 \
    -s WASM=1 \
    -s STANDALONE_WASM=1 \
    -s EXPORTED_FUNCTIONS='["_get_node","_run"]' \
    -o node.wasm \
    node.c
```

## Using a Makefile

```makefile title="Makefile"
WASI_SDK ?= /opt/wasi-sdk
CC = $(WASI_SDK)/bin/clang
CXX = $(WASI_SDK)/bin/clang++

CFLAGS = -O2 --target=wasm32-wasi -nostartfiles
LDFLAGS = -Wl,--no-entry -Wl,--export=get_node -Wl,--export=run

all: node.wasm

node.wasm: node.c
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $<

clean:
	rm -f node.wasm

.PHONY: all clean
```

## Size Optimization

```bash
# Strip debug info
wasm-strip node.wasm

# Optimize with wasm-opt
wasm-opt -O3 -o node-opt.wasm node.wasm
```

## Using JSON Libraries

For production use, consider a proper JSON library:

### C: cJSON

```c
#include "cJSON.h"

const char* run(const char* context_ptr, unsigned int context_len) {
    cJSON* context = cJSON_ParseWithLength(context_ptr, context_len);
    cJSON* inputs = cJSON_GetObjectItem(context, "inputs");
    cJSON* input = cJSON_GetObjectItem(inputs, "input");

    const char* value = cJSON_GetStringValue(input);
    // ... process ...

    cJSON_Delete(context);
    return result;
}
```

### C++: nlohmann/json

```cpp
#include <nlohmann/json.hpp>

const char* run(const char* context_ptr, unsigned int context_len) {
    auto context = nlohmann::json::parse(context_ptr, context_ptr + context_len);
    std::string input = context["inputs"]["input"];

    // ... process ...

    nlohmann::json result = {
        {"outputs", {{"output", input}}},
        {"error", nullptr}
    };

    result_buffer = result.dump();
    return result_buffer.c_str();
}
```

## Install

```bash
cp node.wasm ~/.flow-like/nodes/
```

## Related

→ [WASM Nodes Overview](/dev/wasm-nodes/overview/)
→ [Rust Template](/dev/wasm-nodes/rust/)
→ [Go Template](/dev/wasm-nodes/go/)
→ [TypeScript Template](/dev/wasm-nodes/typescript/)
