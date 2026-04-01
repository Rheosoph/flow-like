#!/usr/bin/env python3
"""Flow-Like Python Interpreter Bootstrap

This script is the execution harness for the Python code interpreter node.
It is embedded in the Rust binary (include_str!) and written to the WASI
/flow directory at runtime, then executed by the Python WASM binary.

Layout expected inside the WASM /flow directory:
  code.py            — user-supplied Python code
  inputs.json        — JSON object passed as `inputs` dict
  config.json        — execution options (packages, allowlists, etc.)
  ws_manifest.json   — list of relative paths available in the workspace
  ws_pending/        — Python writes request files here (UUID → relative path)
  ws_data/           — Rust writes fetched file bytes here (mirrored structure)
  ws_notfound/       — Rust writes empty sentinel files here for 404 responses
  ws_puts/           — Python writes staged files here; Rust uploads after exit

Writes after execution:
  outputs.json — { outputs, stdout, stderr, error, success }
"""

import sys
import json
import os
import io
import time
import traceback

EXEC_DIR = "/flow"

# ─── Workspace directories (created by Rust before execution) ─────────────────

_WS_MANIFEST = EXEC_DIR + "/ws_manifest.json"
_WS_PENDING  = EXEC_DIR + "/ws_pending"
_WS_DATA     = EXEC_DIR + "/ws_data"
_WS_NOTFOUND = EXEC_DIR + "/ws_notfound"
_WS_PUTS     = EXEC_DIR + "/ws_puts"


# ─── Workspace API ────────────────────────────────────────────────────────────

class Workspace:
    """Lazy-loading workspace accessor backed by the object store.

    Files are fetched **on demand** via filesystem IPC with the Rust host:
    Python writes a request file to ``ws_pending/``; a concurrent Rust task
    fetches the object from the store and writes it to ``ws_data/``; Python
    polls until the response arrives.

    Writes are staged locally in ``ws_puts/`` and uploaded by the Rust host
    after execution completes.

    Only paths that are sub-paths of the configured workspace prefix are
    accessible.  Path traversal (``..``) is silently dropped.

    Methods:
        list(prefix="")        → list[str]  relative paths at the prefix
        get(path)              → bytes | None
        put(path, data)        → None
    """

    def __init__(self, manifest: list):
        self._manifest = manifest

    # ── Public API ────────────────────────────────────────────────────────────

    def list(self, prefix: str = "") -> list:
        """Return all workspace paths that start with *prefix*."""
        safe_prefix = self._safe(prefix)
        if not safe_prefix:
            return list(self._manifest)
        return [
            p for p in self._manifest
            if p == safe_prefix or p.startswith(safe_prefix + "/")
        ]

    def get(self, path: str):
        """Fetch *path* from the workspace.  Returns ``bytes`` or ``None``."""
        safe = self._safe(path)
        if not safe:
            return None

        data_path     = _WS_DATA     + "/" + safe
        notfound_base = _WS_NOTFOUND + "/"

        # Fast path — already in local cache.
        if os.path.exists(data_path):
            with open(data_path, "rb") as fh:
                return fh.read()

        # Request the file from the Rust host file server.
        req_id       = _random_id()
        pending_path = _WS_PENDING + "/" + req_id
        notfound_path = notfound_base + req_id

        with open(pending_path, "w") as fh:
            fh.write(safe)

        # Poll: time.sleep() → WASI clock_nanosleep → tokio yield → file server runs.
        # Timeout after 10 s (200 × 50 ms).
        for _ in range(200):
            time.sleep(0.05)

            if os.path.exists(data_path):
                with open(data_path, "rb") as fh:
                    return fh.read()

            if os.path.exists(notfound_path):
                return None  # object does not exist in the store

            if not os.path.exists(pending_path):
                # Pending file removed by host without a data/notfound response
                # (edge case — check one more time before giving up).
                if os.path.exists(data_path):
                    with open(data_path, "rb") as fh:
                        return fh.read()
                return None

        # Timeout — clean up and return None.
        try:
            os.remove(pending_path)
        except OSError:
            # Best-effort cleanup: it's safe to ignore errors if the pending file is already gone or cannot be removed.
            pass
        return None

    def put(self, path: str, data) -> None:
        """Stage *data* for upload to *path* in the workspace.

        *data* must be ``bytes``, ``bytearray``, or ``str`` (UTF-8 encoded).
        """
        safe = self._safe(path)
        if not safe:
            raise ValueError(f"workspace.put(): invalid path {path!r}")

        out_path = _WS_PUTS + "/" + safe
        parent   = os.path.dirname(out_path)
        if parent and parent != _WS_PUTS:
            os.makedirs(parent, exist_ok=True)

        with open(out_path, "wb") as fh:
            if isinstance(data, str):
                fh.write(data.encode("utf-8"))
            elif isinstance(data, (bytes, bytearray)):
                fh.write(data)
            else:
                raise TypeError(
                    f"workspace.put() expects bytes, bytearray, or str — got {type(data).__name__}"
                )

    # ── Internals ─────────────────────────────────────────────────────────────

    @staticmethod
    def _safe(path: str) -> str:
        """Normalise *path*, stripping ``..`` traversal components."""
        parts = []
        for part in path.replace("\\", "/").split("/"):
            if part in ("", "."):
                continue
            if part == "..":
                continue  # silently drop traversal
            parts.append(part)
        return "/".join(parts)


