"""Tests for collection type support (list, dict, set) in declarative pins."""

import json

from pydantic import BaseModel

from conftest import make_context
from sdk import (
    ExecutionResult,
    Input,
    Output,
    PinType,
    ValueType,
    WasmNode,
    get_all_definitions,
)


# ── Models ──────────────────────────────────────────────────────────────

class Item(BaseModel):
    name: str = ""
    count: int = 0


class Tag(BaseModel):
    label: str = ""


# ── Nodes with collection pins ─────────────────────────────────────────


class ListStrings(WasmNode, name="test_list_strings", category="Test"):
    """Node with list[str] input/output."""

    items: list[str] = Input(default_factory=list)
    result: list[str] = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = [s.upper() for s in ctx.items]
        return ctx.success()


class ListModels(WasmNode, name="test_list_models", category="Test"):
    """Node with list[BaseModel] input/output."""

    items: list[Item] = Input(default_factory=list)
    result: list[Item] = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.items  # pass through
        return ctx.success()


class DictValues(WasmNode, name="test_dict_values", category="Test"):
    """Node with dict[str, int] input/output."""

    counts: dict[str, int] = Input(default_factory=dict)
    result: dict[str, int] = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = {k: v * 2 for k, v in ctx.counts.items()}
        return ctx.success()


class DictModels(WasmNode, name="test_dict_models", category="Test"):
    """Node with dict[str, BaseModel] input/output."""

    items: dict[str, Item] = Input(default_factory=dict)
    result: dict[str, Item] = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.items
        return ctx.success()


class SetStrings(WasmNode, name="test_set_strings", category="Test"):
    """Node with set[str] input/output."""

    tags: set[str] = Input(default_factory=set)
    result: set[str] = Output()

    def run(self, ctx) -> ExecutionResult:
        raw = ctx.tags
        if isinstance(raw, list):
            raw = set(raw)
        ctx.result = raw
        return ctx.success()


# ── Pin definition tests ───────────────────────────────────────────────


class TestCollectionPinDefinitions:
    """Verify that collection annotations produce correct data_type + value_type."""

    def _find_pin(self, node_name: str, pin_name: str):
        for nd in get_all_definitions():
            if nd.name == node_name:
                for p in nd.pins:
                    if p.name == pin_name:
                        return p
        raise AssertionError(f"Pin {pin_name} not found on {node_name}")

    def test_list_str_input_pin(self):
        pin = self._find_pin("test_list_strings", "items")
        assert pin.data_type == PinType.STRING
        assert pin.value_type == ValueType.ARRAY

    def test_list_str_output_pin(self):
        pin = self._find_pin("test_list_strings", "result")
        assert pin.data_type == PinType.STRING
        assert pin.value_type == ValueType.ARRAY

    def test_list_model_has_struct_array(self):
        pin = self._find_pin("test_list_models", "items")
        assert pin.data_type == PinType.STRUCT
        assert pin.value_type == ValueType.ARRAY

    def test_list_model_has_schema(self):
        pin = self._find_pin("test_list_models", "items")
        assert pin.schema is not None
        schema = json.loads(pin.schema)
        assert "properties" in schema
        assert "name" in schema["properties"]

    def test_list_model_enforce_schema(self):
        pin = self._find_pin("test_list_models", "items")
        assert pin.enforce_schema is True

    def test_dict_int_input_pin(self):
        pin = self._find_pin("test_dict_values", "counts")
        assert pin.data_type == PinType.I64
        assert pin.value_type == ValueType.HASH_MAP

    def test_dict_int_output_pin(self):
        pin = self._find_pin("test_dict_values", "result")
        assert pin.data_type == PinType.I64
        assert pin.value_type == ValueType.HASH_MAP

    def test_dict_model_has_struct_hashmap(self):
        pin = self._find_pin("test_dict_models", "items")
        assert pin.data_type == PinType.STRUCT
        assert pin.value_type == ValueType.HASH_MAP

    def test_dict_model_has_schema(self):
        pin = self._find_pin("test_dict_models", "items")
        assert pin.schema is not None
        schema = json.loads(pin.schema)
        assert "name" in schema["properties"]

    def test_set_str_input_pin(self):
        pin = self._find_pin("test_set_strings", "tags")
        assert pin.data_type == PinType.STRING
        assert pin.value_type == ValueType.HASH_SET

    def test_set_str_output_pin(self):
        pin = self._find_pin("test_set_strings", "result")
        assert pin.data_type == PinType.STRING
        assert pin.value_type == ValueType.HASH_SET


