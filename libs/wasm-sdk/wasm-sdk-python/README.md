# flow-like-wasm-sdk (Python)

Python SDK for building [Flow-Like](https://github.com/TM9657/flow-like) WASM nodes using [componentize-py](https://github.com/bytecodealliance/componentize-py). Write your node logic in plain Python — the SDK handles compilation to a WASM component via the WIT Component Model.

## Install

```bash
pip install flow-like-wasm-sdk
# or with uv
uv add flow-like-wasm-sdk
```

For optional JSON schema support (typed pins with Pydantic):

```bash
pip install "flow-like-wasm-sdk[schema]"
```

## Quick Start — Single Node

```python
from flow_like_wasm_sdk import (
    NodeDefinition,
    PinDefinition,
    Context,
    ExecutionResult,
)


def get_definition() -> NodeDefinition:
    node = NodeDefinition(
        name="uppercase",
        friendly_name="Uppercase",
        description="Converts text to uppercase",
        category="Text/Transform",
    )
    node.add_pin(PinDefinition.input_exec("exec"))
    node.add_pin(PinDefinition.input_pin("text", "String", default=""))
    node.add_pin(PinDefinition.output_exec("exec_out"))
    node.add_pin(PinDefinition.output_pin("result", "String"))
    return node


def run(ctx: Context) -> ExecutionResult:
    text = ctx.get_string("text") or ""
    ctx.set_output("result", text.upper())
    return ctx.success("exec_out")
```

## Quick Start — Node Package (multiple nodes)

```python
from flow_like_wasm_sdk import NodeDefinition, PinDefinition, Context, ExecutionResult, PackageNodes

def define_add() -> NodeDefinition:
    node = NodeDefinition(name="add", friendly_name="Add", description="Adds two numbers", category="Math")
    node.add_pin(PinDefinition.input_exec("exec"))
    node.add_pin(PinDefinition.input_pin("a", "Float", default="0"))
    node.add_pin(PinDefinition.input_pin("b", "Float", default="0"))
    node.add_pin(PinDefinition.output_exec("exec_out"))
    node.add_pin(PinDefinition.output_pin("result", "Float"))
    return node

def run_add(ctx: Context) -> ExecutionResult:
    a = ctx.get_float("a") or 0.0
    b = ctx.get_float("b") or 0.0
    ctx.set_output("result", a + b)
    return ctx.success("exec_out")


pkg = PackageNodes()
pkg.add_node(define_add(), run_add)
# pkg.add_node(define_subtract(), run_subtract)
```

## Testing with MockHostBridge

```python
from flow_like_wasm_sdk import Context, ExecutionInput, MockHostBridge

def test_uppercase():
    host = MockHostBridge()
    ctx = Context(ExecutionInput(inputs={"text": '"hello"'}), host=host)
    result = run(ctx)
    assert result.outputs["result"] == '"HELLO"'
    assert result.exec_output == "exec_out"
```

## Building to WASM

The recommended workflow uses [componentize-py](https://github.com/bytecodealliance/componentize-py):

```bash
# Install componentize-py
pip install componentize-py

# Build WASM component
componentize-py \
  --wit-path path/to/flow-like.wit \
  componentize my_node \
  -o build/my_node.wasm
```

Or use the [wasm-node-python template](../../../templates/wasm-node-python/) which includes the full build setup.

## Publishing

```bash
uv build && uv publish
```

## API Reference

### `NodeDefinition`

```python
NodeDefinition(
    name: str,
    friendly_name: str,
    description: str,
    category: str,
)
```

| Method | Description |
|---|---|
| `add_pin(pin)` | Add an input or output pin |
| `set_scores(scores)` | Set optional quality scores |

### `PinDefinition`

| Static Method | Description |
|---|---|
| `input_exec(name)` | Execution trigger input |
| `output_exec(name)` | Execution trigger output |
| `input_pin(name, type, default?)` | Typed data input |
| `output_pin(name, type)` | Typed data output |

### `Context`

| Method | Description |
|---|---|
| `get_string(pin)` | Read a string input (`str \| None`) |
| `get_bool(pin)` | Read a boolean input (`bool \| None`) |
| `get_int(pin)` | Read an integer input (`int \| None`) |
| `get_float(pin)` | Read a float input (`float \| None`) |
| `get_json(pin)` | Read a JSON string (`str \| None`) |
| `set_output(pin, value)` | Write an output value |
| `success(exec_pin)` | Return success result |
| `error(message)` | Return error result |
| `log_debug/info/warn/error(msg)` | Log via host bridge |
| `node_id / run_id / app_id` | Runtime metadata |

### `ExecutionResult`

```python
ExecutionResult(
    outputs: dict[str, str],   # JSON-encoded values
    exec_output: str | None,   # which exec pin to fire
    error: str | None,
)
```


```bash
pip install flow-like-wasm-sdk
```

## Quick Example

```python
from flow_like_wasm_sdk import (
    NodeDefinition,
    PinDefinition,
    Context,
    ExecutionResult,
    PackageNodes,
)


def get_definition() -> NodeDefinition:
    node = NodeDefinition("upper", "Uppercase", "Converts text to uppercase", "Text/Transform")
    node.add_pin(PinDefinition.input_exec("exec"))
    node.add_pin(PinDefinition.input_pin("text", "String", default=""))
    node.add_pin(PinDefinition.output_exec("exec_out"))
    node.add_pin(PinDefinition.output_pin("result", "String"))
    return node


def run(ctx: Context) -> ExecutionResult:
    text = ctx.get_string("text") or ""
    ctx.set_output("result", text.upper())
    return ctx.success()
```

### Multi-node packages

```python
from flow_like_wasm_sdk import PackageNodes

pkg = PackageNodes()
pkg.add_node(get_definition())
print(pkg.to_json())
```

## Testing

Use `MockHostBridge` for local testing:

```python
from flow_like_wasm_sdk import Context, ExecutionInput, MockHostBridge

host = MockHostBridge()
ctx = Context(ExecutionInput(inputs={"text": "hello"}), host=host)
result = run(ctx)
assert result.outputs["result"] == "HELLO"
```
