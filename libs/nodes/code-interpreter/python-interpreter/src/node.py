"""
Flow-Like Code Interpreter — WASM Component Nodes

Three nodes for sandboxed Python execution:
  1. PythonEval    — executes inline Python code (string input)
  2. PythonProject — executes a Python project from a FlowPath directory
  3. CodeAgent     — LLM agent that solves tasks by writing & running Python code
"""

import io
import json
import sys
import traceback
from typing import Any

from flow_like_wasm_sdk import (
    Bit,
    ExecInput,
    ExecOutput,
    ExecutionResult,
    FlowPath,
    Input,
    NodeScores,
    Output,
    WasmNode,
)
from flow_like_wasm_sdk.interop import ChatMessage, ToolCallData

from pip_install import install_packages


# ── Helpers ──────────────────────────────────────────────────────────────

def _resolve_workspace_path(workspace: FlowPath, rel_path: str) -> FlowPath:
    """Resolve a relative path against the workspace, supporting / separators."""
    segments = [s for s in rel_path.replace("\\", "/").split("/") if s and s != "."]
    result = workspace
    for seg in segments:
        result = result.child(seg)
    return result


def _exec_python(
    ctx: Any,
    code: str,
    inputs: dict,
    packages: list[str],
    package_allowlist: list[str] | None,
    extra_globals: dict | None = None,
) -> dict:
    """Execute *code* in a sandboxed namespace and return result dict."""

    # Install requested packages via PyPI HTTP
    install_errors = []
    if packages:
        for pkg in packages:
            if package_allowlist is not None and pkg not in package_allowlist:
                install_errors.append(f"Package '{pkg}' is not in the allowlist")
                continue
            try:
                install_packages(ctx, [pkg])
            except Exception as exc:
                install_errors.append(f"Failed to install '{pkg}': {exc}")

    if install_errors:
        error_text = "Package installation failed:\n" + "\n".join(install_errors)
        return {
            "outputs": {},
            "stdout": "",
            "stderr": error_text,
            "error": error_text,
            "success": False,
        }

    # Capture stdout/stderr
    captured_stdout = io.StringIO()
    captured_stderr = io.StringIO()
    orig_stdout = sys.stdout
    orig_stderr = sys.stderr
    sys.stdout = captured_stdout
    sys.stderr = captured_stderr

    outputs: dict = {}
    error = None

    ns = {
        "__name__": "__main__",
        "__builtins__": __builtins__,
        "inputs": inputs,
        "outputs": outputs,
        "ctx": ctx,
    }
    if extra_globals:
        ns.update(extra_globals)

    try:
        exec(compile(code, "<flow_code>", "exec"), ns)
    except SystemExit:
        pass
    except Exception:
        error = traceback.format_exc()

    sys.stdout = orig_stdout
    sys.stderr = orig_stderr

    return {
        "outputs": outputs,
        "stdout": captured_stdout.getvalue(),
        "stderr": captured_stderr.getvalue(),
        "error": error,
        "success": error is None,
    }


# ── Node 1: PythonEval ──────────────────────────────────────────────────

