#!/usr/bin/env python3
"""
Build script for the Flow-Like Code Interpreter WASM component.

Generates a WASM component from the Python interpreter nodes using componentize-py.
Pre-bundles common pure-Python packages into the component.

Usage:
    uv run python build.py                # Full build with pre-bundled packages
    uv run python build.py --no-bundle    # Build without pre-bundled packages
    uv run python build.py --definition-only  # Extract JSON definitions only
"""

import argparse
import json
import subprocess
import sys
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

ROOT = Path(__file__).resolve().parent
BUILD_DIR = ROOT / "build"
SRC_DIR = ROOT / "src"
VENDOR_DIR = ROOT / "vendor"

# WIT file: prefer SDK-shipped copy, fall back to local
_LOCAL_WIT = ROOT / "wit" / "flow-like-node.wit"
try:
    from flow_like_wasm_sdk import WIT_PATH as _SDK_WIT
    WIT_PATH = _SDK_WIT if _SDK_WIT.exists() else _LOCAL_WIT
except ImportError:
    WIT_PATH = _LOCAL_WIT

# Pre-bundled packages — pure-Python wheels downloaded at build time
BUNDLED_PACKAGES = [
    # HTTP / networking
    "requests",
    "urllib3",
    "charset_normalizer",
    "certifi",
    "idna",
    "httpx",
    "httpcore",
    "anyio",
    "sniffio",
    "h11",
    # HTML / XML / templating
    "beautifulsoup4",
    "soupsieve",
    "jinja2",
    "markupsafe",
    "defusedxml",
    "xmltodict",
    # Data serialization / config
    "pyyaml",
    "toml",
    "tomli",
    "json5",
    "jsonlines",
    # Validation / typing
    "pydantic",
    "annotated_types",
    "typing_extensions",
    "attrs",
    "marshmallow",
    "dataclasses_json",
    "marshmallow_enum",
    "validators",
    # Date / time / locale
    "python_dateutil",
    "six",
    "pytz",
    "isodate",
    # Text / formatting
    "tabulate",
    "texttable",
    "humanize",
    "python_slugify",
    "text_unidecode",
    "colorama",
    "pygments",
    "rich",
    "wcwidth",
    "markdown_it_py",
    "mdurl",
    # CLI / utility
    "click",
    "tqdm",
    "tenacity",
    "python_dotenv",
    "semver",
    "packaging",
    "pyparsing",
    "chardet",
    "more_itertools",
    # JSON processing
    "jsonpath_ng",
    "ply",
    # LangChain (agent / LLM orchestration)
    "langchain_core",
    "langsmith",
    "orjson",
    "langgraph",
    "langgraph_sdk",
    "langgraph_checkpoint",
]

# CPython stdlib modules to copy into vendor/ for the WASM component.
# componentize-py only bundles a minimal subset; these are commonly needed
# by third-party packages and user code.
# NOTE: Modules requiring OS syscalls (signal, subprocess, socket) are excluded.
STDLIB_MODULES = [
    # Core data/time types
    "datetime",
    "_pydatetime",  # pure-Python fallback for datetime (C ext unavailable)
    "_strptime",    # datetime.strptime support
    "calendar",
    "decimal",
    "_pydecimal",   # pure-Python fallback for decimal
    "fractions",
    "numbers",
    "statistics",
    # Text processing
    "string",
    "difflib",
    "pprint",
    "shlex",
    # Data formats
    "csv",
    "configparser",
    "html",
    "xml",
    "email",
    "mimetypes",
    # Networking (import support for packages; actual I/O via WIT host)
    "http",
    "urllib",
    # Hashing/security
    "hashlib",
    "hmac",
    "secrets",
    # Containers/algorithms
    "heapq",
    # CLI
    "argparse",
    # Logging
    "logging",
    # Compression
    "gzip",
    "_compression",  # base classes for gzip
    # Serialization
    "pickle",
    "_compat_pickle",  # pickle compatibility layer
    "copyreg",         # pickle dependency
    # Misc
    "locale",
    "platform",
    "uuid",
    "tempfile",
    "unittest",
]


def load_module(path: Path):
    sys.path.insert(0, str(SRC_DIR))
    spec = spec_from_file_location(path.stem, str(path))
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load {path}")
    mod = module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def extract_definition(module_path: Path) -> str:
    load_module(module_path)
    try:
        from flow_like_wasm_sdk import get_all_definitions
        defs = get_all_definitions()
        if defs:
            return json.dumps([d.to_dict() for d in defs], indent=2)
    except ImportError:
        pass
    raise RuntimeError(f"Module {module_path} must define WasmNode subclasses")


def copy_stdlib_modules(modules: list[str]) -> None:
    """Copy pure-Python stdlib modules into vendor/ so componentize-py includes them."""
    import sysconfig
    import shutil

    stdlib_dir = Path(sysconfig.get_paths()["stdlib"])
    if not stdlib_dir.exists():
        print(f"  Warning: stdlib dir not found at {stdlib_dir}, skipping stdlib bundling")
        return

    VENDOR_DIR.mkdir(parents=True, exist_ok=True)
    copied = 0
    for mod_name in modules:
        # Module could be a single .py file or a package directory
        py_file = stdlib_dir / f"{mod_name}.py"
        pkg_dir = stdlib_dir / mod_name

        dest_file = VENDOR_DIR / f"{mod_name}.py"
        dest_dir = VENDOR_DIR / mod_name

        if pkg_dir.is_dir() and (pkg_dir / "__init__.py").exists():
            if dest_dir.exists():
                shutil.rmtree(dest_dir)
            shutil.copytree(pkg_dir, dest_dir, ignore=shutil.ignore_patterns(
                "__pycache__", "*.pyc", "*.pyo", "*.so", "*.dylib",
            ))
            copied += 1
        elif py_file.exists():
            shutil.copy2(py_file, dest_file)
            copied += 1
        else:
            print(f"    ✗ stdlib {mod_name} (not found)")
    print(f"  Copied {copied}/{len(modules)} stdlib modules from {stdlib_dir}")


