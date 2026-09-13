#!/usr/bin/env python3
"""Check extracted intent and shortcut declarations before distributing a build."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


SHORTCUTS = {
    "RunFlowEventIntent",
    "AskFlowPilotIntent",
    "AskAppChatIntent",
    "OpenFlowInboxIntent",
    "OpenFlowHomeIntent",
    "OpenFlowAppIntent",
}


def verify_metadata(path: Path, *, shortcuts: bool = True, module: str | None = None) -> None:
    metadata = json.loads(path.read_text())
    actions = metadata.get("actions", {})
    missing = SHORTCUTS - actions.keys()
    if missing:
        raise ValueError(f"Missing app intents: {', '.join(sorted(missing))}")
    for name in SHORTCUTS:
        action = actions[name]
        if not action.get("isDiscoverable") or not action.get("mangledTypeName"):
            raise ValueError(f"Intent cannot be discovered: {name}")
        if module and action.get("fullyQualifiedTypeName") != f"{module}.{name}":
            raise ValueError(f"Intent is not owned by target {module}: {name}")
    if shortcuts:
        exposed = {item["actionIdentifier"] for item in metadata.get("autoShortcuts", [])}
        missing = SHORTCUTS - exposed
        if missing:
            raise ValueError(f"Missing App Shortcuts: {', '.join(sorted(missing))}")
    text_type = {"primitive": {"wrapper": {"typeIdentifier": 0}}}
    for name in ("AskFlowPilotIntent", "AskAppChatIntent"):
        if actions[name].get("outputType") != text_type:
            raise ValueError(f"Shortcut must return Text: {name}")
    if actions["RunFlowEventIntent"].get("outputType") != {"entity": {"wrapper": {"typeName": "FlowEventResultEntity"}}}:
        raise ValueError("Run Event must return its result entity")
    result = metadata.get("entities", {}).get("FlowEventResultEntity", {})
    if not result.get("transient"):
        raise ValueError("Event result must be transient so Shortcuts preserves its returned values")
    properties = {item["identifier"]: item for item in result.get("properties", [])}
    for name in ("text", "json"):
        if properties.get(name, {}).get("valueType") != text_type:
            raise ValueError(f"Event result must expose {name} as Text")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", type=Path, help="Extracted Metadata.appintents/extract.actionsdata")
    parser.add_argument("--module", help="Expected target module, for example Flow_Like for iOS")
    parser.add_argument("--widget", action="store_true", help="Widget targets contain intents but no App Shortcuts provider")
    args = parser.parse_args()
    try:
        verify_metadata(args.metadata, shortcuts=not args.widget, module=args.module)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"error: {error}\n")
    print("Verified native intent metadata" + ("" if args.widget else f" and all {len(SHORTCUTS)} App Shortcuts"))
