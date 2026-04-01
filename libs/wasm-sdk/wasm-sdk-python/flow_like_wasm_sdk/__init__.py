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
    Exec,
    Input,
    Output,
    ExecInput,
    ExecOutput,
    TypedContext,
    WasmNode,
    get_registered_nodes,
    get_all_definitions,
    run_node,
)
from flow_like_wasm_sdk.interop import (  # noqa: F401 — public re-exports
    FlowPath,
    FlowImage,
    Bit,
    ChatMessage,
    ContentPart,
    CachedEmbeddingModel,
    NodeDBConnection,
    VectorSearchQuery,
    FtsSearchQuery,
    HybridSearchQuery,
    ImageData,
    AudioData,
    VideoData,
    DocumentData,
    ToolCallData,
    ToolResultData,
    ReasoningData,
)
from flow_like_wasm_sdk.context import Context
from flow_like_wasm_sdk.host import HostBridge, MockHostBridge, set_host, get_host
from flow_like_wasm_sdk.helpers import node, humanize

__all__ = [
    "ABI_VERSION",
    "AudioData",
    "Bit",
    "BRIDGE_MODULE",
    "CachedEmbeddingModel",
    "ChatMessage",
    "ContentPart",
    "Context",
    "DataType",
    "DocumentData",
    "Exec",
    "ExecInput",
    "ExecOutput",
    "ExecutionInput",
    "ExecutionResult",
    "FlowImage",
    "FlowPath",
    "FtsSearchQuery",
    "HostBridge",
    "HybridSearchQuery",
    "humanize",
    "ImageData",
    "Input",
    "LogLevel",
    "MockHostBridge",
    "node",
    "NodeDBConnection",
    "NodeDefinition",
    "NodeScores",
    "Output",
    "PackageNodes",
    "PinDefinition",
    "PinType",
    "ReasoningData",
    "SDK_DIR",
    "set_host",
    "get_host",
    "get_all_definitions",
    "get_registered_nodes",
    "get_wit_path",
    "run_node",
    "ToolCallData",
    "ToolResultData",
    "TypedContext",
    "ValueType",
    "VectorSearchQuery",
    "VideoData",
    "WasmNode",
    "WIT_PATH",
]

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
    "Exec",
    "Input",
    "Output",
    "ExecInput",
    "ExecOutput",
    "TypedContext",
    "WasmNode",
    "get_registered_nodes",
    "get_all_definitions",
    "run_node",
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
