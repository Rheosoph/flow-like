"""
WASM Component Model entry point for Flow-Like Python nodes.

This module implements the WIT world exports (get-node, run, get-abi-version)
and connects the WIT host imports to the SDK's HostBridge.

The WitHostBridge and WIT definition are provided by the SDK — this file only
wires the node module to the WIT world exports.

Nodes are auto-discovered: importing the node module triggers WasmNode
subclass registration (via __init_subclass__), so get_all_definitions()
and run_node() work automatically — no manual DISPATCH dict needed.

Usage:
    componentize-py -d <wit-path> -w flow-like-node componentize app -o build/node.wasm
"""

from __future__ import annotations

import json

import node as _node_mod  # noqa: F401 — triggers WasmNode subclass registration
from flow_like_wasm_sdk.bridge import _make_bridge
from flow_like_wasm_sdk import ABI_VERSION, Context, get_all_definitions, run_node, set_host

# Pre-import optional SDK submodules so componentize-py bundles them into the WASM component.
# Without this, lazy imports inside node run() methods fail with ModuleNotFoundError at runtime.
import flow_like_wasm_sdk._langchain_stubs as _stubs  # noqa: F401
import flow_like_wasm_sdk.langchain as _langchain  # noqa: F401

_bridge = _make_bridge()
set_host(_bridge)


class WitWorld:
    def get_node(self) -> str:
        defs = get_all_definitions()
        return json.dumps([d.to_dict() for d in defs])

    def get_nodes(self) -> str:
        return self.get_node()

    def run(self, input_json: str) -> str:
        try:
            ctx = Context.from_json(input_json, _bridge)
            result = run_node(ctx.node_name, ctx)
            return json.dumps(result.to_dict())
        except Exception as exc:
            from flow_like_wasm_sdk.types import ExecutionResult
            return json.dumps(ExecutionResult.fail(f"{type(exc).__name__}: {exc}").to_dict())

    def get_abi_version(self) -> int:
        return ABI_VERSION
