"""Tests for the SDK types and Context."""

import json

from conftest import make_context

from sdk import (
    ABI_VERSION,
    Context,
    ExecutionInput,
    ExecutionResult,
    LogLevel,
    MockHostBridge,
    NodeDefinition,
    NodeScores,
    PinDefinition,
    PinType,
)


class TestPinDefinition:
    def test_input_pin(self):
        pin = PinDefinition.input_pin("value", PinType.F64, default=3.14)
        assert pin.name == "value"
        assert pin.pin_type == "Input"
        assert pin.data_type == "F64"
        assert pin.default_value == 3.14
        assert pin.friendly_name == "Value"

    def test_output_pin(self):
        pin = PinDefinition.output_pin("result", PinType.STRING)
        assert pin.name == "result"
        assert pin.pin_type == "Output"
        assert pin.data_type == "String"
        assert pin.default_value is None

    def test_exec_pins(self):
        inp = PinDefinition.input_exec("exec")
        out = PinDefinition.output_exec("exec_out")
        assert inp.data_type == "Exec"
        assert out.data_type == "Exec"
        assert inp.pin_type == "Input"
        assert out.pin_type == "Output"

    def test_with_default(self):
        pin = PinDefinition.input_pin("x", PinType.I64).with_default(42)
        assert pin.default_value == 42

    def test_with_valid_values(self):
        pin = PinDefinition.input_pin("mode", PinType.STRING).with_valid_values(["a", "b"])
        assert pin.valid_values == ["a", "b"]

    def test_with_range(self):
        pin = PinDefinition.input_pin("pct", PinType.F64).with_range(0.0, 100.0)
        assert pin.range == (0.0, 100.0)

    def test_invalid_type_raises(self):
        import pytest
        with pytest.raises(ValueError, match="Invalid pin data type"):
            PinDefinition.input_pin("x", "InvalidType")

    def test_to_dict_minimal(self):
        pin = PinDefinition.input_pin("x", PinType.I64)
        d = pin.to_dict()
        assert d["name"] == "x"
        assert "default_value" not in d
        assert "schema" not in d

    def test_to_dict_full(self):
        pin = (
            PinDefinition.input_pin("x", PinType.F64, default=1.0)
            .with_schema("number")
            .with_value_type("Array")
        )
        d = pin.to_dict()
        assert d["default_value"] == 1.0
        assert d["schema"] == "number"
        assert d["value_type"] == "Array"


class TestNodeDefinition:
    def test_basic_creation(self):
        nd = NodeDefinition("test", "Test Node", "A test", "Test/Category")
        assert nd.name == "test"
        assert nd.abi_version == ABI_VERSION
        assert nd.pins == []

    def test_add_pins(self):
        nd = NodeDefinition("test", "Test", "desc", "cat")
        nd.add_pin(PinDefinition.input_exec("exec"))
        nd.add_pin(PinDefinition.output_exec("exec_out"))
        assert len(nd.pins) == 2

    def test_set_scores(self):
        nd = NodeDefinition("test", "Test", "desc", "cat")
        scores = NodeScores(privacy=5, security=4)
        nd.set_scores(scores)
        assert nd.scores is not None
        assert nd.scores.privacy == 5

    def test_to_json_roundtrip(self):
        nd = NodeDefinition("test", "Test", "desc", "cat")
        nd.add_pin(PinDefinition.input_pin("x", PinType.I64, default=0))
        json_str = nd.to_json()
        parsed = json.loads(json_str)
        assert parsed["name"] == "test"
        assert len(parsed["pins"]) == 1
        assert parsed["pins"][0]["default_value"] == 0

    def test_long_running(self):
        nd = NodeDefinition("test", "Test", "desc", "cat")
        nd.set_long_running(True)
        d = nd.to_dict()
        assert d["long_running"] is True


class TestExecutionInput:
    def test_from_dict(self):
        data = {
            "inputs": {"a": 1, "b": "hello"},
            "node_id": "n1",
            "run_id": "r1",
            "stream_state": True,
            "log_level": 2,
        }
        ei = ExecutionInput.from_dict(data)
        assert ei.inputs == {"a": 1, "b": "hello"}
        assert ei.node_id == "n1"
        assert ei.stream_state is True
        assert ei.log_level == 2

    def test_from_json(self):
        data = json.dumps({"inputs": {"x": 42}, "node_name": "test_node"})
        ei = ExecutionInput.from_json(data)
        assert ei.inputs["x"] == 42
        assert ei.node_name == "test_node"

    def test_defaults(self):
        ei = ExecutionInput.from_dict({})
        assert ei.inputs == {}
        assert ei.node_id == ""
        assert ei.stream_state is False


