"""Tests for BaseModel integration with declarative pins."""

import json

from pydantic import BaseModel

from conftest import make_context
from sdk import (
    ExecutionResult,
    Input,
    Output,
    WasmNode,
    get_all_definitions,
)


class Config(BaseModel):
    threshold: float = 0.5
    label: str = "default"


class Nested(BaseModel):
    name: str
    config: Config


class ModelNode(WasmNode, name="model_node_test", title="Model Node", category="Test"):
    """Processes a config model"""

    config: Config = Input(default_factory=lambda: Config())
    result_config: Config = Output()
    label_out: str = Output()

    def run(self, ctx) -> ExecutionResult:
        cfg = ctx.config
        ctx.label_out = cfg.label
        ctx.result_config = cfg
        return ctx.success()


class NestedModelNode(WasmNode, name="nested_model_test", title="Nested Model", category="Test"):
    """Processes a nested model"""

    data: Nested = Input()
    result: Nested = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.data
        return ctx.success()


class TestBaseModelPinDefinition:
    def test_data_type_is_struct(self):
        nd = ModelNode().get_node()
        by_name = {p.name: p for p in nd.pins}
        assert by_name["config"].data_type == "Struct"
        assert by_name["result_config"].data_type == "Struct"

    def test_json_schema_injected_on_input(self):
        nd = ModelNode().get_node()
        by_name = {p.name: p for p in nd.pins}
        pin = by_name["config"]
        assert pin.schema is not None
        schema = json.loads(pin.schema)
        assert "properties" in schema
        assert "threshold" in schema["properties"]
        assert "label" in schema["properties"]

    def test_enforce_schema_on_input(self):
        nd = ModelNode().get_node()
        by_name = {p.name: p for p in nd.pins}
        assert by_name["config"].enforce_schema is True

    def test_json_schema_injected_on_output(self):
        nd = ModelNode().get_node()
        by_name = {p.name: p for p in nd.pins}
        pin = by_name["result_config"]
        assert pin.schema is not None
        schema = json.loads(pin.schema)
        assert "threshold" in schema["properties"]

    def test_nested_model_schema(self):
        nd = NestedModelNode().get_node()
        by_name = {p.name: p for p in nd.pins}
        schema = json.loads(by_name["data"].schema)
        assert "name" in schema["properties"]


class TestBaseModelValidation:
    def test_dict_input_becomes_model(self):
        ctx = make_context(
            {"config": {"threshold": 0.9, "label": "custom"}},
            node_name="model_node_test",
        )
        result = ModelNode().run(ctx)
        assert result.error is None
        assert result.outputs["label_out"] == "custom"

    def test_model_output_serialized_to_dict(self):
        ctx = make_context(
            {"config": {"threshold": 0.9, "label": "custom"}},
            node_name="model_node_test",
        )
        result = ModelNode().run(ctx)
        rc = result.outputs["result_config"]
        assert isinstance(rc, dict)
        assert rc["threshold"] == 0.9
        assert rc["label"] == "custom"

    def test_default_factory_used_when_no_input(self):
        ctx = make_context({}, node_name="model_node_test")
        result = ModelNode().run(ctx)
        assert result.error is None
        rc = result.outputs["result_config"]
        assert rc["threshold"] == 0.5
        assert rc["label"] == "default"

    def test_nested_model_roundtrip(self):
        data = {"name": "test", "config": {"threshold": 0.8, "label": "inner"}}
        ctx = make_context({"data": data}, node_name="nested_model_test")
        result = NestedModelNode().run(ctx)
        assert result.error is None
        out = result.outputs["result"]
        assert isinstance(out, dict)
        assert out["name"] == "test"
        assert out["config"]["threshold"] == 0.8

    def test_invalid_data_raises_validation_error(self):
        ctx = make_context(
            {"config": {"threshold": "not_a_number", "label": 123}},
            node_name="model_node_test",
        )
        # Pydantic coerces compatible types, so "not_a_number" should raise
        import pytest
        with pytest.raises(Exception):
            ModelNode().run(ctx)


class TestRegistration:
    def test_model_nodes_registered(self):
        names = {d.name for d in get_all_definitions()}
        assert "model_node_test" in names
        assert "nested_model_test" in names