def download_wheels(packages: list[str]) -> None:
    """Download pure-Python wheels into vendor/ for bundling."""
    VENDOR_DIR.mkdir(parents=True, exist_ok=True)
    if not packages:
        return

    import shutil
    pip_cmd = shutil.which("pip3") or shutil.which("pip") or "pip3"

    print(f"  Downloading {len(packages)} packages to vendor/...")
    try:
        subprocess.run(
            [
                pip_cmd, "download",
                "--dest", str(VENDOR_DIR),
                "--only-binary=:all:",
                "--python-version=3.12",
                "--platform=any",
                "--no-deps",
                *packages,
            ],
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError as e:
        print(f"  Warning: Some packages failed to download: {e.stderr}")
        # Try one-by-one for partial success
        for pkg in packages:
            try:
                subprocess.run(
                    [
                        pip_cmd, "download",
                        "--dest", str(VENDOR_DIR),
                        "--only-binary=:all:",
                        "--python-version=3.12",
                        "--platform=any",
                        "--no-deps",
                        pkg,
                    ],
                    check=True,
                    capture_output=True,
                    text=True,
                )
                print(f"    ✓ {pkg}")
            except subprocess.CalledProcessError:
                # Retry with current-platform wheel (extracts .py fallbacks,
                # .so files are ignored by componentize-py)
                try:
                    subprocess.run(
                        [
                            pip_cmd, "download",
                            "--dest", str(VENDOR_DIR),
                            "--only-binary=:all:",
                            "--no-deps",
                            pkg,
                        ],
                        check=True,
                        capture_output=True,
                        text=True,
                    )
                    print(f"    ✓ {pkg} (platform wheel — .py fallbacks only)")
                except subprocess.CalledProcessError:
                    print(f"    ✗ {pkg} (C-extension or unavailable — skipped)")

    # Extract all wheels into vendor/ so they're importable
    import zipfile
    for whl in VENDOR_DIR.glob("*.whl"):
        with zipfile.ZipFile(whl) as zf:
            for member in zf.namelist():
                # Skip native extensions — only .py files matter for WASM
                if member.endswith((".so", ".dylib", ".pyd")):
                    continue
                zf.extract(member, VENDOR_DIR)
        whl.unlink()

    print(f"  Vendor packages ready in {VENDOR_DIR}")


def build_wasm(source: Path, output: Path | None = None, bundle: bool = True):
    if output is None:
        output = BUILD_DIR / "interpreter.wasm"

    BUILD_DIR.mkdir(parents=True, exist_ok=True)

    # Extract and save definitions
    definition_json = extract_definition(source)
    def_path = BUILD_DIR / "interpreter.definition.json"
    def_path.write_text(definition_json)
    print(f"  Node definitions → {def_path}")

    if not WIT_PATH.exists():
        print(f"  WIT definition not found at {WIT_PATH}")
        return

    app_path = ROOT / "app.py"
    if not app_path.exists():
        print("  app.py entry point not found.")
        return

    # Download pre-bundled packages and copy stdlib modules
    if bundle:
        copy_stdlib_modules(STDLIB_MODULES)
        download_wheels(BUNDLED_PACKAGES)

    try:
        from flow_like_wasm_sdk import SDK_DIR
        sdk_parent = str(SDK_DIR.parent)
    except ImportError:
        sdk_parent = str(ROOT)

    import shutil
    componentize_bin = shutil.which("componentize-py") or str(
        Path(sys.executable).parent / "componentize-py"
    )

    cmd = [
        componentize_bin,
        "-d", str(WIT_PATH),
        "-w", "flow-like-node",
        "componentize",
        "-p", str(ROOT),
        "-p", str(SRC_DIR),
        "-p", sdk_parent,
    ]

    # Include vendor dir if it exists and has content
    if bundle and VENDOR_DIR.exists() and any(VENDOR_DIR.iterdir()):
        cmd.extend(["-p", str(VENDOR_DIR)])

    cmd.extend(["app", "-o", str(output)])

    subprocess.run(cmd, check=True, cwd=str(ROOT))
    print(f"  WASM component → {output}")


def main():
    parser = argparse.ArgumentParser(description="Build Flow-Like Code Interpreter WASM component")
    parser.add_argument(
        "source", nargs="?", default=str(SRC_DIR / "node.py"),
        help="Python source file (default: src/node.py)",
    )
    parser.add_argument("-o", "--output", help="Output WASM path")
    parser.add_argument("--definition-only", action="store_true", help="Extract JSON definitions only")
    parser.add_argument("--no-bundle", action="store_true", help="Skip pre-bundling packages")
    args = parser.parse_args()

    source = Path(args.source).resolve()
    if not source.exists():
        print(f"Error: Source file not found: {source}")
        sys.exit(1)

    print(f"Building: {source.name}")

    if args.definition_only:
        definition = extract_definition(source)
        BUILD_DIR.mkdir(parents=True, exist_ok=True)
        out = BUILD_DIR / "interpreter.definition.json"
        out.write_text(definition)
        print(f"  Definition → {out}")
        print(definition)
    else:
        output = Path(args.output).resolve() if args.output else None
        build_wasm(source, output, bundle=not args.no_bundle)

    print("Done.")


if __name__ == "__main__":
    main()