class PythonEval(
    WasmNode,
    name="python_eval",
    title="Python Eval",
    category="Code/Python",
):
    """Execute inline Python code in a WASM sandbox.

    Available globals inside the script:
      - inputs   — dict with values from the Inputs pin
      - outputs  — write your results here
      - ctx      — SDK Context (storage, HTTP, LLM, logging, …)
    """
    permissions = ["network:http", "storage:read", "storage:write"]
    long_running = True
    scores = NodeScores(security=1, privacy=1, governance=1, performance=7, reliability=1, cost=7)

    exec_in: None = ExecInput(description="Trigger execution")

    code: str = Input(
        default=(
            "# Flow-Like Python Interpreter\n"
            "# inputs  — dict from the Inputs pin\n"
            "# outputs — write your results here\n"
            "# ctx     — SDK Context (storage, HTTP, LLM, …)\n"
            "\n"
            "outputs['result'] = inputs.get('value', 'Hello World')\n"
        ),
        description=(
            "Python source to execute.\n"
            "Available globals:\n"
            "  inputs   — dict with values from the Inputs pin\n"
            "  outputs  — write your results here\n"
            "  ctx      — SDK Context (storage, HTTP, LLM, logging, …)"
        ),
    )
    inputs_data: str = Input(
        default="{}",
        description="JSON object exposed as the `inputs` dict inside Python.",
    )
    packages: str = Input(
        default="[]",
        description=(
            "JSON array of pure-Python package names to install from PyPI "
            "before execution (e.g. [\"requests\", \"pyyaml\"])."
        ),
    )
    package_allowlist: str = Input(
        default="",
        description=(
            "JSON array of allowed package names, or empty for unrestricted.\n"
            "Examples: '' (any), '[]' (none), '[\"requests\"]' (listed only)."
        ),
    )

    # Outputs
    exec_out: None = ExecOutput(description="Activated on success")
    exec_error: None = ExecOutput(description="Activated on error")
    result: str = Output(description="JSON string of the Python `outputs` dict")
    stdout_out: str = Output(description="Captured standard output")
    stderr_out: str = Output(description="Captured standard error")
    error_msg: str = Output(description="Error traceback if execution failed")
    success_flag: bool = Output(description="True if code completed without errors")

    def run(self, ctx) -> ExecutionResult:
        ctx.info("PythonEval: starting execution")

        code: str = ctx.code
        ctx.info(f"PythonEval: code length={len(code) if code else 0}, "
                 f"first 80 chars={repr(code[:80]) if code else '(None)'}")

        if not code.strip():
            ctx.warn("PythonEval: code is empty or whitespace-only, returning early")
            ctx.error_msg = ""
            ctx.success_flag = True
            ctx.result = "{}"
            ctx.stdout_out = ""
            ctx.stderr_out = ""
            return ctx.success()

        # Parse inputs JSON
        try:
            inputs_dict = json.loads(ctx.inputs_data) if ctx.inputs_data else {}
        except (json.JSONDecodeError, TypeError):
            inputs_dict = {}

        # Parse packages
        try:
            pkgs = json.loads(ctx.packages) if ctx.packages else []
        except (json.JSONDecodeError, TypeError):
            pkgs = []

        ctx.info(f"PythonEval: inputs_keys={list(inputs_dict.keys())}, "
                 f"packages={pkgs}")

        # Parse allowlist
        allowlist = None
        if ctx.package_allowlist:
            try:
                allowlist = json.loads(ctx.package_allowlist)
            except (json.JSONDecodeError, TypeError):
                allowlist = None

        result = _exec_python(ctx, code, inputs_dict, pkgs, allowlist)

        ctx.info(f"PythonEval: success={result['success']}, "
                 f"output_keys={list(result['outputs'].keys())}, "
                 f"stdout_len={len(result['stdout'])}, "
                 f"stderr_len={len(result['stderr'])}")

        ctx.result = json.dumps(result["outputs"])
        ctx.stdout_out = result["stdout"]
        ctx.stderr_out = result["stderr"]
        ctx.error_msg = result["error"] or ""
        ctx.success_flag = result["success"]

        if result["success"]:
            return ctx.success()
        else:
            ctx.error(f"PythonEval failed: {result['error']}")
            ctx.activate_exec("exec_error")
            return ctx.finish()


# ── Node 2: PythonProject ───────────────────────────────────────────────

