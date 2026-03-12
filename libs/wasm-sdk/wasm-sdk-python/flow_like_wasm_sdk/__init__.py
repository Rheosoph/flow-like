from __future__ import annotations

from pathlib import Path

from flow_like_wasm_sdk.types import (
    ABI_VERSION,
    DataType,
    LogLevel,
    PinType,
    ValueType,
    NodeScores,
    PinDefinition,
    NodeDefinition,
    PackageNodes,
    ExecutionInput,
    ExecutionResult,
)
from flow_like_wasm_sdk.context import Context
from flow_like_wasm_sdk.host import HostBridge, MockHostBridge, set_host, get_host
from flow_like_wasm_sdk.helpers import node, humanize

SDK_DIR = Path(__file__).resolve().parent
WIT_PATH = SDK_DIR / "wit" / "flow-like-node.wit"
BRIDGE_MODULE = SDK_DIR / "bridge.py"


def get_wit_path() -> Path:
    """Return the path to the WIT interface definition shipped with this SDK."""
    return WIT_PATH


def get_bridge_path() -> Path:
    """Return the path to the componentize-py bridge module shipped with this SDK."""
    return BRIDGE_MODULE


__all__ = [
    "ABI_VERSION",
    "DataType",
    "LogLevel",
    "PinType",
    "ValueType",
    "NodeScores",
    "PinDefinition",
    "NodeDefinition",
    "PackageNodes",
    "ExecutionInput",
    "ExecutionResult",
    "Context",
    "HostBridge",
    "MockHostBridge",
    "set_host",
    "get_host",
    "node",
    "humanize",
    "SDK_DIR",
    "WIT_PATH",
    "BRIDGE_MODULE",
    "get_wit_path",
    "get_bridge_path",
]
