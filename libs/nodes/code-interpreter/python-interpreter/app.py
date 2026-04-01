"""
WASM Component Model entry point for the Flow-Like Code Interpreter.

Implements the WIT world exports (get-node, get-nodes, run, get-abi-version)
and connects the WIT host imports to the SDK's HostBridge.

Two nodes are exported:
  - PythonEval    — executes inline Python code (string input)
  - PythonProject — executes a Python project from a FlowPath directory
  - CodeAgent     — LLM agent that solves tasks by writing & running Python code
"""

from __future__ import annotations

import json

import _preload  # noqa: F401 — force-bundle stdlib & vendor packages
import node as _node_mod  # noqa: F401 — triggers WasmNode subclass registration
from flow_like_wasm_sdk import ABI_VERSION, Context, get_all_definitions, run_node, set_host
from flow_like_wasm_sdk.bridge import _make_bridge

_bridge = _make_bridge()
set_host(_bridge)


class WitWorld:
    def get_node(self) -> str:
        defs = get_all_definitions()
        return json.dumps([d.to_dict() for d in defs])

    def get_nodes(self) -> str:
        return self.get_node()

    def run(self, input_json: str) -> str:
        ctx = Context.from_json(input_json, _bridge)
        result = run_node(ctx.node_name, ctx)
        return json.dumps(result.to_dict())

    def get_abi_version(self) -> int:
        return ABI_VERSION
