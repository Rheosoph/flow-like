# Flow-Like Code Interpreter

WASM Component Model-based Python code interpreter for Flow-Like.

## Nodes

### PythonEval
Executes inline Python code (string input) in a sandboxed WASM environment.

**Inputs:**
- `code` — Python source code string
- `inputs_data` — JSON object exposed as `inputs` dict
- `packages` — JSON array of pure-Python package names to install from PyPI
- `package_allowlist` — JSON array of allowed packages (empty = unrestricted)

**Outputs:**
- `result` — JSON string of the `outputs` dict
- `stdout_out` / `stderr_out` — Captured standard output/error
- `error_msg` — Error traceback if execution failed
- `success_flag` — Boolean success indicator

### PythonProject
Executes a Python project from a FlowPath directory.

**Inputs:**
- `project_root` — FlowPath to directory containing `main.py`
- `entry_point` — Python file to execute (default: `main.py`)
- `inputs_data` — JSON object exposed as `inputs` dict
- `package_allowlist` — Allowed packages for requirements.txt

**Project structure:**
```
project/
  main.py              # Entry point
  requirements.txt     # Optional: pure-Python dependencies
  helpers.py           # Importable by main.py
  utils/
    __init__.py
    ...
```

## Dynamic Package Installation

Pure-Python packages are installed at runtime via the WIT HTTP interface:
1. Queries PyPI JSON API for wheel metadata
2. Downloads `py3-none-any` wheels
3. Extracts to `/tmp/flow_packages/`
4. Adds to `sys.path`

C-extension packages (numpy, pandas, etc.) must be pre-bundled at build time.

## Pre-bundled Packages

The default build includes common pure-Python packages:
- `requests`, `httpx`, `urllib3`, `certifi`
- `pyyaml`, `toml`, `jinja2`
- `beautifulsoup4`, `pydantic`
- `python-dateutil`, `six`, `typing-extensions`

Build without bundles: `mise run build:slim`

## Build

```bash
mise run setup    # Install dependencies
mise run build    # Build WASM component with pre-bundled packages
```

## Architecture

Uses the standard Flow-Like WASM Component Model (same as all Python WASM nodes):
- WIT interface: `flow-like:node@0.1.0`
- Built with `componentize-py`
- Executed via `WasmComponentInstance::call_run()` in the Rust runtime
- Full access to 14 WIT host interfaces (storage, HTTP, LLM, etc.)
