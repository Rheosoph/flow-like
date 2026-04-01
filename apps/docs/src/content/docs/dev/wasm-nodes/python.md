---
title: Python WASM Nodes
description: Create custom WASM nodes using Python with the WASM Component Model
sidebar:
  order: 4
  badge:
    text: Component Model
    variant: success
---

Python can run in WASM through several projects, each with trade-offs.

## Approaches

| Project | Binary Size | Startup | Compatibility |
|---------|-------------|---------|---------------|
| **Pyodide** | ~10 MB | Slow | Full CPython |
| **RustPython** | ~2 MB | Fast | Most Python |
| **MicroPython** | ~300 KB | Fast | Subset |

## Expected Template (Pyodide)

```python title="node.py"
import json

def get_node():
    """Return the node definition."""
    return {
        "name": "wasm_python_uppercase",
        "friendly_name": "Uppercase (Python)",
        "description": "Converts a string to uppercase using Python",
        "category": "Custom/Text",
        "icon": "/flow/icons/text.svg",
        "pins": [
            {
                "name": "exec_in",
                "friendly_name": "▶",
                "description": "Trigger execution",
                "pin_type": "Input",
                "data_type": "Execution",
            },
            {
                "name": "exec_out",
                "friendly_name": "▶",
                "description": "Continue execution",
                "pin_type": "Output",
                "data_type": "Execution",
            },
            {
                "name": "input",
                "friendly_name": "Input",
                "description": "The string to convert",
                "pin_type": "Input",
                "data_type": "String",
                "default_value": "",
            },
            {
                "name": "output",
                "friendly_name": "Output",
                "description": "The uppercase string",
                "pin_type": "Output",
                "data_type": "String",
            },
        ],
        "scores": {
            "privacy": 0,
            "security": 0,
            "performance": 3,  # Python is slower
            "governance": 0,
            "reliability": 0,
            "cost": 1,
        },
    }


def run(context: dict) -> dict:
    """Execute the node logic."""
    input_value = context.get("inputs", {}).get("input", "")

    # Execute logic
    output_value = input_value.upper()

    return {
        "outputs": {
            "output": output_value,
        },
        "error": None,
    }
```

## Why Python in WASM is Challenging

1. **Large runtime** — CPython interpreter is ~10MB
2. **Slow startup** — Interpreter initialization takes time
3. **Limited I/O** — WASM sandbox restricts file/network access
4. **Package compatibility** — Not all PyPI packages work in WASM

## Recommended Alternatives

For most use cases, consider:

| If you need... | Use |
|----------------|-----|
| Fastest execution | [Rust](/dev/wasm-nodes/rust/) |
| Familiar syntax | [TypeScript](/dev/wasm-nodes/typescript/) |
| Easy learning curve | [Go](/dev/wasm-nodes/go/) |
| Python specifically | Wait for Python support |

## Current Workaround

You can call Python scripts via the **Run Script** node in Flow-Like, which executes Python on the host system (not in WASM).

## Stay Updated

Python WASM support is on our roadmap. Watch the repository for updates:

📧 **[info@great-co.de](mailto:info@great-co.de)** for enterprise Python node development

## Related

→ [WASM Nodes Overview](/dev/wasm-nodes/overview/)
→ [Rust Template](/dev/wasm-nodes/rust/)
→ [TypeScript Template](/dev/wasm-nodes/typescript/)
