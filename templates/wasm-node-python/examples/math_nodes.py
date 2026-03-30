"""
Math Nodes — Basic arithmetic and mathematical operations

Demonstrates creating multiple nodes for add, subtract, multiply,
divide, power, and clamp operations.
"""

from sdk import (
    ExecutionResult,
    Input,
    Output,
    WasmNode,
)


class Add(WasmNode, name="math_add_py", title="Add", category="Math/Arithmetic"):
    """Adds two numbers together"""

    a: float = Input(default=0.0)
    b: float = Input(default=0.0)
    result: float = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.a + ctx.b
        return ctx.success()


class Subtract(WasmNode, name="math_subtract_py", title="Subtract", category="Math/Arithmetic"):
    """Subtracts B from A"""

    a: float = Input(default=0.0)
    b: float = Input(default=0.0)
    result: float = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.a - ctx.b
        return ctx.success()


class Multiply(WasmNode, name="math_multiply_py", title="Multiply", category="Math/Arithmetic"):
    """Multiplies two numbers"""

    a: float = Input(default=0.0)
    b: float = Input(default=0.0)
    result: float = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.a * ctx.b
        return ctx.success()


class Divide(WasmNode, name="math_divide_py", title="Divide", category="Math/Arithmetic"):
    """Divides A by B"""

    a: float = Input(default=0.0)
    b: float = Input(default=1.0)
    result: float = Output()
    is_valid: bool = Output()

    def run(self, ctx) -> ExecutionResult:
        if ctx.b == 0.0:
            ctx.result = 0.0
            ctx.is_valid = False
            ctx.warn("Division by zero")
        else:
            ctx.result = ctx.a / ctx.b
            ctx.is_valid = True
        return ctx.success()


class Power(WasmNode, name="math_power_py", title="Power", category="Math/Arithmetic"):
    """Raises base to the power of exponent"""

    base: float = Input(default=0.0)
    exponent: float = Input(default=1.0)
    result: float = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.base ** ctx.exponent
        return ctx.success()


class Clamp(WasmNode, name="math_clamp_py", title="Clamp", category="Math/Utility"):
    """Clamps a value between min and max"""

    value: float = Input(default=0.0)
    min: float = Input(default=0.0, pin_name="min")
    max: float = Input(default=1.0, pin_name="max")
    result: float = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = max(ctx.min, min(ctx.max, ctx.value))
        return ctx.success()