def _random_id() -> str:
    """Return a 16-character hex random string (from WASI ``random_get``)."""
    return os.urandom(8).hex()


# ─── Initialise workspace ─────────────────────────────────────────────────────

_ws_manifest = []
if os.path.exists(_WS_MANIFEST):
    with open(_WS_MANIFEST, "r") as _f:
        _ws_manifest = json.load(_f)

workspace = Workspace(_ws_manifest)


# ─── Load execution parameters ───────────────────────────────────────────────

def _load_json(filename):
    path = os.path.join(EXEC_DIR, filename)
    with open(path, "r") as f:
        return json.load(f)


inputs = _load_json("inputs.json")
config = _load_json("config.json")

with open(os.path.join(EXEC_DIR, "code.py"), "r") as f:
    user_code = f.read()

packages: list = config.get("packages", [])
# None → no restriction (any package allowed)
# []   → deny all packages
# ["pkg1", ...] → only listed packages allowed
package_allowlist = config.get("package_allowlist")  # None or list[str]

# ─── Package installation via micropip ───────────────────────────────────────

if packages:
    try:
        import micropip  # type: ignore[import]
        import asyncio

        async def _install_packages():
            for pkg in packages:
                if package_allowlist is not None and pkg not in package_allowlist:
                    raise PermissionError(
                        f"Package '{pkg}' is not in the allowlist. "
                        f"Allowed: {package_allowlist!r}"
                    )
                await micropip.install(pkg)

        asyncio.run(_install_packages())

    except ImportError:
        sys.stderr.write(
            "[flow-like] Warning: micropip is not available in this Python runtime. "
            "Package installation skipped.\n"
        )
    except PermissionError as exc:
        sys.stderr.write(f"[flow-like] Error: {exc}\n")
        result = {
            "outputs": {},
            "stdout": "",
            "stderr": str(exc),
            "error": str(exc),
            "success": False,
        }
        with open(os.path.join(EXEC_DIR, "outputs.json"), "w") as f:
            json.dump(result, f)
        sys.exit(1)
    except Exception as exc:
        sys.stderr.write(f"[flow-like] Warning: Package installation failed: {exc}\n")


# ─── Execute user code ────────────────────────────────────────────────────────

outputs: dict = {}

_captured_stdout = io.StringIO()
_captured_stderr = io.StringIO()
_orig_stdout = sys.stdout
_orig_stderr = sys.stderr

sys.stdout = _captured_stdout
sys.stderr = _captured_stderr

_error = None

try:
    exec(  # noqa: S102 — intentional sandbox execution
        compile(user_code, "<flow_code>", "exec"),
        {
            "__name__": "__main__",
            "__builtins__": __builtins__,
            "inputs": inputs,
            "outputs": outputs,
            "workspace": workspace,
        },
    )
except SystemExit:
    pass  # allow sys.exit(0) in user code
except Exception:
    _error = traceback.format_exc()

sys.stdout = _orig_stdout
sys.stderr = _orig_stderr

# ─── Write result ─────────────────────────────────────────────────────────────

result = {
    "outputs": outputs,
    "stdout": _captured_stdout.getvalue(),
    "stderr": _captured_stderr.getvalue(),
    "error": _error,
    "success": _error is None,
}

with open(os.path.join(EXEC_DIR, "outputs.json"), "w") as f:
    json.dump(result, f)
