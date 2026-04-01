"""
Template Node — Example showing the new subclass-kwargs style

This is the starter node included when scaffolding a new WASM-node project.
"""

from sdk import (
    ExecutionResult,
    Input,
    Output,
    WasmNode,
)


class MyCustomNode(WasmNode, name="my_custom_node_py", category="Custom/WASM"):
    """Repeats the input text by a multiplier"""

    input_text: str = Input(default="")
    multiplier: int = Input(default=1)
    output_text: str = Output()
    char_count: int = Output()

    def run(self, ctx) -> ExecutionResult:
        repeated = ctx.input_text * max(0, ctx.multiplier)
        ctx.output_text = repeated
        ctx.char_count = len(repeated)
        if ctx.stream_enabled:
            ctx.stream_text(f"{len(repeated)} characters")
        return ctx.success()
