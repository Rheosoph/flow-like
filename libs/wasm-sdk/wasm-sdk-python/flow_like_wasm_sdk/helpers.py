from __future__ import annotations

from typing import Any

from flow_like_wasm_sdk.types import NodeDefinition


def humanize(name: str) -> str:
    return " ".join(w.capitalize() for w in name.split("_") if w)


def node(
    name: str,
    friendly_name: str,
    description: str,
    category: str,
    **kwargs: Any,
) -> NodeDefinition:
    """Shorthand to create a NodeDefinition."""
    return NodeDefinition(name, friendly_name, description, category, **kwargs)
