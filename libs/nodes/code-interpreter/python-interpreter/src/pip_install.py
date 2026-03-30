"""
Dynamic pure-Python package installer via PyPI + WASM HTTP.

Downloads pure-Python wheels (py3-none-any) from PyPI and loads them
in-memory using a custom sys.meta_path finder. No filesystem access
required — works in sandboxed WASM environments.

C-extension packages are not supported at runtime — they must be
pre-bundled at build time.

Uses the WIT HTTP host function (ctx.http_get) for network access.
"""

import base64
import codecs
import importlib.abc
import importlib.machinery
import io
import json
import sys
import zipfile

# zipfile uses cp437 for ZIP entry names; register a fallback if the codec
# is missing (common in stripped-down WASM Python runtimes).
try:
    codecs.lookup("cp437")
except LookupError:
    _latin1 = codecs.lookup("latin-1")
    codecs.register(lambda name: _latin1 if name == "cp437" else None)

_INSTALLED: set[str] = set()

_IMPORT_NAMES = {
    "beautifulsoup4": "bs4",
    "python_dateutil": "dateutil",
    "python-dateutil": "dateutil",
    "charset_normalizer": "charset_normalizer",
    "pyyaml": "yaml",
    "python_slugify": "slugify",
    "python-slugify": "slugify",
    "python_dotenv": "dotenv",
    "python-dotenv": "dotenv",
    "markdown_it_py": "markdown_it",
    "markdown-it-py": "markdown_it",
    "jsonpath_ng": "jsonpath_ng",
    "jsonpath-ng": "jsonpath_ng",
    "text_unidecode": "text_unidecode",
    "text-unidecode": "text_unidecode",
    "dataclasses_json": "dataclasses_json",
    "dataclasses-json": "dataclasses_json",
    "marshmallow_enum": "marshmallow_enum",
    "marshmallow-enum": "marshmallow_enum",
    "more_itertools": "more_itertools",
    "more-itertools": "more_itertools",
}


# ── In-memory wheel importer ───────────────────────────────────────────


class _WheelFinder(importlib.abc.MetaPathFinder):
    """Loads modules directly from wheel bytes kept in memory."""

    def __init__(self):
        self._files: dict[str, bytes] = {}

    def add_wheel(self, wheel_bytes: bytes) -> None:
        with zipfile.ZipFile(io.BytesIO(wheel_bytes)) as zf:
            for name in zf.namelist():
                if name.endswith(".py"):
                    self._files[name] = zf.read(name)

    def find_spec(self, fullname, path, target=None):
        parts = fullname.replace(".", "/")
        init_path = parts + "/__init__.py"
        mod_path = parts + ".py"

        if init_path in self._files:
            return importlib.machinery.ModuleSpec(
                fullname, self, is_package=True, origin=init_path
            )
        if mod_path in self._files:
            return importlib.machinery.ModuleSpec(
                fullname, self, is_package=False, origin=mod_path
            )
        return None

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        spec = module.__spec__
        file_path = spec.origin
        source = self._files[file_path]

        if spec.submodule_search_locations is not None:
            parts = spec.name.replace(".", "/")
            module.__path__ = [parts]

        code = compile(source, file_path, "exec")
        exec(code, module.__dict__)


_wheel_finder = _WheelFinder()
sys.meta_path.insert(0, _wheel_finder)


# ── Public API ──────────────────────────────────────────────────────────


def _is_importable(name: str) -> bool:
    """Check if a package is already importable (e.g. pre-bundled)."""
    import_name = _IMPORT_NAMES.get(name, name.replace("-", "_"))
    try:
        __import__(import_name)
        return True
    except ImportError:
        return False


def install_packages(ctx, packages: list[str]) -> None:
    """Install a list of pure-Python packages from PyPI."""
    for pkg in packages:
        name = pkg.strip().lower()
        if name in _INSTALLED:
            ctx.info(f"pip_install: {name} already installed (cached)")
            continue
        if _is_importable(name):
            ctx.info(f"pip_install: {name} already importable (pre-bundled)")
            _INSTALLED.add(name)
            continue
        ctx.info(f"pip_install: {name} not pre-bundled, attempting dynamic install")
        _install_one(ctx, name)
        _INSTALLED.add(name)


def _install_one(ctx, name: str) -> None:
    """Download a single pure-Python wheel from PyPI and load it in-memory."""
    ctx.info(f"pip_install: resolving {name} from PyPI")

    url = f"https://pypi.org/pypi/{name}/json"
    resp = ctx.http_get(url)
    if resp is None:
        raise ImportError(
            f"Cannot reach PyPI for package '{name}'. "
            "HTTP request returned None — the host may have denied network access "
            "or the request failed. Ensure the node has 'network:http' permission."
        )

    body = resp.get("body", "")
    try:
        meta = json.loads(body) if isinstance(body, str) else body
    except (json.JSONDecodeError, TypeError):
        raise ImportError(f"Invalid PyPI response for '{name}'")

    if not isinstance(meta, dict) or "urls" not in meta:
        status = resp.get("status", "unknown")
        raise ImportError(f"Package '{name}' not found on PyPI (status: {status})")

    wheel_url = _find_wheel(meta["urls"])
    if wheel_url is None:
        releases = meta.get("releases", {})
        version = meta.get("info", {}).get("version", "")
        if version and version in releases:
            wheel_url = _find_wheel(releases[version])

    if wheel_url is None:
        raise ImportError(
            f"No pure-Python wheel found for '{name}'. "
            "C-extension packages must be pre-bundled at build time."
        )

    ctx.info(f"pip_install: downloading {name} wheel")

    wheel_resp = ctx.http_get(wheel_url)
    if wheel_resp is None:
        raise ImportError(f"Failed to download wheel for '{name}' — HTTP returned None")

    status = wheel_resp.get("status", 0)
    if status >= 400:
        raise ImportError(
            f"Failed to download wheel for '{name}': HTTP {status}"
        )

    # Use body_bytes (decoded from body_base64 by SDK) for binary-safe data
    wheel_body = wheel_resp.get("body_bytes")
    if not wheel_body:
        # Fallback: try body_base64 directly
        wheel_b64 = wheel_resp.get("body_base64")
        if wheel_b64:
            wheel_body = base64.b64decode(wheel_b64)
        else:
            raise ImportError(
                f"No binary body in wheel response for '{name}' (HTTP {status}). "
                f"Response keys: {list(wheel_resp.keys())}"
            )

    if len(wheel_body) == 0:
        raise ImportError(f"Downloaded empty body for wheel '{name}' (HTTP {status})")

    try:
        _wheel_finder.add_wheel(wheel_body)
    except zipfile.BadZipFile:
        raise ImportError(
            f"Downloaded wheel for '{name}' is not a valid zip file "
            f"(size={len(wheel_body)}, HTTP {status})"
        )

    ctx.info(f"pip_install: installed {name} (in-memory)")


def _find_wheel(files: list[dict]) -> str | None:
    """Find a pure-Python wheel URL from PyPI file list."""
    for f in files:
        filename = f.get("filename", "")
        if filename.endswith("-py3-none-any.whl"):
            return f.get("url")
    # Also accept py2.py3-none-any
    for f in files:
        filename = f.get("filename", "")
        if "none-any.whl" in filename and ("py3" in filename or "py2.py3" in filename):
            return f.get("url")
    return None
