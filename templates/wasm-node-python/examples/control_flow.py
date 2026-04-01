"""
Control Flow Nodes — Logic and branching operations

Demonstrates if/else branching, comparison, boolean gates,
conditional pass-through, and sequencing.
"""

from sdk import (
    Exec,
    ExecOutput,
    ExecutionResult,
    Input,
    Output,
    WasmNode,
)


class IfBranch(WasmNode, name="if_branch_py", title="If Branch", category="Control/Branch"):
    """Branches based on boolean condition"""

    condition: bool = Input(default=False)
    true: Exec = ExecOutput()
    false: Exec = ExecOutput()

    def run(self, ctx) -> ExecutionResult:
        if ctx.condition:
            ctx.activate_exec("true")
        else:
            ctx.activate_exec("false")
        return ctx.finish()


class Compare(WasmNode, name="compare_py", title="Compare", category="Control/Logic"):
    """Compares two values"""

    a: float = Input(default=0.0)
    b: float = Input(default=0.0)
    equal: bool = Output()
    less_than: bool = Output()
    greater_than: bool = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.equal = ctx.a == ctx.b
        ctx.less_than = ctx.a < ctx.b
        ctx.greater_than = ctx.a > ctx.b
        return ctx.success()


class AndGate(WasmNode, name="and_gate_py", title="AND Gate", category="Control/Logic"):
    """Logical AND of two booleans"""

    a: bool = Input(default=False)
    b: bool = Input(default=False)
    result: bool = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.a and ctx.b
        return ctx.success()


class OrGate(WasmNode, name="or_gate_py", title="OR Gate", category="Control/Logic"):
    """Logical OR of two booleans"""

    a: bool = Input(default=False)
    b: bool = Input(default=False)
    result: bool = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.a or ctx.b
        return ctx.success()


class NotGate(WasmNode, name="not_gate_py", title="NOT Gate", category="Control/Logic"):
    """Logical NOT"""

    value: bool = Input(default=False)
    result: bool = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = not ctx.value
        return ctx.success()


class Gate(WasmNode, name="gate_py", title="Gate", category="Control/Branch"):
    """Passes execution only if condition is true"""

    condition: bool = Input(default=True)

    def run(self, ctx) -> ExecutionResult:
        if ctx.condition:
            return ctx.success()
        return ctx.finish()


class Sequence(WasmNode, name="sequence_py", title="Sequence", category="Control/Flow"):
    """Activates multiple outputs in order"""

    out_1: Exec = ExecOutput()
    out_2: Exec = ExecOutput()
    out_3: Exec = ExecOutput()

    def run(self, ctx) -> ExecutionResult:
        ctx.activate_exec("out_1")
        ctx.activate_exec("out_2")
        ctx.activate_exec("out_3")
        return ctx.finish()