class PythonProject(
    WasmNode,
    name="python_project",
    title="Python Project",
    category="Code/Python",
):
    """Execute a Python project from a FlowPath directory.

    Reads main.py (or a custom entry point) from the project root,
    along with any requirements.txt for automatic dependency installation.
    All .py files in the project are available for import.
    """
    permissions = ["network:http", "storage:read", "storage:write"]
    long_running = True
    scores = NodeScores(security=1, privacy=1, governance=1, performance=7, reliability=1, cost=7)

    exec_in: None = ExecInput(description="Trigger execution")

    project_root: FlowPath = Input(
        description=(
            "FlowPath to the Python project root directory.\n"
            "Expected structure:\n"
            "  main.py          — entry point (required)\n"
            "  requirements.txt — optional deps (pure-Python only)\n"
            "  *.py             — importable modules"
        ),
    )
    entry_point: str = Input(
        default="main.py",
        description="Python file to execute (relative to project root).",
    )
    inputs_data: str = Input(
        default="{}",
        description="JSON object exposed as the `inputs` dict inside the script.",
    )
    package_allowlist: str = Input(
        default="",
        description=(
            "JSON array of allowed package names, or empty for unrestricted.\n"
            "requirements.txt packages are checked against this list."
        ),
    )

    # Outputs
    exec_out: None = ExecOutput(description="Activated on success")
    exec_error: None = ExecOutput(description="Activated on error")
    result: str = Output(description="JSON string of the Python `outputs` dict")
    stdout_out: str = Output(description="Captured standard output")
    stderr_out: str = Output(description="Captured standard error")
    error_msg: str = Output(description="Error traceback if execution failed")
    success_flag: bool = Output(description="True if code completed without errors")

    def run(self, ctx) -> ExecutionResult:
        ctx.info("PythonProject: starting execution")

        project_root: FlowPath = ctx.project_root

        # List project files
        entries = project_root.list(ctx)
        if entries is None:
            ctx.error_msg = "Cannot list project root directory"
            ctx.success_flag = False
            ctx.activate_exec("exec_error")
            return ctx.finish()

        # Read entry point
        entry = ctx.entry_point or "main.py"
        entry_path = project_root.child(entry)
        entry_code = entry_path.get_string(ctx)
        if entry_code is None:
            ctx.error_msg = f"Entry point '{entry}' not found in project root"
            ctx.success_flag = False
            ctx.activate_exec("exec_error")
            return ctx.finish()

        # Parse inputs JSON
        try:
            inputs_dict = json.loads(ctx.inputs_data) if ctx.inputs_data else {}
        except (json.JSONDecodeError, TypeError):
            inputs_dict = {}

        # Parse allowlist
        allowlist = None
        if ctx.package_allowlist:
            try:
                allowlist = json.loads(ctx.package_allowlist)
            except (json.JSONDecodeError, TypeError):
                allowlist = None

        # Read and install requirements.txt if present
        req_path = project_root.child("requirements.txt")
        req_text = req_path.get_string(ctx)
        pkgs = []
        if req_text:
            for line in req_text.splitlines():
                line = line.strip()
                if line and not line.startswith("#"):
                    # Strip version specifiers for the package name
                    pkg_name = line.split("==")[0].split(">=")[0].split("<=")[0].split("!=")[0].split("~=")[0].strip()
                    if pkg_name:
                        pkgs.append(pkg_name)

        # Load all .py files from project as importable modules
        _setup_project_imports(ctx, project_root, entries, entry)

        result = _exec_python(ctx, entry_code, inputs_dict, pkgs, allowlist)

        ctx.result = json.dumps(result["outputs"])
        ctx.stdout_out = result["stdout"]
        ctx.stderr_out = result["stderr"]
        ctx.error_msg = result["error"] or ""
        ctx.success_flag = result["success"]

        if result["success"]:
            return ctx.success()
        else:
            ctx.error(f"PythonProject failed: {result['error']}")
            ctx.activate_exec("exec_error")
            return ctx.finish()


def _setup_project_imports(ctx: Any, root: FlowPath, entries: list, skip_entry: str) -> None:
    """Make .py files from the project importable by writing them to a temp dir on sys.path."""
    import os
    import tempfile

    project_dir = tempfile.mkdtemp(prefix="flow_project_")
    sys.path.insert(0, project_dir)

    for fp in entries:
        name = fp.file_name() if isinstance(fp, FlowPath) else str(fp)
        if not name or not name.endswith(".py"):
            continue

        content = fp.get_string(ctx) if isinstance(fp, FlowPath) else root.child(name).get_string(ctx)
        if content is None:
            continue

        full_path = os.path.join(project_dir, name)
        parent = os.path.dirname(full_path)
        if parent and parent != project_dir:
            os.makedirs(parent, exist_ok=True)

        with open(full_path, "w") as f:
            f.write(content)


