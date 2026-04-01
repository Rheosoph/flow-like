"""Tests for declarative nodes with auto-derived metadata and subclass kwargs."""

from sdk import (
    ExecutionResult,
    Input,
    Output,
    WasmNode,
    get_all_definitions,
)
from conftest import make_context


# ── Minimal nodes (no Meta at all) ─────────────────────────────────────


class SimpleAdd(WasmNode):
    """Adds two numbers."""

    a: float = Input(default=0.0)
    b: float = Input(default=0.0)
    result: float = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.a + ctx.b
        return ctx.success()


class MyCustomNode(WasmNode):
    """Node with CamelCase name."""

    value: str = Input(default="hello")
    output: str = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.output = ctx.value.upper()
        return ctx.success()


# ── Partial Meta (only some fields) ────────────────────────────────────


class PartialMetaNode(WasmNode, category="Math"):
    """Only category specified."""

    x: float = Input(default=1.0)
    result: float = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.x * 2
        return ctx.success()


class NameOnlyMeta(WasmNode, name="custom_name_node"):
    """Only name specified via kwargs."""

    x: int = Input(default=0)
    out: int = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.out = ctx.x + 1
        return ctx.success()


# ── Tests ───────────────────────────────────────────────────────────────

def _find_def(name: str):
    for nd in get_all_definitions():
        if nd.name == name:
            return nd
    raise AssertionError(f"Node '{name}' not found in registry")


class TestAutoName:
    def test_simple_add_name(self):
        nd = _find_def("simple_add")
        assert nd.name == "simple_add"

    def test_simple_add_title(self):
        nd = _find_def("simple_add")
        assert nd.friendly_name == "Simple Add"

    def test_simple_add_category_default(self):
        nd = _find_def("simple_add")
        assert nd.category == "Custom"

    def test_simple_add_description_from_docstring(self):
        nd = _find_def("simple_add")
        assert nd.description == "Adds two numbers."

    def test_camel_case_to_snake(self):
        nd = _find_def("my_custom_node")
        assert nd.name == "my_custom_node"

    def test_camel_case_title(self):
        nd = _find_def("my_custom_node")
        assert nd.friendly_name == "My Custom Node"


class TestPartialMeta:
    def test_category_from_meta(self):
        nd = _find_def("partial_meta_node")
        assert nd.category == "Math"

    def test_name_auto_derived(self):
        nd = _find_def("partial_meta_node")
        assert nd.name == "partial_meta_node"

    def test_title_auto_derived(self):
        nd = _find_def("partial_meta_node")
        assert nd.friendly_name == "Partial Meta Node"

    def test_name_from_meta(self):
        nd = _find_def("custom_name_node")
        assert nd.name == "custom_name_node"

    def test_title_from_custom_name(self):
        nd = _find_def("custom_name_node")
        assert nd.friendly_name == "Custom Name Node"


class TestMetaLessExecution:
    def test_simple_add_runs(self):
        ctx = make_context({"a": 3.0, "b": 4.0})
        node = SimpleAdd()
        result = node.run(ctx)
        assert result.error is None
        assert result.outputs["result"] == 7.0

    def test_my_custom_node_runs(self):
        ctx = make_context({"value": "hello"})
        node = MyCustomNode()
        result = node.run(ctx)
        assert result.error is None
        assert result.outputs["output"] == "HELLO"

    def test_partial_meta_runs(self):
        ctx = make_context({"x": 5.0})
        node = PartialMetaNode()
        result = node.run(ctx)
        assert result.error is None
        assert result.outputs["result"] == 10.0


class TestRegistration:
    def test_all_meta_less_nodes_registered(self):
        names = {nd.name for nd in get_all_definitions()}
        assert "simple_add" in names
        assert "my_custom_node" in names
        assert "partial_meta_node" in names
        assert "custom_name_node" in names
