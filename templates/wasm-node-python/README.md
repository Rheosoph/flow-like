# Flow-Like WASM Node Template (Python)

This template provides a starting point for creating custom WASM nodes using Python.

## Prerequisites

- Python 3.11+
- [uv](https://docs.astral.sh/uv/)

## Quick Start

1. **Install dependencies:**
   ```bash
   uv sync --group dev
   ```

2. **Run the tests:**
   ```bash
   uv run pytest
   ```

3. **Build the node definition:**
   ```bash
   uv run python build.py --definition-only
   ```

4. **Build the WASM module (requires `componentize-py`):**
   ```bash
   uv sync --group build
   uv run python build.py
   ```

## Project Structure

```
wasm-node-python/
├── src/
│   ├── sdk.py            # SDK types and utilities
│   └── node.py           # Main node implementation (edit this)
├── examples/
│   ├── math_nodes.py     # Arithmetic operations (add, subtract, multiply, divide, power, clamp)
│   ├── string_nodes.py   # Text manipulation (uppercase, lowercase, trim, reverse, length, ...)
│   └── control_flow.py   # Logic and branching (if, compare, gates, gate, sequence)
├── tests/
│   ├── conftest.py       # Shared fixtures
│   ├── test_sdk.py       # SDK type tests
│   ├── test_node.py      # Main node tests
│   ├── test_math_nodes.py
│   ├── test_string_nodes.py
│   └── test_control_flow.py
├── build.py              # Build script
├── pyproject.toml        # Python project config (uv)
└── README.md
```

## Example Node Catalog

### Math Nodes (`examples/math_nodes.py`)

| Node | Description |
|------|-------------|
| `math_add_py` | Adds two numbers |
| `math_subtract_py` | Subtracts B from A |
| `math_multiply_py` | Multiplies two numbers |
| `math_divide_py` | Divides A by B (with zero check) |
| `math_power_py` | Raises A to the power of B |
| `math_clamp_py` | Clamps value between min and max |

### String Nodes (`examples/string_nodes.py`)

| Node | Description |
|------|-------------|
| `string_uppercase_py` | Converts text to uppercase |
| `string_lowercase_py` | Converts text to lowercase |
| `string_trim_py` | Removes leading/trailing whitespace |
| `string_length_py` | Returns string length and empty check |
| `string_contains_py` | Checks if text contains substring |
| `string_replace_py` | Replaces pattern occurrences |
| `string_concat_py` | Joins strings with optional separator |
| `string_reverse_py` | Reverses characters in string |

### Control Flow Nodes (`examples/control_flow.py`)

| Node | Description |
|------|-------------|
| `if_branch_py` | Branches based on boolean condition |
| `compare_py` | Compares two values |
| `and_gate_py` | Logical AND of two booleans |
| `or_gate_py` | Logical OR of two booleans |
| `not_gate_py` | Logical NOT |
| `gate_py` | Conditional pass-through |
| `sequence_py` | Activates multiple outputs in order |

## Creating Your Node

### 1. Define the Node

Edit `src/node.py` — return a `NodeDefinition` with your pins:

```python
from sdk import NodeDefinition, PinDefinition, PinType

def get_definition() -> NodeDefinition:
    nd = NodeDefinition(
        name="your_node_name",
        friendly_name="Your Node",
        description="What your node does",
        category="Custom/YourCategory",
    )
    nd.add_pin(PinDefinition.input_exec("exec"))
    nd.add_pin(PinDefinition.input_pin("my_input", PinType.STRING, default=""))
    nd.add_pin(PinDefinition.input_pin("count", PinType.I64, default=1))
    nd.add_pin(PinDefinition.output_exec("exec_out"))
    nd.add_pin(PinDefinition.output_pin("result", PinType.STRING))
    return nd
```

### 2. Implement the Logic

Wire up the `run()` function:

```python
from sdk import Context, ExecutionResult

def run(ctx: Context) -> ExecutionResult:
    my_input = ctx.get_string("my_input") or ""
    count = ctx.get_i64("count") or 1

    result = my_input * count

    ctx.set_output("result", result)
    return ctx.success()
```

### 3. Test

```python
# tests/test_node.py
from conftest import make_context
from node import run

def test_basic():
    result = run(make_context({"my_input": "hello", "count": 3}))
    assert result.outputs["result"] == "hellohellohello"
```

Run:
```bash
uv run pytest -v
```

## Available Pin Types

| Type | Description |
|------|-------------|
| `PinType.EXEC` | Flow control pin |
| `PinType.STRING` | Text value |
| `PinType.I64` | 64-bit integer |
| `PinType.F64` | 64-bit float |
| `PinType.BOOL` | Boolean |
| `PinType.GENERIC` | JSON/Object |
| `PinType.BYTES` | Binary data |

## SDK Features

### Logging (level-gated)
```python
ctx.debug("Low-level detail")
ctx.info("General information")
ctx.warn("Something unusual")
ctx.error("Something went wrong")
```

### Streaming
```python
ctx.stream_text("Processing step 1...")
ctx.stream_progress(0.5, "Halfway done")
ctx.stream_json({"key": "value"})
```

### Variables
```python
ctx.set_variable("counter", 42)
value = ctx.get_variable("counter")
```

### Testing with MockHostBridge
```python
from sdk import MockHostBridge
from conftest import make_context

host = MockHostBridge()
ctx = make_context({"text": "hi"}, host=host, stream=True)
run(ctx)

assert len(host.logs) > 0      # Check logged messages
assert len(host.streams) > 0   # Check streamed data
```

## Building for WASM

The build pipeline uses [componentize-py](https://github.com/bytecodealliance/componentize-py)
to compile your Python node into a WASM Component:

```bash
uv sync --group build
uv run python build.py
```

The output is placed in `build/node.wasm`.

## Publishing

1. Open Flow-Like Desktop
2. Go to **Library → Packages → Publish**
3. Select your `.wasm` file from `build/`
