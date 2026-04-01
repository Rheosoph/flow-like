---
title: TypeScript WASM Nodes
description: Create custom WASM nodes using TypeScript with the WASM Component Model
sidebar:
  order: 3
  badge:
    text: Component Model
    variant: success
---

TypeScript can compile to WASM using **AssemblyScript** (TypeScript-like syntax) or **Javy** (full JavaScript/TypeScript).

## Option 1: AssemblyScript (Recommended)

AssemblyScript is a TypeScript-like language that compiles directly to WASM with small output sizes.

### Setup

```bash
mkdir my-custom-node
cd my-custom-node
npm init -y
npm install --save-dev assemblyscript
npx asinit .
```

### Template Code

```typescript title="assembly/index.ts"
// Memory management
let resultBuffer: ArrayBuffer;

class NodeDefinition {
  name: string;
  friendly_name: string;
  description: string;
  category: string;
  icon: string;
  pins: PinDefinition[];
}

class PinDefinition {
  name: string;
  friendly_name: string;
  description: string;
  pin_type: string;
  data_type: string;
  default_value: string | null;
}

class ExecutionContext {
  inputs: Map<string, string>;
}

class ExecutionResult {
  outputs: Map<string, string>;
  error: string | null;
}

// Export: get_node
export function get_node(): usize {
  const json = `{
    "name": "wasm_ts_uppercase",
    "friendly_name": "Uppercase (TS)",
    "description": "Converts a string to uppercase using TypeScript",
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
      "performance": 1,
      "governance": 0,
      "reliability": 0,
      "cost": 0
    }
  }`;

  return changetype<usize>(String.UTF8.encode(json));
}

// Export: run
export function run(contextPtr: usize, contextLen: u32): usize {
  // Read context from memory
  const contextBytes = new Uint8Array(contextLen);
  memory.copy(
    changetype<usize>(contextBytes.buffer),
    contextPtr,
    contextLen
  );

  const contextJson = String.UTF8.decode(contextBytes.buffer);

  // Simple JSON parsing (AssemblyScript has limited JSON support)
  // In production, use a JSON library like @assemblyscript/json
  const inputMatch = contextJson.match(/"input"\s*:\s*"([^"]*)"/);
  const input = inputMatch ? inputMatch[1] : "";

  // Execute logic
  const output = input.toUpperCase();

  // Return result
  const result = `{"outputs":{"output":"${output}"},"error":null}`;
  return changetype<usize>(String.UTF8.encode(result));
}
```

### Build

```bash
npm run asbuild:release
```

Output: `build/release.wasm`

### Configuration

Update `asconfig.json` for optimized builds:

```json title="asconfig.json"
{
  "targets": {
    "release": {
      "outFile": "build/release.wasm",
      "optimizeLevel": 3,
      "shrinkLevel": 2,
      "noAssert": true
    }
  }
}
```

---

## Option 2: Javy (Full TypeScript/JavaScript)

Javy compiles full JavaScript to WASM, supporting the entire language but with larger binary sizes.

### Setup

```bash
# Install Javy
# macOS
brew install aspect-build/aspect/javy

# Or download from releases
# https://github.com/aspect-build/aspect-cli/releases
```

### Template Code

```typescript title="src/index.ts"
// Node definition
const nodeDefinition = {
  name: "wasm_javy_uppercase",
  friendly_name: "Uppercase (Javy)",
  description: "Converts a string to uppercase using Javy",
  category: "Custom/Text",
  icon: "/flow/icons/text.svg",
  pins: [
    {
      name: "exec_in",
      friendly_name: "▶",
      description: "Trigger execution",
      pin_type: "Input",
      data_type: "Execution",
    },
    {
      name: "exec_out",
      friendly_name: "▶",
      description: "Continue execution",
      pin_type: "Output",
      data_type: "Execution",
    },
    {
      name: "input",
      friendly_name: "Input",
      description: "The string to convert",
      pin_type: "Input",
      data_type: "String",
      default_value: "",
    },
    {
      name: "output",
      friendly_name: "Output",
      description: "The uppercase string",
      pin_type: "Output",
      data_type: "String",
    },
  ],
  scores: {
    privacy: 0,
    security: 0,
    performance: 2,
    governance: 0,
    reliability: 0,
    cost: 0,
  },
};

// Javy uses stdin/stdout for I/O
const decoder = new TextDecoder();
const encoder = new TextEncoder();

// Read operation from stdin
const input = Javy.IO.readSync(0);
const request = JSON.parse(decoder.decode(input));

let response: any;

if (request.operation === "get_node") {
  response = nodeDefinition;
} else if (request.operation === "run") {
  const context = request.context;
  const inputValue = context.inputs?.input || "";

  response = {
    outputs: {
      output: inputValue.toUpperCase(),
    },
    error: null,
  };
} else {
  response = { error: "Unknown operation" };
}

// Write response to stdout
const outputBytes = encoder.encode(JSON.stringify(response));
Javy.IO.writeSync(1, outputBytes);
```

### Build

First compile TypeScript:

```bash
npx tsc --outDir dist
```

Then compile to WASM:

```bash
javy build -o my-node.wasm dist/index.js
```

### Optimize with Javy

```bash
javy build -o my-node.wasm dist/index.js -O
```

---

## Size Comparison

| Compiler | Typical Size | Full TS Support |
|----------|--------------|-----------------|
| AssemblyScript | 10-100 KB | Limited |
| Javy | 500 KB - 2 MB | Full |

## When to Use What

| Use Case | Recommended |
|----------|-------------|
| Simple transformations | AssemblyScript |
| Complex logic, npm packages | Javy |
| Smallest possible binary | AssemblyScript |
| Rapid prototyping | Javy |

## Install

```bash
cp build/release.wasm ~/.flow-like/nodes/my-custom-node.wasm
```

## Related

→ [WASM Nodes Overview](/dev/wasm-nodes/overview/)
→ [Rust Template](/dev/wasm-nodes/rust/)
→ [Go Template](/dev/wasm-nodes/go/)
→ [C/C++ Template](/dev/wasm-nodes/cpp/)