# ── Node 3: CodeAgent ───────────────────────────────────────────────────

_CODE_AGENT_SYSTEM_PROMPT = """\
You are a Python code execution agent. You solve tasks by writing and running Python code.

## Environment
- Python 3.12 running in a sandboxed WASM environment. Each python_exec call is **stateless** — variables, imports, and definitions do not persist between calls.
- **No native filesystem.** `open()`, `os.path`, `pathlib`, etc. do NOT work. Use the FlowPath API (see below) or the file tools.
- **No graphical output.** matplotlib, PIL.show(), and similar will not render. Return data as text, numbers, or JSON.
- **No subprocess/threading.** subprocess, multiprocessing, threading, signal, and socket modules are unavailable.
- **Network access is available** via pre-installed libraries (requests, httpx). Use them for HTTP calls, API requests, web scraping, etc.
- **File system access** is available through the `workspace` FlowPath object and the file_write / file_read / file_list tools.

## Tools

### python_exec
Executes Python code and returns JSON with: stdout, stderr, error, outputs, success.
- `print()` writes to stdout (returned to you for inspection).
- `outputs['result']` stores the final answer — this is what gets returned to the user.
- Each call is independent. If you need results from a prior call, you must redefine or recompute them.
- Two special globals are injected: `ctx` (SDK context) and `workspace` (a FlowPath to the workspace directory).

#### Writing files from python_exec
Inside python_exec you have direct access to the `workspace` FlowPath and `ctx`:
```python
# Write text
workspace.child("report.txt").put_string(ctx, "Hello World")

# Write binary (e.g. PDF)
pdf_bytes = generate_pdf()  # bytes
workspace.child("report.pdf").put(ctx, pdf_bytes)

# Write JSON
workspace.child("data.json").put_json(ctx, {"key": "value"})

# Read text
content = workspace.child("file.txt").get_string(ctx)  # str | None

# Read binary
raw = workspace.child("image.png").get(ctx)  # bytes | None

# List directory
files = workspace.child("subdir").list(ctx)  # list[FlowPath] | None
for f in (files or []):
    print(f.file_name())  # filename string

# Subdirectories are created automatically on write
workspace.child("deep").child("nested").child("file.txt").put_string(ctx, "ok")
```

### file_write
Write a file to the workspace. For text content, pass `text`. For binary content, pass `base64` encoded data.

### file_read
Read a file from the workspace. Returns the file content as text (UTF-8) or base64-encoded binary.

### file_list
List files and directories under a workspace path. Returns a list of names.

### pip_install
Installs additional **pure-Python** packages at runtime from PyPI.
- Only `py3-none-any` wheels work — packages with C extensions (numpy, pandas, scipy, Pillow, etc.) will **fail**.
- Already installed packages are skipped automatically.

## Pre-installed packages (do NOT reinstall)
requests, urllib3, httpx, beautifulsoup4, jinja2, pyyaml, toml, json5, \
pydantic, attrs, marshmallow, python-dateutil, pytz, tabulate, rich, \
click, tqdm, tenacity, packaging, jsonpath-ng, langchain_core, langgraph

## Strategy
1. **Think first.** Before writing code, briefly reason about the approach. Identify what data you have, what computation is needed, and what output format to use.
2. **Start simple.** Write the smallest piece of code that makes progress. Avoid writing long scripts on the first try.
3. **Iterate on errors.** If code fails, read the full traceback carefully. Identify the root cause. Fix only the broken part — do not rewrite everything from scratch.
4. **Use print() for debugging.** Add print() statements to inspect intermediate values when something goes wrong.
5. **Build incrementally.** For complex tasks, solve one sub-problem at a time. Once each step works, combine them.
6. **Prefer stdlib.** Use Python's built-in modules (math, statistics, collections, itertools, re, json, csv, decimal) before reaching for external packages.
7. **No C-extension packages.** numpy, pandas, scipy, matplotlib, Pillow, and similar C-based packages are **not available**. Use pure-Python alternatives or write the computation directly.
8. **Return structured results.** Set `outputs['result']` to the final answer. For complex data, use JSON-serializable structures (dicts, lists, strings, numbers).
9. **Write files for rich output.** When producing PDFs, CSVs, HTML, images, or other files, write them to the workspace using `workspace.child("name").put(ctx, data)` or the file_write tool.

## When to stop
- When you have the final answer, respond with a plain text message (no tool call) containing the answer.
- Do NOT call python_exec if you can answer directly from existing knowledge.
- If you need a package that is not pre-installed and is pure-Python, call pip_install first, then python_exec.
"""