class TestExecutionResult:
    def test_ok(self):
        r = ExecutionResult.ok()
        assert r.error is None
        assert r.outputs == {}
        assert r.activate_exec == []

    def test_fail(self):
        r = ExecutionResult.fail("something broke")
        assert r.error == "something broke"

    def test_set_output(self):
        r = ExecutionResult.ok()
        r.set_output("x", 42).set_output("y", "hello")
        assert r.outputs["x"] == 42
        assert r.outputs["y"] == "hello"

    def test_activate_exec(self):
        r = ExecutionResult.ok()
        r.exec("out_1").exec("out_2")
        assert r.activate_exec == ["out_1", "out_2"]

    def test_pending(self):
        r = ExecutionResult.ok()
        r.set_pending(True)
        d = r.to_dict()
        assert d["pending"] is True

    def test_to_json(self):
        r = ExecutionResult.ok()
        r.set_output("val", 99)
        parsed = json.loads(r.to_json())
        assert parsed["outputs"]["val"] == 99
        assert "error" not in parsed


class TestContext:
    def test_get_string(self):
        ctx = make_context({"name": "hello"})
        assert ctx.get_string("name") == "hello"
        assert ctx.get_string("missing") is None

    def test_get_i64(self):
        ctx = make_context({"count": 42})
        assert ctx.get_i64("count") == 42
        assert ctx.get_i64("missing") is None

    def test_get_f64(self):
        ctx = make_context({"value": 3.14})
        assert ctx.get_f64("value") == 3.14

    def test_get_bool(self):
        ctx = make_context({"flag": True})
        assert ctx.get_bool("flag") is True

    def test_require_input(self):
        ctx = make_context({"x": 1})
        assert ctx.require_input("x") == 1

    def test_require_input_missing(self):
        import pytest
        ctx = make_context({})
        with pytest.raises(ValueError, match="Required input"):
            ctx.require_input("missing")

    def test_set_output_and_success(self):
        ctx = make_context({})
        ctx.set_output("result", "done")
        result = ctx.success()
        assert result.outputs["result"] == "done"
        assert "exec_out" in result.activate_exec
        assert result.error is None

    def test_fail(self):
        ctx = make_context({})
        result = ctx.fail("oops")
        assert result.error == "oops"
        assert "exec_out" not in result.activate_exec

    def test_finish_no_exec(self):
        ctx = make_context({})
        ctx.activate_exec("custom_out")
        result = ctx.finish()
        assert result.activate_exec == ["custom_out"]

    def test_metadata(self):
        ctx = make_context({}, node_name="test_node")
        assert ctx.node_name == "test_node"
        assert ctx.node_id == "test-node-id"
        assert ctx.run_id == "test-run-id"

    def test_logging(self):
        host = MockHostBridge()
        ctx = make_context({}, host=host, log_level=0)
        ctx.debug("d")
        ctx.info("i")
        ctx.warn("w")
        ctx.error("e")
        assert len(host.logs) == 4
        assert host.logs[0] == (LogLevel.DEBUG, "d")
        assert host.logs[3] == (LogLevel.ERROR, "e")

    def test_logging_level_gating(self):
        host = MockHostBridge()
        ctx = make_context({}, host=host, log_level=LogLevel.WARN)
        ctx.debug("nope")
        ctx.info("nope")
        ctx.warn("yes")
        ctx.error("yes")
        assert len(host.logs) == 2

    def test_streaming(self):
        host = MockHostBridge()
        ctx = make_context({}, host=host, stream=True)
        ctx.stream_text("hello")
        ctx.stream_progress(0.5, "halfway")
        assert len(host.streams) == 2
        assert host.streams[0] == ("text", "hello")

    def test_streaming_disabled(self):
        host = MockHostBridge()
        ctx = make_context({}, host=host, stream=False)
        ctx.stream_text("should not appear")
        assert len(host.streams) == 0

    def test_variables(self):
        host = MockHostBridge()
        ctx = make_context({}, host=host)
        assert ctx.set_variable("key", "value") is True
        assert ctx.get_variable("key") == "value"
        assert ctx.get_variable("missing") is None

    def test_from_json(self):
        json_str = json.dumps({
            "inputs": {"x": 10},
            "node_id": "n",
            "run_id": "r",
            "app_id": "a",
            "board_id": "b",
            "user_id": "u",
            "stream_state": False,
            "log_level": 1,
            "node_name": "test",
        })
        ctx = Context.from_json(json_str)
        assert ctx.get_i64("x") == 10
        assert ctx.node_name == "test"


class TestMockHostBridge:
    def test_variables_roundtrip(self):
        host = MockHostBridge()
        host.set_variable("a", [1, 2, 3])
        assert host.get_variable("a") == [1, 2, 3]
        assert host.get_variable("missing") is None

    def test_log_capture(self):
        host = MockHostBridge()
        host.log(LogLevel.INFO, "test message")
        assert host.logs == [(LogLevel.INFO, "test message")]
