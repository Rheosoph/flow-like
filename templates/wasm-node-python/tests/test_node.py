"""Tests for the main template nodes — definition and basic run checks."""

from conftest import make_context
from node import RepeatText, CharCount


class TestNodeDefinition:
    def test_repeat_text_name(self):
        nd = RepeatText().get_node()
        assert nd.name == "repeat_text_py"
        assert nd.category == "Custom/WASM"

    def test_repeat_text_pins(self):
        nd = RepeatText().get_node()
        pin_names = {p.name for p in nd.pins}
        assert "exec" in pin_names
        assert "exec_out" in pin_names
        assert "input_text" in pin_names
        assert "multiplier" in pin_names
        assert "output_text" in pin_names

    def test_pin_types(self):
        nd = RepeatText().get_node()
        by_name = {p.name: p for p in nd.pins}
        assert by_name["exec"].data_type == "Exec"
        assert by_name["input_text"].data_type == "String"
        assert by_name["multiplier"].data_type == "I64"
        assert by_name["output_text"].data_type == "String"

    def test_serialization(self):
        nd = RepeatText().get_node()
        d = nd.to_dict()
        assert d["name"] == "repeat_text_py"
        assert len(d["pins"]) >= 4

    def test_defaults(self):
        nd = RepeatText().get_node()
        by_name = {p.name: p for p in nd.pins}
        assert by_name["input_text"].default_value == ""
        assert by_name["multiplier"].default_value == 1

    def test_char_count_name(self):
        nd = CharCount().get_node()
        assert nd.name == "char_count_py"


_node = RepeatText()


class TestNodeRun:
    def test_basic_repeat(self):
        ctx = make_context({"input_text": "ab", "multiplier": 3})
        result = _node.run(ctx)
        assert result.error is None
        assert result.outputs["output_text"] == "ababab"
        assert "exec_out" in result.activate_exec

    def test_empty_text(self):
        ctx = make_context({"input_text": "", "multiplier": 5})
        result = _node.run(ctx)
        assert result.outputs["output_text"] == ""

    def test_zero_multiplier(self):
        ctx = make_context({"input_text": "hello", "multiplier": 0})
        result = _node.run(ctx)
        assert result.outputs["output_text"] == ""

    def test_negative_multiplier(self):
        ctx = make_context({"input_text": "hello", "multiplier": -3})
        result = _node.run(ctx)
        assert result.outputs["output_text"] == ""

    def test_default_inputs(self):
        ctx = make_context({})
        result = _node.run(ctx)
        assert result.error is None
        assert result.outputs["output_text"] == ""

    def test_single_char(self):
        ctx = make_context({"input_text": "x", "multiplier": 5})
        result = _node.run(ctx)
        assert result.outputs["output_text"] == "xxxxx"