def _run_agent_loop(ctx: Any, bit: Bit, task: str, max_iterations: int, workspace: FlowPath | None = None) -> dict:
    """Run the code-execution agent loop using LangChain tool calling."""

    # Build tool definition for the LLM
    python_exec_tool = {
        "name": "python_exec",
        "description": (
            "Execute Python code in a sandboxed environment. "
            "The code has access to: print() for output, outputs dict for results. "
            "Returns JSON with: stdout, stderr, error, outputs, success."
        ),
        "parameters": {
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "The Python code to execute.",
                },
            },
            "required": ["code"],
        },
    }

    pip_install_tool = {
        "name": "pip_install",
        "description": (
            "Install pure-Python packages from PyPI at runtime. "
            "Only py3-none-any wheels are supported — C-extension packages will fail. "
            "Returns JSON with: installed (list of installed packages), errors (list of errors), success."
        ),
        "parameters": {
            "type": "object",
            "properties": {
                "packages": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of package names to install, e.g. ['sympy', 'scipy'].",
                },
            },
            "required": ["packages"],
        },
    }

    tools_for_host = [python_exec_tool, pip_install_tool]

    if workspace is not None:
        file_write_tool = {
            "name": "file_write",
            "description": (
                "Write a file to the workspace directory. "
                "For text content use the 'text' field. For binary content use the 'base64' field. "
                "Subdirectories are created automatically. "
                "Returns JSON with: path, success."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path within the workspace, e.g. 'report.pdf' or 'output/data.csv'.",
                    },
                    "text": {
                        "type": "string",
                        "description": "Text content to write (UTF-8). Mutually exclusive with base64.",
                    },
                    "base64": {
                        "type": "string",
                        "description": "Base64-encoded binary content. Mutually exclusive with text.",
                    },
                },
                "required": ["path"],
            },
        }
        file_read_tool = {
            "name": "file_read",
            "description": (
                "Read a file from the workspace directory. "
                "Returns JSON with: content (text), base64 (if binary), exists, success."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path within the workspace.",
                    },
                },
                "required": ["path"],
            },
        }
        file_list_tool = {
            "name": "file_list",
            "description": (
                "List files and directories under a workspace path. "
                "Returns JSON with: files (list of filenames), success."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative directory path within the workspace. Use '' or '.' for root.",
                    },
                },
                "required": ["path"],
            },
        }
        tools_for_host.extend([file_write_tool, file_read_tool, file_list_tool])
    messages: list[ChatMessage] = [
        ChatMessage.system(_CODE_AGENT_SYSTEM_PROMPT),
        ChatMessage.user(task),
    ]

    all_scripts: list[str] = []
    final_answer = ""
    iterations_used = 0
    content = ""

    for iteration in range(max_iterations):
        iterations_used = iteration + 1
        ctx.info(f"CodeAgent: iteration {iterations_used}/{max_iterations}")

        response_text = ctx.llm_prompt(bit, messages, stream=False, tools=tools_for_host)
        if response_text is None:
            return {
                "answer": "",
                "scripts": all_scripts,
                "error": "LLM returned no response",
                "iterations": iterations_used,
                "success": False,
            }

        try:
            response = json.loads(response_text)
        except (json.JSONDecodeError, TypeError):
            # Plain text response — treat as final answer
            final_answer = response_text
            break

        content = response.get("content", "") or ""
        tool_calls = response.get("tool_calls")

        if not tool_calls:
            # No tool call — this is the final answer
            final_answer = content
            break

        # Add assistant message with tool calls to history
        messages.append(ChatMessage.assistant_with_tool_calls(
            content,
            [
                ToolCallData(
                    id=tc.get("id", f"call_{iteration}_{i}"),
                    name=tc["name"],
                    arguments=tc.get("arguments", {}),
                )
                for i, tc in enumerate(tool_calls)
            ],
        ))

        # Execute each tool call
        for i, tc in enumerate(tool_calls):
            tc_id = tc.get("id", f"call_{iteration}_{i}")
            tc_name = tc.get("name", "")
            tc_args = tc.get("arguments", {})

            if isinstance(tc_args, str):
                try:
                    tc_args = json.loads(tc_args)
                except (json.JSONDecodeError, TypeError):
                    tc_args = {}

            if tc_name == "python_exec":
                code = tc_args.get("code", "")
                all_scripts.append(code)
                ctx.info(f"CodeAgent: executing code ({len(code)} chars)")

                extra_globals = {}
                if workspace is not None:
                    extra_globals["workspace"] = workspace
                result = _exec_python(ctx, code, {}, [], None, extra_globals=extra_globals)

                tool_result = json.dumps({
                    "stdout": result["stdout"],
                    "stderr": result["stderr"],
                    "error": result["error"],
                    "outputs": result["outputs"],
                    "success": result["success"],
                })
                messages.append(ChatMessage.tool_result(tc_id, tool_result))
            elif tc_name == "pip_install":
                pkgs = tc_args.get("packages", [])
                if isinstance(pkgs, str):
                    try:
                        pkgs = json.loads(pkgs)
                    except (json.JSONDecodeError, TypeError):
                        pkgs = [pkgs]
                ctx.info(f"CodeAgent: installing packages {pkgs}")

                installed = []
                errors = []
                for pkg in pkgs:
                    try:
                        install_packages(ctx, [pkg])
                        installed.append(pkg)
                    except Exception as exc:
                        errors.append(f"{pkg}: {exc}")

                tool_result = json.dumps({
                    "installed": installed,
                    "errors": errors,
                    "success": len(errors) == 0,
                })
                messages.append(ChatMessage.tool_result(tc_id, tool_result))
            elif tc_name == "file_write" and workspace is not None:
                rel_path = tc_args.get("path", "")
                text_content = tc_args.get("text")
                b64_content = tc_args.get("base64")
                ctx.info(f"CodeAgent: file_write '{rel_path}'")

                target = _resolve_workspace_path(workspace, rel_path)
                ok = False
                if text_content is not None:
                    ok = target.put_string(ctx, text_content)
                elif b64_content is not None:
                    import base64 as b64mod
                    ok = target.put(ctx, b64mod.b64decode(b64_content))
                else:
                    ok = target.put_string(ctx, "")

                tool_result = json.dumps({"path": rel_path, "success": ok})
                messages.append(ChatMessage.tool_result(tc_id, tool_result))
            elif tc_name == "file_read" and workspace is not None:
                rel_path = tc_args.get("path", "")
                ctx.info(f"CodeAgent: file_read '{rel_path}'")

                target = _resolve_workspace_path(workspace, rel_path)
                text = target.get_string(ctx)
                if text is not None:
                    tool_result = json.dumps({"content": text, "exists": True, "success": True})
                else:
                    raw = target.get(ctx)
                    if raw is not None:
                        import base64 as b64mod
                        tool_result = json.dumps({
                            "base64": b64mod.b64encode(raw).decode("ascii"),
                            "exists": True,
                            "success": True,
                        })
                    else:
                        tool_result = json.dumps({"exists": False, "success": False})
                messages.append(ChatMessage.tool_result(tc_id, tool_result))
            elif tc_name == "file_list" and workspace is not None:
                rel_path = tc_args.get("path", "") or ""
                ctx.info(f"CodeAgent: file_list '{rel_path}'")

                target = workspace if rel_path in ("", ".") else _resolve_workspace_path(workspace, rel_path)
                entries = target.list(ctx)
                names = [e.file_name() or "" for e in (entries or [])]
                tool_result = json.dumps({"files": names, "success": entries is not None})
                messages.append(ChatMessage.tool_result(tc_id, tool_result))
            else:
                messages.append(ChatMessage.tool_result(
                    tc_id, json.dumps({"error": f"Unknown tool: {tc_name}"}),
                ))
    else:
        final_answer = content if content else "Max iterations reached without a final answer."

    return {
        "answer": final_answer,
        "scripts": all_scripts,
        "error": None,
        "iterations": iterations_used,
        "success": True,
    }


