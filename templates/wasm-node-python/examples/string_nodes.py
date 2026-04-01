"""
String Nodes — Text manipulation utilities

Demonstrates string processing nodes including case conversion,
trimming, length analysis, search, replace, concat, and reverse.
"""

from sdk import (
    ExecutionResult,
    Input,
    Output,
    WasmNode,
)


class ToUppercase(WasmNode, name="string_uppercase_py", title="To Uppercase", category="String/Transform"):
    """Converts text to uppercase"""

    text: str = Input(default="")
    result: str = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.text.upper()
        return ctx.success()


class ToLowercase(WasmNode, name="string_lowercase_py", title="To Lowercase", category="String/Transform"):
    """Converts text to lowercase"""

    text: str = Input(default="")
    result: str = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.text.lower()
        return ctx.success()


class Trim(WasmNode, name="string_trim_py", title="Trim", category="String/Transform"):
    """Removes leading and trailing whitespace"""

    text: str = Input(default="")
    result: str = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.text.strip()
        return ctx.success()


class Reverse(WasmNode, name="string_reverse_py", title="Reverse", category="String/Transform"):
    """Reverses the characters in a string"""

    text: str = Input(default="")
    result: str = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.text[::-1]
        return ctx.success()


class StringLength(WasmNode, name="string_length_py", title="String Length", category="String/Analysis"):
    """Returns the length of a string"""

    text: str = Input(default="")
    length: int = Output()
    is_empty: bool = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.length = len(ctx.text)
        ctx.is_empty = len(ctx.text) == 0
        return ctx.success()


class Contains(WasmNode, name="string_contains_py", title="Contains", category="String/Analysis"):
    """Checks if text contains a substring"""

    text: str = Input(default="")
    search: str = Input(default="")
    result: bool = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = ctx.search in ctx.text
        return ctx.success()


class Replace(WasmNode, name="string_replace_py", title="Replace", category="String/Transform"):
    """Replaces occurrences of a pattern"""

    text: str = Input(default="")
    find: str = Input(default="")
    replace_with: str = Input(default="")
    result: str = Output()
    count: int = Output()

    def run(self, ctx) -> ExecutionResult:
        count = ctx.text.count(ctx.find) if ctx.find else 0
        result = ctx.text.replace(ctx.find, ctx.replace_with) if ctx.find else ctx.text
        ctx.result = result
        ctx.count = count
        return ctx.success()


class Concatenate(WasmNode, name="string_concat_py", title="Concatenate", category="String/Transform"):
    """Joins two strings together"""

    a: str = Input(default="")
    b: str = Input(default="")
    separator: str = Input(default="")
    result: str = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.result = f"{ctx.a}{ctx.separator}{ctx.b}" if ctx.separator else f"{ctx.a}{ctx.b}"
        return ctx.success()
