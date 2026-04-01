"""Tests for math example nodes."""

from conftest import make_context
from sdk import MockHostBridge, get_all_definitions, run_node
from math_nodes import Add, Subtract, Multiply, Divide, Power, Clamp

MATH_NAMES = {
    "math_add_py", "math_subtract_py", "math_multiply_py",
    "math_divide_py", "math_power_py", "math_clamp_py",
}


class TestMathDefinitions:
    def test_all_registered(self):
        names = {d.name for d in get_all_definitions()}
        assert MATH_NAMES.issubset(names)

    def test_node_count(self):
        defs = [d for d in get_all_definitions() if d.name in MATH_NAMES]
        assert len(defs) == 6

    def test_all_have_exec_pins(self):
        for nd in get_all_definitions():
            if nd.name not in MATH_NAMES:
                continue
            pin_names = {p.name for p in nd.pins}
            assert "exec" in pin_names, f"{nd.name} missing exec pin"
            assert "exec_out" in pin_names, f"{nd.name} missing exec_out pin"


_add = Add()
_sub = Subtract()
_mul = Multiply()
_div = Divide()
_pow = Power()
_clamp = Clamp()


class TestAdd:
    def test_basic(self):
        result = _add.run(make_context({"a": 5.0, "b": 3.0}))
        assert result.outputs["result"] == 8.0

    def test_negative(self):
        result = _add.run(make_context({"a": -5.0, "b": 3.0}))
        assert result.outputs["result"] == -2.0

    def test_defaults(self):
        result = _add.run(make_context({}))
        assert result.outputs["result"] == 0.0

    def test_floats(self):
        result = _add.run(make_context({"a": 0.1, "b": 0.2}))
        assert abs(result.outputs["result"] - 0.3) < 1e-10


class TestSubtract:
    def test_basic(self):
        result = _sub.run(make_context({"a": 10.0, "b": 4.0}))
        assert result.outputs["result"] == 6.0

    def test_negative_result(self):
        result = _sub.run(make_context({"a": 3.0, "b": 7.0}))
        assert result.outputs["result"] == -4.0


class TestMultiply:
    def test_basic(self):
        result = _mul.run(make_context({"a": 3.0, "b": 4.0}))
        assert result.outputs["result"] == 12.0

    def test_by_zero(self):
        result = _mul.run(make_context({"a": 99.0, "b": 0.0}))
        assert result.outputs["result"] == 0.0


class TestDivide:
    def test_basic(self):
        result = _div.run(make_context({"a": 10.0, "b": 2.0}))
        assert result.outputs["result"] == 5.0
        assert result.outputs["is_valid"] is True

    def test_by_zero(self):
        host = MockHostBridge()
        result = _div.run(make_context({"a": 10.0, "b": 0.0}, host=host))
        assert result.outputs["result"] == 0.0
        assert result.outputs["is_valid"] is False
        assert any("Division by zero" in msg for _, msg in host.logs)

    def test_fractional(self):
        result = _div.run(make_context({"a": 1.0, "b": 3.0}))
        assert abs(result.outputs["result"] - 1 / 3) < 1e-10


class TestPower:
    def test_square(self):
        result = _pow.run(make_context({"base": 3.0, "exponent": 2.0}))
        assert result.outputs["result"] == 9.0

    def test_cube(self):
        result = _pow.run(make_context({"base": 2.0, "exponent": 3.0}))
        assert result.outputs["result"] == 8.0

    def test_zero_exponent(self):
        result = _pow.run(make_context({"base": 5.0, "exponent": 0.0}))
        assert result.outputs["result"] == 1.0

    def test_fractional_exponent(self):
        result = _pow.run(make_context({"base": 4.0, "exponent": 0.5}))
        assert result.outputs["result"] == 2.0


class TestClamp:
    def test_within_range(self):
        result = _clamp.run(make_context({"value": 0.5, "min": 0.0, "max": 1.0}))
        assert result.outputs["result"] == 0.5

    def test_below_min(self):
        result = _clamp.run(make_context({"value": -5.0, "min": 0.0, "max": 10.0}))
        assert result.outputs["result"] == 0.0

    def test_above_max(self):
        result = _clamp.run(make_context({"value": 15.0, "min": 0.0, "max": 10.0}))
        assert result.outputs["result"] == 10.0

    def test_equal_bounds(self):
        result = _clamp.run(make_context({"value": 99.0, "min": 5.0, "max": 5.0}))
        assert result.outputs["result"] == 5.0


class TestDispatch:
    def test_known_node(self):
        result = run_node("math_add_py", make_context({"a": 1.0, "b": 2.0}))
        assert result.outputs["result"] == 3.0

    def test_unknown_node(self):
        result = run_node("nonexistent", make_context({}))
        assert result.error is not None
        assert "Unknown node" in result.error