class CodeAgent(
    WasmNode,
    name="code_agent",
    title="Code Agent",
    category="Code/Python",
):
    """LLM-powered agent that solves tasks by writing and executing Python code.

    Takes a model (Bit) and a task description, then uses the model to
    iteratively write and run Python code until it produces an answer.
    Compatible with any LLM that supports tool/function calling.
    """
    permissions = ["network:http", "storage:read", "storage:write", "models"]
    long_running = True
    scores = NodeScores(security=1, privacy=1, governance=1, performance=5, reliability=3, cost=3)

    exec_in: None = ExecInput(description="Trigger execution")

    model: Bit = Input(description="LLM model to use for code generation (must support tool calling).")
    task: str = Input(description="The task or question for the agent to solve using Python code.")
    workspace: FlowPath = Input(
        default=None,
        description=(
            "Optional workspace directory for file I/O. "
            "When provided, the agent can read/write files (PDFs, CSVs, etc.) "
            "in this directory and its subdirectories."
        ),
    )
    max_iterations: int = Input(
        default=10,
        description="Maximum number of code generation/execution cycles before stopping.",
    )

    # Outputs
    exec_out: None = ExecOutput(description="Activated on success")
    exec_error: None = ExecOutput(description="Activated on error")
    answer: str = Output(description="The agent's final answer to the task.")
    scripts: str = Output(description="JSON array of all Python scripts the agent executed.")
    iterations_used: int = Output(description="Number of agent iterations used.")
    error_msg: str = Output(description="Error message if the agent failed.")
    success_flag: bool = Output(description="True if the agent completed successfully.")

    def run(self, ctx) -> ExecutionResult:
        ctx.info("CodeAgent: starting")

        model: Bit = ctx.model
        task: str = ctx.task
        max_iter: int = ctx.max_iterations or 10

        if not task or not task.strip():
            ctx.answer = ""
            ctx.scripts = "[]"
            ctx.iterations_used = 0
            ctx.error_msg = "No task provided"
            ctx.success_flag = False
            ctx.activate_exec("exec_error")
            return ctx.finish()

        workspace: FlowPath | None = ctx.workspace
        ctx.info(f"CodeAgent: task length={len(task)}, max_iterations={max_iter}, workspace={'yes' if workspace else 'no'}")

        result = _run_agent_loop(ctx, model, task, max_iter, workspace=workspace)

        ctx.answer = result["answer"]
        ctx.scripts = json.dumps(result["scripts"])
        ctx.iterations_used = result.get("iterations", 0)
        ctx.error_msg = result["error"] or ""
        ctx.success_flag = result["success"]

        if result["success"]:
            ctx.info(f"CodeAgent: completed in {result.get('iterations', 0)} iterations")
            return ctx.success()
        else:
            ctx.error(f"CodeAgent failed: {result['error']}")
            ctx.activate_exec("exec_error")
            return ctx.finish()
