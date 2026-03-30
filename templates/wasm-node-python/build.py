#!/usr/bin/env python3
"""
Build script for Flow-Like Python WASM nodes.

Generates a WASM module from your Python node using componentize-py.

Usage:
    uv run python build.py                    # Build src/node.py
    uv run python build.py examples/math_nodes.py  # Build an example

Prerequisites:
    uv sync --group build
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

# WIT file: prefer the SDK-shipped copy, fall back to local wit/ directory
_LOCAL_WIT = ROOT / "wit" / "flow-like-node.wit"
try:
    from flow_like_wasm_sdk import WIT_PATH as _SDK_WIT
    WIT_PATH = _SDK_WIT if _SDK_WIT.exists() else _LOCAL_WIT
except ImportError:
    WIT_PATH = _LOCAL_WIT


def load_module(path: Path):
    """Dynamically load a Python module from a file path."""
    sys.path.insert(0, str(SRC_DIR))
    spec = spec_from_file_location(path.stem, str(path))
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load {path}")
    mod = module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def extract_definition(module_path: Path) -> str:
    """Extract the node definition JSON from a Python node module.

    Supports both the class-based pattern (WasmNode subclasses with
    auto-registration) and the legacy function-based pattern
    (get_definition / get_definitions).
    """
    mod = load_module(module_path)

    # Class-based: importing the module registers WasmNode subclasses
    try:
        from flow_like_wasm_sdk import get_all_definitions
        defs = get_all_definitions()
        if defs:
            return json.dumps([d.to_dict() for d in defs], indent=2)
    except ImportError:
        pass

    # Legacy function-based fallback
    if hasattr(mod, "get_definition"):
        nd = mod.get_definition()
        return json.dumps(nd.to_dict(), indent=2)
    elif hasattr(mod, "get_definitions"):
        defs = mod.get_definitions()
        return json.dumps([d.to_dict() for d in defs], indent=2)

    raise RuntimeError(f"Module {module_path} must define WasmNode subclasses or export get_definition()/get_definitions()")


def build_wasm(source: Path, output: Path | None = None):
    """Build a WASM module from a Python node source file."""
    if output is None:
        output = BUILD_DIR / f"{source.stem}.wasm"

    BUILD_DIR.mkdir(parents=True, exist_ok=True)

    # Extract and save the node definition for reference
    definition_json = extract_definition(source)
    def_path = BUILD_DIR / f"{source.stem}.definition.json"
    def_path.write_text(definition_json)
    print(f"  Node definition → {def_path}")

    # Build with componentize-py
    if not WIT_PATH.exists():
        print(f"  WIT definition not found at {WIT_PATH}")
        print("  Skipping WASM compilation — WIT file missing")
        return

    # Prepare the app module that bridges WIT ↔ SDK.
    # componentize-py compiles app.py which imports the user's node module.
    app_path = ROOT / "app.py"
    if not app_path.exists():
        print("  app.py entry point not found.")
        return

    try:
        # Resolve SDK package location for componentize-py path inclusion
        try:
            from flow_like_wasm_sdk import SDK_DIR
            sdk_parent = str(SDK_DIR.parent)
        except ImportError:
            sdk_parent = str(ROOT)

        cmd = [
            "componentize-py",
            "-d", str(WIT_PATH),
            "-w", "flow-like-node",
            "componentize",
            "-p", str(ROOT),
            "-p", str(SRC_DIR),
            "-p", sdk_parent,
            "app",
            "-o", str(output),
        ]
        subprocess.run(cmd, check=True, cwd=str(ROOT))
        print(f"  WASM component → {output}")
    except FileNotFoundError:
        print("  componentize-py not found. Install with: uv sync --group build")
        print("  The node definition JSON has been generated for reference.")
    except subprocess.CalledProcessError as e:
        print(f"  Build failed: {e}")
        sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="Build Flow-Like Python WASM nodes")
    parser.add_argument(
        "source",
        nargs="?",
        default=str(SRC_DIR / "node.py"),
        help="Python source file to build (default: src/node.py)",
    )
    parser.add_argument(
        "-o", "--output",
        help="Output WASM file path (default: build/<name>.wasm)",
    )
    parser.add_argument(
        "--definition-only",
        action="store_true",
        help="Only extract the node definition JSON without building WASM",
    )
    args = parser.parse_args()

    source = Path(args.source).resolve()
    if not source.exists():
        print(f"Error: Source file not found: {source}")
        sys.exit(1)

    print(f"Building: {source.name}")

    if args.definition_only:
        definition = extract_definition(source)
        out = BUILD_DIR / f"{source.stem}.definition.json"
        BUILD_DIR.mkdir(parents=True, exist_ok=True)
        out.write_text(definition)
        print(f"  Definition → {out}")
        print(definition)
    else:
        output = Path(args.output).resolve() if args.output else None
        build_wasm(source, output)

    print("Done.")


if __name__ == "__main__":
    main()
