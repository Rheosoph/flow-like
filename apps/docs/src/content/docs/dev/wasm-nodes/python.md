---
title: Python WASM Nodes
description: Build Flow-Like WASM nodes in Python with componentize-py
sidebar:
  order: 4
  badge:
    text: Component Model
    variant: success
---

Flow-Like's Python SDK builds ordinary Python node logic into a WASM Component
with `componentize-py`. Python support is available now; it does not use
Pyodide, RustPython, or MicroPython.

## Start from the template

Copy `templates/wasm-node-python`, then run:

```bash
mise run setup
mise run test
mise run build
```

The template uses Python 3.12, `uv`, the published
`flow-like-wasm-sdk` package, and `componentize-py`. The component is written to:

```text
build/node.wasm
```

Without mise, the equivalent commands are:

```bash
uv sync --group dev --group build
uv run pytest -v
uv run python build.py
```

## Define a node

The recommended declarative API derives pins from Python annotations:

```python title="src/node.py"
from flow_like_wasm_sdk import (
    Exec,
    ExecInput,
    ExecOutput,
    ExecutionResult,
    Input,
    Output,
    WasmNode,
)


class Uppercase(
    WasmNode,
    name="uppercase_py",
    title="Uppercase",
    category="Custom/Text",
):
    """Converts text to uppercase."""

    trigger: Exec = ExecInput(description="Trigger execution")
    text: str = Input(default="", description="Text to transform")
    done: Exec = ExecOutput(description="Continue execution")
    result: str = Output(description="Uppercase text")

    def run(self, ctx) -> ExecutionResult:
        ctx.result = (ctx.text or "").upper()
        ctx.activate_exec("done")
        return ctx.success()
```

Defining a concrete `WasmNode` subclass registers it automatically. The
template's `app.py` exposes every registered class through the WIT
`get-nodes` and `run` exports.

Use stable `name` and pin identifiers. They become part of persisted board
interfaces.

## Pin types

The declarative API maps common annotations automatically:

| Python annotation | Flow-Like pin |
| --- | --- |
| `str` | String |
| `int` | 64-bit integer |
| `float` | 64-bit float |
| `bool` | Boolean |
| `bytes` | Bytes |
| `Exec` with `ExecInput`/`ExecOutput` | Execution |

Pydantic models and SDK interop types such as `FlowPath` become struct pins.
Lists, dictionaries, and sets are supported as collection value types.

## Permissions

Declare capability labels on the class:

```python
class FetchText(
    WasmNode,
    name="fetch_text_py",
    title="Fetch Text",
    category="Custom/Network",
):
    permissions = ["network:http", "streaming"]
```

Permissions are exported with that node and used by the runtime sandbox. Common
labels include `network:http`, `storage:read`, `storage:write`, `variables`,
`cache`, `streaming`, `models`, `a2ui`, `oauth`, and `functions`.

Package memory and timeout limits belong in `flow-like.toml`. See the
[manifest reference](/dev/wasm-nodes/manifest/).

## Host services

The context exposes gated host services for logging, streaming, variables,
cache, storage, HTTP, OAuth, and models. For example:

```python
ctx.info("Calling upstream API")
response = ctx.http_get("https://api.example.com/data")
ctx.stream_progress(0.5, "Halfway")
```

The node must declare the matching permission before using a gated service.

Python packages that depend on native extensions or unavailable WASI features
may not componentize successfully. Test dependencies in the template rather
than assuming all PyPI packages are portable to WASM.

## Test and inspect

Unit tests execute the node classes natively with the SDK's mock host bridge:

```bash
mise run test
```

To inspect definitions without compiling a component:

```bash
mise run build-definition
```

That command writes a JSON definition under `build/`. It is useful for checking
names, pins, and permissions in review.

## Publish

1. Run `mise run build`.
2. Open Flow-Like Desktop.
3. Go to **Library → Packages → Publish**.
4. Select `build/node.wasm` and `flow-like.toml`.
5. Review the nodes extracted from the binary and submit.

## Related

- [Package Manifest](/dev/wasm-nodes/manifest/)
- [WASM Nodes Overview](/dev/wasm-nodes/overview/)
- [TypeScript WASM Nodes](/dev/wasm-nodes/typescript/)
- [Rust WASM Nodes](/dev/wasm-nodes/rust/)
