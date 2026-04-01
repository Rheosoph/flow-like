# Flow-Like WASM Node Template (AssemblyScript)

This template provides a starting point for creating custom WASM nodes using AssemblyScript.

## Prerequisites

- Node.js 18+
- npm or yarn

## Quick Start

1. **Install dependencies:**
   ```bash
   npm install
   ```

2. **Build the WASM module:**
   ```bash
   npm run build
   ```

3. **Copy the output to your Flow-Like project:**
   ```bash
   cp build/release.wasm /path/to/flow-like/wasm-nodes/
   ```

## Project Structure

```
wasm-node-assemblyscript/
├── assembly/
│   ├── index.ts      # Main node implementation
│   └── examples.ts   # Example node bundle entrypoint
├── examples/
│   ├── math_nodes.ts     # Arithmetic operations (add, subtract, multiply, divide)
│   ├── string_nodes.ts   # Text manipulation (uppercase, trim, replace, concat)
│   └── control_flow.ts   # Logic and branching (if, compare, gates)
├── build/
│   └── release.wasm  # Compiled output (after build)
├── asconfig.json     # AssemblyScript configuration
├── package.json      # Node.js package config
└── tsconfig.json     # TypeScript configuration
```

## Example Node Catalog

The `examples/` directory contains ready-to-use node implementations:

### Math Nodes (`examples/math_nodes.ts`)

| Node | Description |
|------|-------------|
| `math_add_as` | Adds two numbers |
| `math_subtract_as` | Subtracts B from A |
| `math_multiply_as` | Multiplies two numbers |
| `math_divide_as` | Divides A by B (with zero check) |
| `math_clamp_as` | Clamps value between min and max |

### String Nodes (`examples/string_nodes.ts`)

| Node | Description |
|------|-------------|
| `string_uppercase_as` | Converts text to uppercase |
| `string_lowercase_as` | Converts text to lowercase |
| `string_trim_as` | Removes leading/trailing whitespace |
| `string_length_as` | Returns string length and empty check |
| `string_contains_as` | Checks if text contains substring |
| `string_replace_as` | Replaces pattern occurrences |
| `string_concat_as` | Joins strings with separator |

### Control Flow Nodes (`examples/control_flow.ts`)

| Node | Description |
|------|-------------|
| `if_branch_as` | Branches based on boolean condition |
| `compare_as` | Compares two numbers |
| `and_gate_as` | Logical AND of two booleans |
| `or_gate_as` | Logical OR of two booleans |
| `not_gate_as` | Inverts a boolean |
| `gate_as` | Passes execution only if open |

### Using Examples

Copy example functions to your `assembly/index.ts`:

```typescript
// Import from examples
export { get_add_definition, run_add } from "./examples/math_nodes";
export { get_uppercase_definition, run_uppercase } from "./examples/string_nodes";
```

Or use as reference when building your own nodes.

## Creating Your Node

### 1. Define the Node

Edit `assembly/index.ts` and modify the `get_definition()` function:

```typescript
export function get_definition(): i64 {
  const def = new NodeDefinition();
  def.name = "your_node_name";
  def.friendly_name = "Your Node";
  def.description = "What your node does";
  def.category = "Custom/YourCategory";

  // Add inputs
  def.addPin(PinDefinition.input("exec", "Execute", "Trigger", "Exec"));
  def.addPin(PinDefinition.input("my_input", "My Input", "Description", "String"));

  // Add outputs
  def.addPin(PinDefinition.output("exec_out", "Done", "Complete", "Exec"));
  def.addPin(PinDefinition.output("result", "Result", "Description", "String"));

  return serializeDefinition(def);
}
```

### 2. Implement the Logic

Modify the `run()` function:

```typescript
export function run(ptr: i32, len: i32): i64 {
  const input = parseInput(ptr, len);

  // Your logic here
  const myInput = getStringInput(input, "my_input", "default");
  const result = myInput.toUpperCase();

  // Return result
  const output = ExecutionResult.success();
  output.setOutput("result", JSON.stringify(result));
  output.activateExec("exec_out");

  return serializeResult(output);
}
```

## Available Pin Types

| Type | Description |
|------|-------------|
| `Exec` | Execution flow pin |
| `String` | Text value |
| `I64` | Integer (64-bit) |
| `F64` | Float (64-bit) |
| `Bool` | Boolean |
| `Generic` | JSON/Object |
| `Bytes` | Binary data |

## SDK Functions

### Logging

```typescript
import { debug, info, warn, error } from "@flow-like/wasm-sdk/assembly/index";

debug("Debug message");
info("Info message");
warn("Warning message");
error("Error message");
```

### Streaming

```typescript
import { streamText, streamProgress } from "@flow-like/wasm-sdk/assembly/index";

streamText("Processing...");
streamProgress(0.5, "Halfway done");
```

### Utilities

```typescript
import { now, random } from "@flow-like/wasm-sdk/assembly/index";

const timestamp = now();  // Current time in ms
const rand = random();    // Random i64
```

## Testing

AssemblyScript doesn't have built-in unit testing, but you can:

1. **Test logic in TypeScript** before porting to AssemblyScript
2. **Use as-pect** for AssemblyScript-native testing:
   ```bash
   npm install -D @as-pect/cli
   npx asp --init
   npx asp
   ```

3. **Integration test** by loading the WASM in Flow-Like

Example test with as-pect:

```typescript
// assembly/__tests__/math.spec.ts
import { run_add } from "../examples/math_nodes";

describe("Math Nodes", () => {
  it("should add two numbers", () => {
    // Test your node logic here
    expect(2 + 3).toBe(5);
  });
});
```

## Building for Production

For optimized production builds:

```bash
npm run asbuild:release
```

The output will be optimized with:
- Level 3 optimization
- Level 2 shrinking
- No debug symbols

## Debugging

For development builds with debug info:

```bash
npm run asbuild:debug
```

This produces:
- Source maps
- Debug symbols
- WAT text format

## Publishing to the Registry

Once your package is ready:

1. Navigate to **Library → Packages → Publish** in Flow-Like Desktop
2. Select your compiled `build/release.wasm` file
3. Review and edit the manifest metadata
4. Configure required permissions
5. Submit for review

See the [Registry Documentation](https://docs.flow-like.com/dev/wasm-nodes/registry/) for details on the governance and approval process.
