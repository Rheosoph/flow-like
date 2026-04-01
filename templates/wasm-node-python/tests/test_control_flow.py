"""Tests for control flow example nodes."""

from conftest import make_context
from sdk import get_all_definitions, run_node
from control_flow import (
    IfBranch,
    Compare,
    AndGate,
    OrGate,
    NotGate,
    Gate,
    Sequence,
)

CONTROL_FLOW_NAMES = {
    "if_branch_py", "compare_py", "and_gate_py", "or_gate_py",
    "not_gate_py", "gate_py", "sequence_py",
}


class TestControlFlowDefinitions:
    def test_all_registered(self):
        names = {d.name for d in get_all_definitions()}
        assert CONTROL_FLOW_NAMES.issubset(names)

    def test_node_count(self):
        defs = [d for d in get_all_definitions() if d.name in CONTROL_FLOW_NAMES]
        assert len(defs) == 7


_if_branch = IfBranch()
_compare = Compare()
_and = AndGate()
_or = OrGate()
_not = NotGate()
_gate = Gate()
_seq = Sequence()


class TestIfBranch:
    def test_true_branch(self):
        result = _if_branch.run(make_context({"condition": True}))
        assert "true" in result.activate_exec
        assert "false" not in result.activate_exec

    def test_false_branch(self):
        result = _if_branch.run(make_context({"condition": False}))
        assert "false" in result.activate_exec
        assert "true" not in result.activate_exec

    def test_default_is_false(self):
        result = _if_branch.run(make_context({}))
        assert "false" in result.activate_exec


class TestCompare:
    def test_equal(self):
        result = _compare.run(make_context({"a": 5.0, "b": 5.0}))
        assert result.outputs["equal"] is True
        assert result.outputs["less_than"] is False
        assert result.outputs["greater_than"] is False

    def test_less_than(self):
        result = _compare.run(make_context({"a": 3.0, "b": 7.0}))
        assert result.outputs["equal"] is False
        assert result.outputs["less_than"] is True
        assert result.outputs["greater_than"] is False

    def test_greater_than(self):
        result = _compare.run(make_context({"a": 10.0, "b": 2.0}))
        assert result.outputs["equal"] is False
        assert result.outputs["less_than"] is False
        assert result.outputs["greater_than"] is True


class TestAndGate:
    def test_true_true(self):
        result = _and.run(make_context({"a": True, "b": True}))
        assert result.outputs["result"] is True

    def test_true_false(self):
        result = _and.run(make_context({"a": True, "b": False}))
        assert result.outputs["result"] is False

    def test_false_false(self):
        result = _and.run(make_context({"a": False, "b": False}))
        assert result.outputs["result"] is False


class TestOrGate:
    def test_true_true(self):
        result = _or.run(make_context({"a": True, "b": True}))
        assert result.outputs["result"] is True

    def test_true_false(self):
        result = _or.run(make_context({"a": True, "b": False}))
        assert result.outputs["result"] is True

    def test_false_false(self):
        result = _or.run(make_context({"a": False, "b": False}))
        assert result.outputs["result"] is False


class TestNotGate:
    def test_true(self):
        result = _not.run(make_context({"value": True}))
        assert result.outputs["result"] is False

    def test_false(self):
        result = _not.run(make_context({"value": False}))
        assert result.outputs["result"] is True


class TestGate:
    def test_open(self):
        result = _gate.run(make_context({"condition": True}))
        assert "exec_out" in result.activate_exec

    def test_closed(self):
        result = _gate.run(make_context({"condition": False}))
        assert "exec_out" not in result.activate_exec

    def test_default_open(self):
        result = _gate.run(make_context({}))
        assert "exec_out" in result.activate_exec


class TestSequence:
    def test_all_outputs_activated(self):
        result = _seq.run(make_context({}))
        assert "out_1" in result.activate_exec
        assert "out_2" in result.activate_exec
        assert "out_3" in result.activate_exec

    def test_order_preserved(self):
        result = _seq.run(make_context({}))
        assert result.activate_exec == ["out_1", "out_2", "out_3"]


class TestDispatch:
    def test_if_branch(self):
        result = run_node("if_branch_py", make_context({"condition": True}))
        assert "true" in result.activate_exec

    def test_unknown_node(self):
        result = run_node("nonexistent", make_context({}))
        assert result.error is not None
        assert "Unknown node" in result.error
