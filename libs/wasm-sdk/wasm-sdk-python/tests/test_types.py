from __future__ import annotations

import json

import pytest

from flow_like_wasm_sdk.types import (
    ABI_VERSION,
    ExecutionInput,
    ExecutionResult,
    LogLevel,
    NodeDefinition,
    NodeScores,
    PackageNodes,
    PinDefinition,
    PinType,
    ValueType,
)


class TestPinType:
    def test_validate_valid(self) -> None:
        assert PinType.validate("String") == "String"

    def test_validate_invalid(self) -> None:
        with pytest.raises(ValueError, match="Invalid pin data type"):
            PinType.validate("Unknown")


class TestNodeScores:
    def test_defaults(self) -> None:
        s = NodeScores()
        assert s.to_dict() == {
            "privacy": 0, "security": 0, "performance": 0,
            "governance": 0, "reliability": 0, "cost": 0,
        }

    def test_custom(self) -> None:
        s = NodeScores(privacy=5, cost=3)
        d = s.to_dict()
        assert d["privacy"] == 5
        assert d["cost"] == 3


class TestPinDefinition:
    def test_input_pin(self) -> None:
        p = PinDefinition.input_pin("my_input", "String", default="hello")
        assert p.pin_type == "Input"
        assert p.data_type == "String"
        assert p.default_value == "hello"
        assert p.friendly_name == "My Input"

    def test_output_pin(self) -> None:
        p = PinDefinition.output_pin("result", "I64")
        assert p.pin_type == "Output"
        assert p.data_type == "I64"

    def test_exec_pins(self) -> None:
        ie = PinDefinition.input_exec()
        oe = PinDefinition.output_exec()
        assert ie.data_type == PinType.EXEC
        assert oe.data_type == PinType.EXEC

    def test_builder_methods(self) -> None:
        p = (
            PinDefinition.input_pin("x", "String")
            .with_default("abc")
            .with_value_type(ValueType.ARRAY)
            .with_schema('{"type":"string"}')
            .with_valid_values(["a", "b"])
            .with_range(0.0, 100.0)
        )
        d = p.to_dict()
        assert d["default_value"] == "abc"
        assert d["value_type"] == "Array"
        assert d["schema"] == '{"type":"string"}'
        assert d["valid_values"] == ["a", "b"]
        assert d["range"] == [0.0, 100.0]

    def test_to_dict_omits_none(self) -> None:
        p = PinDefinition.input_pin("x", "Bool")
        d = p.to_dict()
        assert "default_value" not in d
        assert "schema" not in d


class TestNodeDefinition:
    def test_basic(self) -> None:
        n = NodeDefinition("test", "Test", "desc", "Cat")
        n.add_pin(PinDefinition.input_exec())
        n.add_pin(PinDefinition.output_exec())
        d = n.to_dict()
        assert d["name"] == "test"
        assert d["abi_version"] == ABI_VERSION
        assert len(d["pins"]) == 2

    def test_scores_and_long_running(self) -> None:
        n = NodeDefinition("n", "N", "d", "C")
        n.set_scores(NodeScores(security=8))
        n.set_long_running(True)
        d = n.to_dict()
        assert d["scores"]["security"] == 8
        assert d["long_running"] is True

    def test_to_json_roundtrip(self) -> None:
        n = NodeDefinition("n", "N", "d", "C")
        parsed = json.loads(n.to_json())
        assert parsed["name"] == "n"


class TestPackageNodes:
    def test_empty(self) -> None:
        pkg = PackageNodes()
        assert pkg.to_dict() == []

    def test_add_nodes(self) -> None:
        pkg = PackageNodes()
        pkg.add_node(NodeDefinition("a", "A", "da", "C"))
        pkg.add_node(NodeDefinition("b", "B", "db", "C"))
        assert len(pkg.to_dict()) == 2

    def test_to_json(self) -> None:
        pkg = PackageNodes()
        pkg.add_node(NodeDefinition("x", "X", "dx", "C"))
        parsed = json.loads(pkg.to_json())
        assert isinstance(parsed, list)
        assert parsed[0]["name"] == "x"


class TestExecutionInput:
    def test_from_dict(self) -> None:
        ei = ExecutionInput.from_dict({
            "inputs": {"a": 1},
            "node_id": "n1",
            "stream_state": True,
            "log_level": LogLevel.DEBUG,
        })
        assert ei.inputs == {"a": 1}
        assert ei.node_id == "n1"
        assert ei.stream_state is True
        assert ei.log_level == LogLevel.DEBUG

    def test_from_json(self) -> None:
        data = json.dumps({"inputs": {"b": 2}, "run_id": "r1"})
        ei = ExecutionInput.from_json(data)
        assert ei.inputs == {"b": 2}
        assert ei.run_id == "r1"

    def test_defaults(self) -> None:
        ei = ExecutionInput.from_dict({})
        assert ei.inputs == {}
        assert ei.node_id == ""
        assert ei.stream_state is False


class TestExecutionResult:
    def test_ok(self) -> None:
        r = ExecutionResult.ok()
        assert r.error is None
        assert r.outputs == {}

    def test_fail(self) -> None:
        r = ExecutionResult.fail("boom")
        assert r.error == "boom"

    def test_builder(self) -> None:
        r = ExecutionResult.ok().set_output("x", 42).exec("done").set_pending(True)
        d = r.to_dict()
        assert d["outputs"]["x"] == 42
        assert "done" in d["activate_exec"]
        assert d["pending"] is True

    def test_to_json(self) -> None:
        r = ExecutionResult.ok().set_output("v", "hi")
        parsed = json.loads(r.to_json())
        assert parsed["outputs"]["v"] == "hi"
