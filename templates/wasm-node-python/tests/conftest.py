"""Shared test fixtures for all node tests."""

import sys
from pathlib import Path

import pytest

# Add src and examples to path so tests can import them
# src must come first so src/node.py takes precedence over examples/node.py
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "examples"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sdk import Context, ExecutionInput, MockHostBridge


@pytest.fixture
def host() -> MockHostBridge:
    return MockHostBridge()


def make_context(
    inputs: dict | None = None,
    *,
    host: MockHostBridge | None = None,
    node_name: str = "",
    stream: bool = False,
    log_level: int = 0,
) -> Context:
    """Helper to build a Context with the given inputs."""
    ei = ExecutionInput(
        inputs=inputs or {},
        node_id="test-node-id",
        run_id="test-run-id",
        app_id="test-app-id",
        board_id="test-board-id",
        user_id="test-user-id",
        stream_state=stream,
        log_level=log_level,
        node_name=node_name,
    )
    return Context(ei, host or MockHostBridge())


# Ensure all modules are imported (registering their WasmNode subclasses)
# before any test runs, so the global registry is stable.
import node as _node_mod  # noqa: F401, E402  (src/node.py — main nodes)
import math_nodes as _math_mod  # noqa: F401, E402
import string_nodes as _string_mod  # noqa: F401, E402
import control_flow as _control_mod  # noqa: F401, E402
import permissions as _perms_mod  # noqa: F401, E402

# Also import the examples/node.py example (shadowed by src/node.py on sys.path)
import importlib.util as _iu  # noqa: E402
_spec = _iu.spec_from_file_location(
    "example_node",
    str(Path(__file__).resolve().parent.parent / "examples" / "node.py"),
)
_example_node = _iu.module_from_spec(_spec)  # type: ignore[arg-type]
_spec.loader.exec_module(_example_node)  # type: ignore[union-attr]
