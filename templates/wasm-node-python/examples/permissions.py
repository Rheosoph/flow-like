"""
HTTP Request Example — Demonstrates how to declare permissions and make HTTP requests

Permissions tell the runtime which host capabilities your node requires.
When users place the node, they see the requested permissions and must
consent before execution.

This example declares the "http" permission and makes a GET request
to a public API.
"""

from sdk import (
    ExecutionResult,
    Input,
    Output,
    WasmNode,
)


class HttpRequestExample(WasmNode, name="http_request_example_py", title="HTTP Request Example", category="Examples/HTTP"):
    """Fetches data from a public API using HTTP"""

    permissions = ["http"]

    url: str = Input(default="https://httpbin.org/get")
    status: int = Output()
    body: str = Output()

    def run(self, ctx) -> ExecutionResult:
        ctx.info(f"Making GET request to: {ctx.url}")
        response = ctx.http_get(ctx.url)
        if response is None:
            return ctx.fail("HTTP request failed or permission denied")
        ctx.status = response.get("status", 0)
        ctx.body = response.get("body", "")
        ctx.info(f"Response status: {response.get('status')}")
        return ctx.success()