# ── TypedContext collection handling ────────────────────────────────────


class TestCollectionContext:
    """Verify TypedContext reads/writes collections correctly."""

    def test_list_str_roundtrip(self):
        ctx = make_context({"items": ["hello", "world"]})
        node = ListStrings()
        result = node.run(ctx)
        assert result.error is None
        assert result.outputs["result"] == ["HELLO", "WORLD"]

    def test_list_str_empty_default(self):
        ctx = make_context({})
        node = ListStrings()
        result = node.run(ctx)
        assert result.error is None
        assert result.outputs["result"] == []

    def test_list_model_read_validates(self):
        ctx = make_context({
            "items": [
                {"name": "a", "count": 1},
                {"name": "b", "count": 2},
            ]
        })
        node = ListModels()
        result = node.run(ctx)
        assert result.error is None
        # Output should be serialized back to dicts
        out = result.outputs["result"]
        assert isinstance(out, list)
        assert len(out) == 2
        assert out[0]["name"] == "a"
        assert out[1]["count"] == 2

    def test_dict_roundtrip(self):
        ctx = make_context({"counts": {"a": 3, "b": 5}})
        node = DictValues()
        result = node.run(ctx)
        assert result.error is None
        assert result.outputs["result"] == {"a": 6, "b": 10}

    def test_dict_model_read_validates(self):
        ctx = make_context({
            "items": {
                "x": {"name": "first", "count": 10},
                "y": {"name": "second", "count": 20},
            }
        })
        node = DictModels()
        result = node.run(ctx)
        assert result.error is None
        out = result.outputs["result"]
        assert isinstance(out, dict)
        assert out["x"]["name"] == "first"
        assert out["y"]["count"] == 20

    def test_set_str_roundtrip(self):
        # Runtime sends sets as lists (JSON has no set type)
        ctx = make_context({"tags": ["a", "b", "c"]})
        node = SetStrings()
        result = node.run(ctx)
        assert result.error is None
        # Output: serialized from set, order may vary
        out = result.outputs["result"]
        assert isinstance(out, list)
        assert set(out) == {"a", "b", "c"}


# ── Descriptor override ────────────────────────────────────────────────


class TestDescriptorOverride:
    """Verify that explicit value_type in descriptor overrides auto-detection."""

    def test_explicit_value_type_overrides(self):
        class OverrideNode(WasmNode, name="test_override_value_type", category="Test"):

            # Annotated as list[str] but descriptor forces Normal
            data: list[str] = Input(value_type=ValueType.NORMAL)
            result: float = Output()

            def run(self, ctx) -> ExecutionResult:
                return ctx.success()

        for nd in get_all_definitions():
            if nd.name == "test_override_value_type":
                for p in nd.pins:
                    if p.name == "data":
                        # Descriptor override wins
                        assert p.value_type == ValueType.NORMAL
                        return
        raise AssertionError("Node not found")


# ── Registration ────────────────────────────────────────────────────────


class TestRegistration:
    def test_collection_nodes_registered(self):
        names = {nd.name for nd in get_all_definitions()}
        assert "test_list_strings" in names
        assert "test_list_models" in names
        assert "test_dict_values" in names
        assert "test_dict_models" in names
        assert "test_set_strings" in names
